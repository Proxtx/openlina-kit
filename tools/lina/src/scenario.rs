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
                "no `{prefix}…` line: add the fixture `trace-positions` with `types` including {:?} and `ticks` including {}",
                self.kind, self.tick
            ));
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
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))
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

/// Build and run a scenario headless. Returns (exit code, log).
fn run(
    game: &Path,
    sc: &Scenario,
    extra_harness: toml::Table,
    wasm: bool,
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
    let bytes = build::build_pack(game, &pack, wasm, &overlay)?;
    std::fs::write(work.join("hlboot.dat"), &bytes)?;
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
    let mut log = String::from_utf8_lossy(&out.stdout).to_string();
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
        Some(124) => fails.push(format!("timed out after {}s (set harness.end_tick?)", sc.timeout)),
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

/// Scenario files: the given paths, or every `mods/*/tests/*.toml` (optionally of one mod).
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
    out.sort();
    Ok(out)
}

pub fn test(game: &Path, files: &[PathBuf], wasm: bool) -> Result<()> {
    ensure!(!files.is_empty(), "no scenarios found (mods/<id>/tests/*.toml)");
    let mut failed = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let sc = Scenario::load(f)?;
        let work = PathBuf::from("work/test").join(i.to_string());
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        std::fs::create_dir_all(&work)?;
        println!("--- {} ({})", sc.name, f.display());
        let t = std::time::Instant::now();
        let fails = match run(game, &sc, toml::Table::new(), wasm, &work) {
            Ok((code, log)) => check(&sc, code, &log),
            Err(e) => vec![format!("{e:#}")],
        };
        if fails.is_empty() {
            println!("PASS {} ({:.1}s)", sc.name, t.elapsed().as_secs_f64());
        } else {
            println!("FAIL {} ({:.1}s), log: {}", sc.name, t.elapsed().as_secs_f64(), work.join("log.txt").display());
            for x in &fails {
                println!("    {x}");
            }
            failed.push(sc.name);
        }
    }
    println!("\n{} passed, {} failed", files.len() - failed.len(), failed.len());
    if !failed.is_empty() {
        bail!("failed: {}", failed.join(", "));
    }
    Ok(())
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
    let (code, log) = run(game, &sc, extra, false, &work)?;
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
