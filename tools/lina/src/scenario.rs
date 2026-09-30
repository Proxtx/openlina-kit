//! Scenarios: scripted game runs for `lina test` (with expectations on the log) and `lina gif`
//! (with frame capture). They live in `mods/<id>/tests/*.toml`:
//!
//! ```toml
//! name = "a box falling off the bottom comes back at the top"
//! mods = ["screen-wrap"]            # mods under test; `core`, `harness` and requirements are added
//! fixtures = ["debug-spawn"]        # extra mods that set the scene
//! timeout = 60                      # seconds (wall clock) before the run is killed
//!
//! [options.screen-wrap]             # options per mod (mods and fixtures)
//! trace = true
//!
//! [harness]                         # harness options: level, seed, modifier, items, inputs, end_tick
//! level = "greendemo 1"
//! end_tick = 600
//!
//! [[expect]]
//! contains = "[screen-wrap] tick"   # lines containing this text...
//! min = 2                           # ...at least 2 of them (default 1); `max` for an upper bound
//! [[expect]]
//! not_contains = "Uncaught"
//! [[expect]]                        # needs the fixture `trace-positions` (prints `[pos]` lines)
//! position = { tick = 130, type = "player", x = 412, y = 150, within = 6 }   # `away = true` inverts
//! [[expect]]                        # bounds: some box above y 110 (y grows downwards)
//! position = { tick = 200, type = "box", y_lt = 110 }
//!
//! [gif]                             # used by `lina gif`
//! capture = "60-420/2"              # level ticks from-to/step (120 ticks per second)
//! out = "media/wrap.gif"            # relative to the mod directory
//! scale = 1                         # 1 = 600x338
//! ```
//!
//! Each run gets its own empty `userdata` (saves, settings, level pack states), so runs are
//! independent of the player's saves and never change them.
//!
//! A run passes when the game exits with code 0 (the harness ends it at `end_tick`), the log has
//! no crash, and every expectation holds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::manifest::{ModPack, PackEntry};
use serde::Deserialize;

use crate::build;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub fixtures: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub options: BTreeMap<String, toml::Table>,
    #[serde(default)]
    pub harness: toml::Table,
    #[serde(default)]
    pub expect: Vec<Expect>,
    pub gif: Option<GifSpec>,
}

fn default_timeout() -> u64 {
    60
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub contains: Option<String>,
    pub not_contains: Option<String>,
    pub min: Option<usize>,
    pub max: Option<usize>,
    /// Where an object is at a tick, from the `trace-positions` fixture's `[pos]` lines.
    pub position: Option<PositionExpect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionExpect {
    pub tick: i64,
    #[serde(rename = "type")]
    pub kind: String,
    /// Omit x or y to check one axis only.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Allowed distance (layout units).
    #[serde(default = "eight")]
    pub within: f64,
    /// Invert: no such object may be within `within` (e.g. "it left its spot").
    #[serde(default)]
    pub away: bool,
    /// Bounds (strict): some object of the type must be left of / right of / above / below them
    /// (y grows downwards: `y_lt` = higher on screen). E.g. "the box is still above 110 at tick
    /// 200", where vanilla has it lower. Combine with x/y, or use alone.
    pub x_lt: Option<f64>,
    pub x_gt: Option<f64>,
    pub y_lt: Option<f64>,
    pub y_gt: Option<f64>,
}

fn eight() -> f64 {
    8.0
}

impl PositionExpect {
    /// Failure message, if any.
    fn check(&self, log: &str) -> Option<String> {
        let prefix = format!("[pos] tick {} {} ", self.tick, self.kind);
        let found: Vec<(f64, f64)> = log
            .lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .filter_map(|rest| {
                let mut it = rest.split_whitespace().map(|v| v.parse::<f64>());
                Some((it.next()?.ok()?, it.next()?.ok()?))
            })
            .collect();
        if found.is_empty() {
            return Some(format!(
                "no `{prefix}…` line: the fixture `trace-positions` needs `types` including {:?} and `ticks` including \
                 {}; an object shows up from the tick after it was created (debug-spawn at tick T: trace T+1 or later)",
                self.kind, self.tick
            ));
        }
        let bounded = |&(x, y): &(f64, f64)| {
            self.x_lt.is_none_or(|b| x < b)
                && self.x_gt.is_none_or(|b| x > b)
                && self.y_lt.is_none_or(|b| y < b)
                && self.y_gt.is_none_or(|b| y > b)
        };
        let bounds_text = || {
            [("x <", self.x_lt), ("x >", self.x_gt), ("y <", self.y_lt), ("y >", self.y_gt)]
                .iter()
                .filter_map(|(k, v)| v.map(|v| format!("{k} {v}")))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let all_found: Vec<String> = found.iter().take(5).map(|(x, y)| format!("({x}, {y})")).collect();
        let found: Vec<(f64, f64)> = found.iter().copied().filter(bounded).collect();
        if self.x.is_none() && self.y.is_none() {
            return match (found.is_empty(), self.away) {
                (true, false) => Some(format!(
                    "{} at tick {}: expected one with {}, found at {}",
                    self.kind,
                    self.tick,
                    bounds_text(),
                    all_found.join(" ")
                )),
                (false, true) => Some(format!(
                    "{} at tick {}: expected none with {}, found at {}",
                    self.kind,
                    self.tick,
                    bounds_text(),
                    all_found.join(" ")
                )),
                _ => None,
            };
        }
        if found.is_empty() {
            return (!self.away).then(|| {
                format!(
                    "{} at tick {}: none with {} (found at {})",
                    self.kind,
                    self.tick,
                    bounds_text(),
                    all_found.join(" ")
                )
            });
        }
        let dist = |(x, y): (f64, f64)| {
            let dx = self.x.map_or(0.0, |w| x - w);
            let dy = self.y.map_or(0.0, |w| y - w);
            (dx * dx + dy * dy).sqrt()
        };
        let best = found.iter().copied().min_by(|a, b| dist(*a).total_cmp(&dist(*b))).unwrap();
        let near = dist(best) <= self.within;
        let want = format!(
            "({}, {})",
            self.x.map_or("*".into(), |v| v.to_string()),
            self.y.map_or("*".into(), |v| v.to_string())
        );
        match (near, self.away) {
            (false, false) => Some(format!(
                "{} at tick {}: expected within {} of {want}, nearest at ({}, {}), {:.1} away",
                self.kind,
                self.tick,
                self.within,
                best.0,
                best.1,
                dist(best)
            )),
            (true, true) => Some(format!(
                "{} at tick {}: expected none within {} of {want}, found one at ({}, {})",
                self.kind, self.tick, self.within, best.0, best.1
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GifSpec {
    pub capture: String,
    pub out: String,
    #[serde(default = "one")]
    pub scale: u32,
}

fn one() -> u32 {
    1
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let sc: Scenario = toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        for e in &sc.expect {
            if let Some(p) = &e.position {
                let bounds = p.x_lt.is_some() || p.x_gt.is_some() || p.y_lt.is_some() || p.y_gt.is_some();
                ensure!(
                    p.x.is_some() || p.y.is_some() || bounds,
                    "{}: a `position` expectation needs x, y or a bound (x_lt, x_gt, y_lt, y_gt)",
                    path.display()
                );
            }
        }
        Ok(sc)
    }

    /// The modpack for this scenario: core, harness (+ extra harness options), fixtures, mods.
    fn pack(&self, extra_harness: toml::Table) -> Result<ModPack> {
        let mut pack = ModPack { openlina: 1, ..Default::default() };
        let mut harness = self.harness.clone();
        harness.extend(extra_harness);
        pack.mods.push(PackEntry { id: "core".into(), ..Default::default() });
        pack.mods.push(PackEntry { id: "harness".into(), options: harness, ..Default::default() });
        for id in self.fixtures.iter().chain(&self.mods) {
            if pack.mods.iter().any(|e| &e.id == id) {
                continue;
            }
            let options = self.options.get(id).cloned().unwrap_or_default();
            pack.mods.push(PackEntry { id: id.clone(), options, ..Default::default() });
        }
        for id in self.options.keys() {
            ensure!(pack.mods.iter().any(|e| &e.id == id), "options for `{id}`, which is not in mods or fixtures");
        }
        build::add_requirements(&mut pack)?;
        Ok(pack)
    }
}

/// Build and run a scenario headless in `work`. Returns (exit code, log); the build's progress
/// is the log's first lines. With `compiled`, the mods were compiled beforehand.
fn run(
    game: &Path,
    sc: &Scenario,
    extra_harness: toml::Table,
    wasm: bool,
    compiled: bool,
    work: &Path,
) -> Result<(Option<i32>, String)> {
    let mut extra_harness = extra_harness;
    // Frame capture outside `lina gif` (e.g. `capture` in a scenario's [harness] to look at a
    // probe run): the frames go to <work>/frames, which must exist.
    let capturing = sc.harness.contains_key("capture") || extra_harness.contains_key("capture");
    if capturing && !sc.harness.contains_key("capture_dir") && !extra_harness.contains_key("capture_dir") {
        let frames = work.join("frames");
        std::fs::create_dir_all(&frames)?;
        extra_harness.insert(
            "capture_dir".into(),
            toml::Value::String(std::fs::canonicalize(&frames)?.to_string_lossy().into()),
        );
    }
    let pack = sc.pack(extra_harness)?;
    let overlay = work.join("game");
    let mut build_log = String::new();
    let built = build::build_pack(game, &pack, wasm, &overlay, compiled, &mut |l| {
        build_log.push_str(&l);
        build_log.push('\n');
    });
    if let Err(e) = built {
        std::fs::write(work.join("log.txt"), format!("{build_log}build failed: {e:#}\n"))?;
        return Err(e);
    }
    // A fresh save directory per run: results don't depend on the player's saves (e.g. which level
    // packs they enabled), and test runs never write to them.
    let userdata = overlay.join("userdata");
    if userdata.symlink_metadata().is_ok() {
        std::fs::remove_file(&userdata).or_else(|_| std::fs::remove_dir_all(&userdata))?;
    }
    std::fs::create_dir(&userdata)?;
    let out = openlina::game::command(
        game,
        &std::fs::canonicalize(&overlay)?,
        Some(&overlay.join("hlboot.dat")),
        Some(sc.timeout),
        true,
    )?
    .output()
    .context("running the game")?;
    let mut log = build_log;
    log.push_str(&String::from_utf8_lossy(&out.stdout));
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    // Steam's client library is chatty; keep the log readable.
    let log: String = log
        .lines()
        .filter(|l| {
            !l.starts_with("[S_API")
                && !l.starts_with("[STEAM]")
                && !l.starts_with("Setting breakpad")
                && !l.starts_with("SteamInternal")
        })
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(work.join("log.txt"), &log)?;
    Ok((out.status.code(), log))
}

/// Check a finished run; returns the failures.
fn check(sc: &Scenario, code: Option<i32>, log: &str) -> Vec<String> {
    let mut fails = Vec::new();
    match code {
        Some(0) => {}
        Some(124) => {
            let last = log.lines().rev().find(|l| l.starts_with("[harness] layout "));
            fails.push(match last {
                Some(l) => format!(
                    "timed out after {}s; last seen: {} (a hang or a screen waiting for input there? else set harness.end_tick)",
                    sc.timeout,
                    l.trim_start_matches("[harness] ")
                ),
                None => format!("timed out after {}s (set harness.end_tick?)", sc.timeout),
            })
        }
        c => fails.push(format!("game exited with {c:?}")),
    }
    // Haxe exceptions the game catches and logs still mean the mod broke something.
    for bad in ["SIGNAL", "Uncaught exception", "[harness] ERROR", "Null access", "Called from "] {
        if let Some(line) = log.lines().find(|l| l.contains(bad)) {
            fails.push(format!("log: {line}"));
        }
    }
    for e in &sc.expect {
        if let Some(text) = &e.contains {
            let n = log.lines().filter(|l| l.contains(text.as_str())).count();
            let min = e.min.unwrap_or(1);
            if n < min {
                fails.push(format!("expected at least {min} line(s) containing {text:?}, found {n}"));
            }
            if let Some(max) = e.max {
                if n > max {
                    fails.push(format!("expected at most {max} line(s) containing {text:?}, found {n}"));
                }
            }
        }
        if let Some(p) = &e.position {
            if let Some(msg) = p.check(log) {
                fails.push(msg);
            }
        }
        if let Some(text) = &e.not_contains {
            if let Some(line) = log.lines().find(|l| l.contains(text.as_str())) {
                fails.push(format!("unexpected line: {line}"));
            }
        }
    }
    fails
}

/// Scenario files: the given paths, or every `mods/*/tests/*.toml` (optionally of one mod) plus
/// `tests/*.toml`.
pub fn find(paths: &[PathBuf], only_mod: Option<&str>) -> Result<Vec<PathBuf>> {
    if !paths.is_empty() {
        return Ok(paths.to_vec());
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir("mods")? {
        let dir = e?.path();
        if only_mod.is_some_and(|m| dir.file_name().is_some_and(|n| n != m)) {
            continue;
        }
        let tests = dir.join("tests");
        if tests.is_dir() {
            for t in std::fs::read_dir(&tests)? {
                let p = t?.path();
                if p.extension().is_some_and(|x| x == "toml") {
                    out.push(p);
                }
            }
        }
    }
    // Scenarios across mods (packs of several mods, soak runs) live in tests/.
    if only_mod.is_none() && Path::new("tests").is_dir() {
        for t in std::fs::read_dir("tests")? {
            let p = t?.path();
            if p.extension().is_some_and(|x| x == "toml") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// How `lina test` runs.
pub struct TestOpts {
    pub wasm: bool,
    /// Scenarios running at the same time (each is its own game process).
    pub jobs: usize,
}

/// The default for `--jobs`: half the cores, at most 8.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(1).clamp(1, 8)
}

/// The scenarios that failed in the last `lina test` (for `--failed`).
const FAILED: &str = "work/test/failed.txt";

pub fn last_failed() -> Result<Vec<PathBuf>> {
    let text = std::fs::read_to_string(FAILED).context("no failed scenarios recorded (work/test/failed.txt)")?;
    Ok(text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

struct Outcome {
    name: String,
    file: PathBuf,
    work: PathBuf,
    secs: f64,
    fails: Vec<String>,
    tail: Vec<String>,
}

pub fn test(game: &Path, files: &[PathBuf], opts: &TestOpts) -> Result<()> {
    ensure!(!files.is_empty(), "no scenarios found (mods/<id>/tests/*.toml)");
    let started = std::time::Instant::now();
    let scenarios: Vec<Scenario> = files.iter().map(|f| Scenario::load(f)).collect::<Result<_>>()?;
    // Compile every mod the scenarios need once, up front (cargo output stays visible).
    let mut ids = std::collections::BTreeSet::new();
    for sc in &scenarios {
        ids.extend(sc.pack(toml::Table::new())?.mods.into_iter().map(|e| e.id));
    }
    build::compile(&ids.into_iter().collect::<Vec<_>>(), opts.wasm)?;

    let root = PathBuf::from("work/test");
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    std::fs::create_dir_all(&root)?;
    let jobs = opts.jobs.clamp(1, scenarios.len());
    println!("running {} scenario(s), {jobs} at a time{}", scenarios.len(), if opts.wasm { " (wasm)" } else { "" });
    let next = std::sync::atomic::AtomicUsize::new(0);
    let outcomes = std::sync::Mutex::new(Vec::new());
    let print = std::sync::Mutex::new(());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(sc) = scenarios.get(i) else { break };
                let o = run_one(game, sc, &files[i], &root.join(i.to_string()), opts.wasm);
                let _guard = print.lock().unwrap();
                report(&o);
                outcomes.lock().unwrap().push(o);
            });
        }
    });
    let outcomes = outcomes.into_inner().unwrap();
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| !o.fails.is_empty()).collect();
    std::fs::write(FAILED, failed.iter().map(|o| format!("{}\n", o.file.display())).collect::<String>())?;
    println!(
        "\n{} passed, {} failed ({:.0}s)",
        outcomes.len() - failed.len(),
        failed.len(),
        started.elapsed().as_secs_f64()
    );
    if !failed.is_empty() {
        println!("rerun the failures with `lina test --failed`");
        bail!("failed: {}", failed.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(", "));
    }
    Ok(())
}

fn run_one(game: &Path, sc: &Scenario, file: &Path, work: &Path, wasm: bool) -> Outcome {
    let t = std::time::Instant::now();
    let (fails, log) = match std::fs::create_dir_all(work)
        .map_err(anyhow::Error::from)
        .and_then(|_| run(game, sc, toml::Table::new(), wasm, true, work))
    {
        Ok((code, log)) => (check(sc, code, &log), log),
        Err(e) => (vec![format!("{e:#}")], String::new()),
    };
    let frames = work.join("frames");
    if frames.is_dir() {
        contact_sheet(&frames, &work.join("frames.png"));
    }
    if fails.is_empty() {
        // Passed: the overlay (patched bytecode, links) isn't needed; keep log and frames.
        let _ = std::fs::remove_dir_all(work.join("game"));
    }
    // The last lines of the game's output: where a hang or crash left off.
    let tail = log.lines().rev().filter(|l| !l.trim().is_empty()).take(4).map(String::from).collect::<Vec<_>>();
    Outcome {
        name: sc.name.clone(),
        file: file.to_path_buf(),
        work: work.to_path_buf(),
        secs: t.elapsed().as_secs_f64(),
        fails,
        tail: tail.into_iter().rev().collect(),
    }
}

fn report(o: &Outcome) {
    let frames = o.work.join("frames.png");
    let frames = if frames.exists() { format!(", frames: {}", frames.display()) } else { String::new() };
    if o.fails.is_empty() {
        println!("PASS {} ({:.1}s){frames}", o.name, o.secs);
        return;
    }
    println!("FAIL {} ({:.1}s), {}{frames}", o.name, o.secs, o.file.display());
    for x in &o.fails {
        println!("    {x}");
    }
    println!(
        "    log: {} (the patched game is kept in {})",
        o.work.join("log.txt").display(),
        o.work.join("game").display()
    );
    if !o.tail.is_empty() {
        println!("    last lines:");
        for l in &o.tail {
            println!("      {l}");
        }
    }
}

/// All captured frames of a run in one image (5 per row, half size), to look at in one go.
fn contact_sheet(frames: &Path, out: &Path) {
    let Ok(dir) = std::fs::read_dir(frames) else { return };
    let mut pngs: Vec<PathBuf> =
        dir.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    if pngs.is_empty() {
        return;
    }
    pngs.sort();
    // At most 40 frames: every n-th one.
    let step = pngs.len().div_ceil(40);
    let pngs: Vec<PathBuf> = pngs.into_iter().step_by(step).collect();
    let mut cmd = if magick() == "magick" {
        let mut c = Command::new("magick");
        c.arg("montage");
        c
    } else {
        Command::new("montage")
    };
    cmd.args(&pngs).args(["-tile", "5x", "-geometry", "300x169+2+2", "-background", "#000"]).arg(out);
    let _ = cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

/// Run a scenario with frame capture and assemble the frames into a gif (needs ImageMagick's
/// `magick`, and optionally `gifsicle`, both in the nix dev shell).
pub fn gif(game: &Path, file: &Path, out: Option<&Path>) -> Result<()> {
    let sc = Scenario::load(file)?;
    let spec = sc.gif.as_ref().with_context(|| format!("{}: no [gif] section", file.display()))?;
    let (range, step) = spec.capture.split_once('/').unwrap_or((&spec.capture, "1"));
    let step: u32 = step.trim().parse().context("gif.capture step")?;
    ensure!(range.contains('-'), "gif.capture must be `from-to/step`");
    let work = PathBuf::from("work/gif");
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let frames = work.join("frames");
    std::fs::create_dir_all(&frames)?;
    let mut extra = toml::Table::new();
    extra.insert("capture".into(), toml::Value::String(spec.capture.clone()));
    extra.insert("capture_dir".into(), toml::Value::String(std::fs::canonicalize(&frames)?.to_string_lossy().into()));
    let (code, log) = run(game, &sc, extra, false, false, &work)?;
    let fails = check(&sc, code, &log);
    if !fails.is_empty() {
        bail!("the scenario failed ({}):\n  {}", work.join("log.txt").display(), fails.join("\n  "));
    }
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&frames)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    pngs.sort();
    ensure!(!pngs.is_empty(), "no frames were captured (does the scenario reach the capture range?)");

    let dest = match out {
        Some(p) => p.to_path_buf(),
        None => file.parent().and_then(|t| t.parent()).context("scenario outside mods/<id>/tests")?.join(&spec.out),
    };
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    // 120 ticks per second: one frame every `step` ticks lasts step/120 s.
    let mut cmd = Command::new(magick());
    cmd.arg("-delay").arg(format!("{step}x120")).arg("-loop").arg("0");
    cmd.args(&pngs);
    if spec.scale > 1 {
        cmd.arg("-filter").arg("point").arg("-resize").arg(format!("{}%", spec.scale * 100));
    }
    cmd.arg("-layers").arg("Optimize").arg(&dest);
    let st = cmd.status().context("running ImageMagick (install it, or use `nix develop`; `lina doctor` checks)")?;
    ensure!(st.success(), "magick failed");
    if Command::new("gifsicle").arg("-O3").arg("--batch").arg(&dest).status().is_err() {
        eprintln!("(gifsicle not found; gif not optimized)");
    }
    let size = std::fs::metadata(&dest)?.len();
    println!("wrote {} ({} frames, {:.0} KB)", dest.display(), pngs.len(), size as f64 / 1024.0);
    Ok(())
}

/// ImageMagick 7 is `magick`, ImageMagick 6 (still common in distributions) is `convert`.
pub fn magick() -> &'static str {
    let works = |c: &str| {
        Command::new(c)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("ImageMagick"))
    };
    if works("magick") {
        "magick"
    } else if works("convert") {
        "convert"
    } else {
        "magick"
    }
}

/// What `lina probe` runs.
pub struct ProbeSpec {
    pub paths: Vec<String>,
    pub mods: Vec<String>,
    pub level: Option<String>,
    pub at: Vec<String>,
    pub new_run: bool,
    pub inputs: Vec<String>,
    pub capture: Option<String>,
    pub end: Option<u32>,
    pub seed: i64,
    /// `key=value` harness options.
    pub harness: Vec<String>,
    /// `mod.key=value` mod options.
    pub set: Vec<String>,
}

/// A command-line value: TOML if it parses (`0.25`, `true`, `[1, 2]`), else a string.
fn cli_value(v: &str) -> toml::Value {
    toml::from_str::<toml::Table>(&format!("v = {v}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(v.to_string()))
}

/// `lina probe`: a throwaway scenario with the `inspect` fixture (written to work/probe.toml, so it
/// can be kept or rerun), then only what matters from its log.
pub fn probe(game: &Path, p: &ProbeSpec) -> Result<()> {
    ensure!(!p.paths.is_empty(), "give paths to print, e.g. game.levelManager.currentLevel.type.name");
    let last_tick = p.at.iter().filter_map(|a| a.strip_prefix("tick:")?.parse::<u32>().ok()).max();
    let end = p.end.unwrap_or_else(|| last_tick.map(|t| t + 60).unwrap_or(1200));
    let mut harness = toml::Table::new();
    if let Some(l) = &p.level {
        harness.insert("level".into(), l.clone().into());
    }
    harness.insert("seed".into(), p.seed.into());
    harness.insert("end_tick".into(), (end as i64).into());
    if p.new_run {
        harness.insert("new_run".into(), true.into());
    }
    if !p.inputs.is_empty() {
        harness.insert("inputs".into(), p.inputs.clone().into());
    }
    if let Some(c) = &p.capture {
        harness.insert("capture".into(), c.clone().into());
    }
    for kv in &p.harness {
        let (k, v) = kv.split_once('=').with_context(|| format!("--harness {kv}: expected key=value"))?;
        harness.insert(k.trim().to_string(), cli_value(v.trim()));
    }
    let mut inspect = toml::Table::new();
    inspect.insert("at".into(), p.at.clone().into());
    inspect.insert("print".into(), p.paths.clone().into());
    let mut options = toml::Table::new();
    options.insert("inspect".into(), inspect.into());
    for kv in &p.set {
        let (k, v) = kv.split_once('=').with_context(|| format!("--set {kv}: expected mod.key=value"))?;
        let (m, key) = k.trim().split_once('.').with_context(|| format!("--set {kv}: expected mod.key=value"))?;
        let entry = options.entry(m.to_string()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        entry.as_table_mut().context("option table")?.insert(key.to_string(), cli_value(v.trim()));
    }
    let mut sc = toml::Table::new();
    sc.insert("name".into(), "probe".into());
    sc.insert("mods".into(), p.mods.clone().into());
    sc.insert("fixtures".into(), vec!["inspect".to_string()].into());
    sc.insert("timeout".into(), 120.into());
    sc.insert("options".into(), options.into());
    sc.insert("harness".into(), harness.into());
    let file = PathBuf::from("work/probe.toml");
    std::fs::create_dir_all("work")?;
    std::fs::write(&file, toml::to_string(&sc)?)?;

    let scenario = Scenario::load(&file)?;
    let ids: Vec<String> = scenario.pack(toml::Table::new())?.mods.into_iter().map(|e| e.id).collect();
    build::compile(&ids, false)?;
    let work = PathBuf::from("work/test/probe");
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let o = run_one(game, &scenario, &file, &work, false);
    let log = std::fs::read_to_string(work.join("log.txt")).unwrap_or_default();
    for l in log.lines().filter(|l| l.starts_with("[inspect]") || l.starts_with("[harness] ERROR")) {
        println!("{}", l.trim_start_matches("[inspect] "));
    }
    if !o.fails.is_empty() {
        println!("(the run failed; details below)");
        report(&o);
    } else if work.join("frames.png").exists() {
        println!("frames: {}", work.join("frames.png").display());
    }
    println!("scenario: {} (edit and rerun with `lina test {}`)", file.display(), file.display());
    Ok(())
}

/// The non-dev mods of a modpack (what a recording's replays load).
pub fn pack_mods(pack: &Path) -> Result<Vec<String>> {
    let p = ModPack::load(pack)?;
    let dev = ["core", "harness", "record", "inspect", "trace-calls", "trace-positions", "debug-spawn"];
    Ok(p.mods.into_iter().map(|e| e.id).filter(|id| !dev.contains(&id.as_str())).collect())
}

/// One level attempt from `[record]` lines.
struct Attempt {
    level: String,
    modifier: i64,
    slots: Vec<(String, i64)>,
    /// (tick, bits) whenever the input changed.
    changes: Vec<(i64, i64)>,
}

/// Turn the `[record]` lines of a log into replay scenarios (`work/recordings/<n>-<level>.toml`):
/// the level, its modifier and tool slots, and the inputs as `from-to:keys` ranges. The harness
/// seeds the run from `seed`, not the recorded run's seeds: levels with random elements may play
/// out differently, so check a replay before relying on it.
pub fn recordings(log: &str, mods: &[String]) -> Result<Vec<PathBuf>> {
    let mut attempts: Vec<Attempt> = Vec::new();
    for l in log.lines() {
        let Some(rest) = l.strip_prefix("[record] ") else { continue };
        if let Some(r) = rest.strip_prefix("level ") {
            let (level, m) = r.rsplit_once(" modifier ").context("bad [record] level line")?;
            attempts.push(Attempt { level: level.into(), modifier: m.trim().parse()?, slots: vec![], changes: vec![] });
        } else if let Some(r) = rest.strip_prefix("slot ") {
            let parts: Vec<&str> = r.split_whitespace().collect();
            if let (Some(a), [_, name, ammo]) = (attempts.last_mut(), parts.as_slice()) {
                a.slots.push((name.to_string(), ammo.parse().unwrap_or(0)));
            }
        } else if let Some(r) = rest.strip_prefix("tick ") {
            let (t, b) = r.split_once(" bits ").context("bad [record] tick line")?;
            if let Some(a) = attempts.last_mut() {
                a.changes.push((t.trim().parse()?, b.trim().parse()?));
            }
        }
    }
    let dir = PathBuf::from("work/recordings");
    std::fs::create_dir_all(&dir)?;
    let mut out = Vec::new();
    let keys = ["up", "down", "left", "right", "jump", "shoot", "switch", "restart"];
    for (n, a) in attempts.iter().enumerate() {
        // Only attempts with input (skips the title screen and levels the harness passed through).
        if !a.changes.iter().any(|&(_, b)| b != 0) {
            continue;
        }
        // Recorded at tick t = fed by the harness at tick t-1; a range lasts until the next change.
        let mut inputs = Vec::new();
        for (i, &(t, bits)) in a.changes.iter().enumerate() {
            if bits == 0 {
                continue;
            }
            let end = a.changes.get(i + 1).map(|&(t2, _)| t2 - 1).unwrap_or(t + 1);
            let names: Vec<&str> = (0..8).filter(|k| bits & (1 << k) != 0).map(|k| keys[k]).collect();
            inputs.push(format!("{}-{}:{}", t - 1, end, names.join("+")));
        }
        let last = a.changes.last().map(|c| c.0).unwrap_or(0);
        let end_tick = last + 240;
        let slots: Vec<String> = a.slots.iter().map(|(s, am)| format!("{s}:{am}")).collect();
        let slug: String =
            a.level.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let path = dir.join(format!("{n}-{slug}.toml"));
        let text = format!(
            "# Replay of a recorded level attempt (lina run --record). The harness seeds the run itself:\n\
             # levels with random elements may differ from what was played. Add expectations, then move it to\n\
             # mods/<id>/tests/ (see docs/testing.md).\n\
             name = {name:?}\n\
             mods = {mods:?}\n\
             fixtures = [\"trace-positions\"]\n\
             timeout = 120\n\n\
             [options.trace-positions]\n\
             types = [\"player\"]\n\
             ticks = \"60-{end_tick}/60\"\n\n\
             [harness]\n\
             level = {level:?}\n\
             modifier = {modifier}\n\
             slots = {slots:?}\n\
             inputs = {inputs:?}\n\
             end_tick = {end_tick}\n\n\
             [[expect]]\n\
             contains = {expect:?}\n",
            name = format!("recording: {} (attempt {n})", a.level),
            level = a.level,
            modifier = a.modifier,
            expect = format!("[harness] level tick 1: {}", a.level),
        );
        std::fs::write(&path, text)?;
        out.push(path);
    }
    ensure!(!out.is_empty(), "no level attempt with input in the log (were there `[record]` lines?)");
    Ok(out)
}

/// `lina run --record`: build the pack plus `record`, play (output shown and kept), then write
/// replay scenarios.
pub fn run_recording(game: &Path, pack_path: &Path, timeout: Option<u64>, headless: bool) -> Result<()> {
    let mut pack = ModPack::load(pack_path)?;
    if !pack.mods.iter().any(|e| e.id == "record") {
        pack.mods.push(PackEntry { id: "record".into(), ..Default::default() });
    }
    build::add_requirements(&mut pack)?;
    let overlay = PathBuf::from("work/record/game");
    build::build_pack(game, &pack, false, &overlay, false, &mut |l| println!("{l}"))?;
    let mut cmd = openlina::game::command(
        game,
        &std::fs::canonicalize(&overlay)?,
        Some(&overlay.join("hlboot.dat")),
        timeout,
        headless,
    )?;
    cmd.stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn().context("launching the game")?;
    let mut log = String::new();
    if let Some(out) = child.stdout.take() {
        use std::io::BufRead;
        for line in std::io::BufReader::new(out).lines().map_while(|l| l.ok()) {
            if !line.starts_with("[record]") {
                println!("{line}");
            }
            log.push_str(&line);
            log.push('\n');
        }
    }
    child.wait()?;
    std::fs::write("work/record/log.txt", &log)?;
    let mods = pack_mods(pack_path)?;
    for p in recordings(&log, &mods)? {
        println!("wrote {} (replay: lina test {})", p.display(), p.display());
    }
    Ok(())
}
