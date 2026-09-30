//! Locating, versioning and launching the game.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Game versions the mods were tested against: (sha256 of hlboot.dat, Steam build id).
pub const KNOWN_VERSIONS: &[(&str, &str)] =
    &[("57afd61ac3cc7f78ee1f7bbd183821a03d2f5725a49a477cd505444fa342d01c", "22056877")];

/// The HashLink JIT VM shipped with the game. (`Mosa Lina` itself is the HL/C native build,
/// which ignores bytecode files, so mods only work through the JIT.)
pub const JIT_EXE: &str = "Mosa Lina_jit";

/// The game directory: `arg`, else `$MOSA_GAME_DIR`, else the default Steam library.
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

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Steam build id of a known `hlboot.dat` hash.
pub fn known_version(hash: &str) -> Option<&'static str> {
    KNOWN_VERSIONS.iter().find(|(h, _)| *h == hash).map(|(_, b)| *b)
}

/// Read the game's pristine bytecode, warning about unknown versions.
pub fn read_bytecode(game: &Path) -> Result<Vec<u8>> {
    let path = game.join("hlboot.dat");
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    if known_version(&sha256(&bytes)).is_none() {
        eprintln!(
            "warning: unknown game version ({}). Mods check their assumptions and fail if the \
             code they patch changed.",
            sha256(&bytes)
        );
    }
    Ok(bytes)
}

/// The command that runs the JIT in `run_dir` (the overlay, or the game directory itself). With
/// `bytecode` it runs that file, else `run_dir/hlboot.dat`. The game's bundled libraries must be
/// on the library path; the JIT binary has no rpath for them.
///
/// `headless` runs without any window or display server: SDL's `offscreen` video driver renders
/// through EGL (Mesa). The game runs normally (faster than real time), which is what automated
/// tests need.
pub fn command(
    game: &Path,
    run_dir: &Path,
    bytecode: Option<&Path>,
    timeout: Option<u64>,
    headless: bool,
) -> Result<Command> {
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
    if let Some(b) = bytecode {
        cmd.arg(std::fs::canonicalize(b).with_context(|| format!("{} not found", b.display()))?);
    }
    cmd.current_dir(run_dir).env("LD_LIBRARY_PATH", ld);
    if headless {
        cmd.env("SDL_VIDEODRIVER", "offscreen").env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    }
    Ok(cmd)
}

/// Run the game (see [`command`]) with inherited stdio.
pub fn launch(
    game: &Path,
    run_dir: &Path,
    bytecode: Option<&Path>,
    timeout: Option<u64>,
    headless: bool,
) -> Result<ExitStatus> {
    command(game, run_dir, bytecode, timeout, headless)?.status().context("launching the game")
}
