//! `lina doctor`: check that this machine can build, test and share mods, and say how to fix what's
//! missing. Works with and without nix.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

struct Report {
    missing: usize,
}

impl Report {
    fn ok(&self, what: &str, detail: &str) {
        println!("  ok    {what}: {detail}");
    }
    fn warn(&self, what: &str, fix: &str) {
        println!("  warn  {what}\n        {fix}");
    }
    fn fail(&mut self, what: &str, fix: &str) {
        self.missing += 1;
        println!("  MISS  {what}\n        {fix}");
    }
}

pub fn doctor(game_dir: Option<PathBuf>) -> Result<()> {
    let mut r = Report { missing: 0 };
    let nix = std::env::var_os("IN_NIX_SHELL").is_some();
    println!("toolchain ({})", if nix { "nix dev shell" } else { "no nix: rustup" });
    match run("cargo", &["--version"]) {
        Some(v) => r.ok(
            "cargo",
            &format!(
                "{v}{}",
                run("rustup", &["show", "active-toolchain"]).map(|t| format!(" (rustup: {t})")).unwrap_or_default()
            ),
        ),
        None => r.fail("cargo", "install Rust with rustup (https://rustup.rs), or use `nix develop`"),
    }
    let wasm = run("rustc", &["--print", "sysroot"]).map(|s| Path::new(&s).join("lib/rustlib/wasm32-wasip1").is_dir());
    match wasm {
        Some(true) => r.ok("wasm target", "wasm32-wasip1 (what players run)"),
        _ => r.fail(
            "wasm target wasm32-wasip1",
            "`rustup target add wasm32-wasip1` (adds it to the toolchain cargo uses here; ask the user first), or use `nix develop`",
        ),
    }
    match crate::scenario::magick() {
        m if run(m, &["-version"]).is_some_and(|v| v.contains("ImageMagick")) => {
            r.ok("ImageMagick", &format!("`{m}` (lina gif)"))
        }
        _ => r.warn(
            "ImageMagick not found: `lina gif` can't assemble gifs",
            "install imagemagick with your package manager",
        ),
    }
    if run("gifsicle", &["--version"]).is_some() {
        r.ok("gifsicle", "gifs get optimized");
    } else {
        r.warn("gifsicle not found (optional)", "install gifsicle for smaller gifs");
    }

    println!("game");
    if !cfg!(target_os = "linux") {
        r.warn(
            "not Linux",
            "building and packing work; running the game (lina run/test/gif) is only tested on Linux so far",
        );
    }
    match openlina::game::game_dir(game_dir) {
        Ok(dir) => {
            r.ok("install", &dir.display().to_string());
            match std::fs::read(dir.join("hlboot.dat")) {
                Ok(b) => {
                    let h = openlina::game::sha256(&b);
                    match openlina::game::known_version(&h) {
                        Some(build) => r.ok("version", &format!("Steam build {build}")),
                        None => r.warn(
                            "unknown game version",
                            "mods may need updates for this build; `lina check` and the tests tell",
                        ),
                    }
                }
                Err(_) => r.fail("hlboot.dat", "the install looks incomplete (verify the game files in Steam)"),
            }
        }
        Err(e) => r.fail("Mosa Lina install", &format!("{e:#}; pass --game-dir or set MOSA_GAME_DIR")),
    }
    if Path::new(crate::ORIG).is_file() {
        r.ok("lina setup", crate::ORIG);
    } else {
        r.fail("lina setup not run", "`./lina setup`");
    }
    if Path::new("work/dump/functions.tsv").is_file() {
        r.ok("lina dump", "work/dump/");
    } else {
        r.warn("no dump yet", "`./lina dump` (needed to find game code)");
    }

    println!("sharing");
    match crate::hub::logged_in() {
        Some(site) => r.ok("OpenLina site", &site),
        None => r.warn("not logged in to an OpenLina site (only for publishing)", "`./lina login <site> <token>`"),
    }

    if r.missing > 0 {
        bail!("{} thing(s) missing", r.missing);
    }
    println!("\nall set");
    Ok(())
}
