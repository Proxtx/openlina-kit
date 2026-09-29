//! Locating, versioning and launching the game.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Pristine bytecode copied from the game by `mosa setup`.
pub const ORIG: &str = "work/hlboot.orig.dat";
/// Output of `mosa build`.
pub const MODDED: &str = "work/hlboot.modded.dat";

/// Game versions the mods were written against: (sha256 of hlboot.dat, Steam build id).
pub const KNOWN_VERSIONS: &[(&str, &str)] =
    &[("57afd61ac3cc7f78ee1f7bbd183821a03d2f5725a49a477cd505444fa342d01c", "22056877")];

/// The HashLink JIT VM shipped with the game. (`Mosa Lina` itself is the HL/C native build,
/// which ignores bytecode files, so mods only work through the JIT.)
pub const JIT_EXE: &str = "Mosa Lina_jit";

pub fn game_dir(arg: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match arg.or_else(|| std::env::var_os("MOSA_GAME_DIR").map(PathBuf::from)) {
        Some(d) => d,
        None => {
            let home = std::env::var_os("HOME").context("HOME not set")?;
            PathBuf::from(home).join(".local/share/Steam/steamapps/common/Mosa Lina")
        }
    };
    if !dir.join("hlboot.dat").is_file() || !dir.join(JIT_EXE).is_file() {
        bail!(
            "{} does not look like a Mosa Lina install (need hlboot.dat and `{JIT_EXE}`); \
             pass --game-dir or set MOSA_GAME_DIR",
            dir.display()
        );
    }
    Ok(dir)
}

pub fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub fn known_version(hash: &str) -> Option<&'static str> {
    KNOWN_VERSIONS.iter().find(|(h, _)| *h == hash).map(|(_, b)| *b)
}

pub fn setup(game: &Path) -> Result<()> {
    let src = game.join("hlboot.dat");
    let hash = sha256(&src)?;
    std::fs::create_dir_all("work")?;
    std::fs::copy(&src, ORIG)?;
    println!("copied {} -> {ORIG}", src.display());
    println!("sha256 {hash}");
    match known_version(&hash) {
        Some(build) => println!("known game version (Steam build {build})"),
        None => println!(
            "WARNING: unknown game version. Mods verify their anchors and will refuse to build \
             if the code changed, but run `mosa check` and test carefully."
        ),
    }
    Ok(())
}

/// Launch the JIT with `bytecode`. The working directory must be the game directory so the
/// game finds `fish/game/res` and `userdata/`, and the game's bundled libraries need to be on
/// the library path (the JIT binary has no rpath for them).
pub fn run(game: &Path, bytecode: &Path, timeout: Option<u64>) -> Result<()> {
    let bytecode = std::fs::canonicalize(bytecode)
        .with_context(|| format!("{} not found (run `mosa build`?)", bytecode.display()))?;
    let mut ld = game.as_os_str().to_owned();
    if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
        ld.push(":");
        ld.push(old);
    }
    let exe = game.join(JIT_EXE);
    let mut cmd = match timeout {
        Some(t) => {
            let mut c = Command::new("timeout");
            c.arg(t.to_string()).arg(&exe);
            c
        }
        None => Command::new(&exe),
    };
    cmd.arg(&bytecode).current_dir(game).env("LD_LIBRARY_PATH", ld);
    println!("launching {} {}", exe.display(), bytecode.display());
    let status = cmd.status().context("launching the game")?;
    // 124 = killed by `timeout`, which is what we asked for.
    if !status.success() && status.code() != Some(124) {
        bail!("game exited with {status}");
    }
    Ok(())
}
