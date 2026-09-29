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
//!
//! [gif]                             # used by `lina gif`
//! capture = "60-420/2"              # level ticks from-to/step (120 ticks per second)
//! out = "media/wrap.gif"            # relative to the mod directory
//! scale = 1                         # 1 = 600x338
//! ```
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
fn run(game: &Path, sc: &Scenario, extra_harness: toml::Table, wasm: bool, work: &Path) -> Result<(Option<i32>, String)> {
    let pack = sc.pack(extra_harness)?;
    let overlay = work.join("game");
    let bytes = build::build_pack(game, &pack, wasm, &overlay)?;
    std::fs::write(work.join("hlboot.dat"), &bytes)?;
    let out = openlina::game::command(game, &std::fs::canonicalize(&overlay)?, Some(&overlay.join("hlboot.dat")), Some(sc.timeout), true)?
        .output()
        .context("running the game")?;
    let mut log = String::from_utf8_lossy(&out.stdout).to_string();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    // Steam's client library is chatty; keep the log readable.
    let log: String = log
        .lines()
        .filter(|l| !l.starts_with("[S_API") && !l.starts_with("[STEAM]") && !l.starts_with("Setting breakpad") && !l.starts_with("SteamInternal"))
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
    for bad in ["SIGNAL", "Uncaught exception", "[harness] ERROR"] {
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
    let mut cmd = Command::new("magick");
    cmd.arg("-delay").arg(format!("{step}x120")).arg("-loop").arg("0");
    cmd.args(&pngs);
    if spec.scale > 1 {
        cmd.arg("-filter").arg("point").arg("-resize").arg(format!("{}%", spec.scale * 100));
    }
    cmd.arg("-layers").arg("Optimize").arg(&dest);
    let st = cmd.status().context("running ImageMagick `magick` (inside `nix develop`)")?;
    ensure!(st.success(), "magick failed");
    if Command::new("gifsicle").arg("-O3").arg("--batch").arg(&dest).status().is_err() {
        eprintln!("(gifsicle not found; gif not optimized)");
    }
    let size = std::fs::metadata(&dest)?.len();
    println!("wrote {} ({} frames, {:.0} KB)", dest.display(), pngs.len(), size as f64 / 1024.0);
    Ok(())
}
