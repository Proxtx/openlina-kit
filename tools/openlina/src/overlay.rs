//! The overlay game directory.
//!
//! The game finds its resources (`./fish/game/res`) and saves (`./userdata`) relative to its
//! working directory. The overlay is a directory of symlinks to every entry of the install, with
//! the patched `hlboot.dat` on top. When mods ship assets, `fish/` is mirrored with hard links
//! (symlinks inside the resource tree confuse Heaps' path handling) and the assets are copied in.
//! Running the game there leaves the install untouched, and `userdata` stays shared with the
//! vanilla game.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// (Re)create `overlay` for `game` with `bytecode` and `assets` (paths relative to
/// `fish/game/res`, source files).
pub fn create(game: &Path, overlay: &Path, bytecode: &[u8], assets: &[(PathBuf, PathBuf)]) -> Result<()> {
    if overlay.exists() {
        // Only symlinks and files we created live here; symlinks are removed, not followed.
        std::fs::remove_dir_all(overlay).with_context(|| format!("removing {}", overlay.display()))?;
    }
    std::fs::create_dir_all(overlay)?;
    for e in std::fs::read_dir(game)? {
        let e = e?;
        if e.file_name() == "hlboot.dat" {
            continue;
        }
        symlink(&e.path(), &overlay.join(e.file_name()))?;
    }
    std::fs::write(overlay.join("hlboot.dat"), bytecode)?;

    if !assets.is_empty() {
        // Heaps resolves resource paths through realpath and slices them against its base
        // directory, so symlinked entries below the resource root break it. Mirror `fish/` as
        // real directories with hard links (no disk space; copies across filesystems).
        let fish = overlay.join("fish");
        std::fs::remove_file(&fish)?;
        mirror(&game.join("fish"), &fish)?;
    }
    let res = Path::new("fish/game/res");
    for (rel, src) in assets {
        let dst = overlay.join(res).join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dst.symlink_metadata().is_ok() {
            std::fs::remove_file(&dst)?; // a mod asset replaces a game file of the same name
        }
        std::fs::copy(src, &dst).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(())
}

/// Recreate `src` at `dst`: real directories, files hard-linked (copied if that fails).
fn mirror(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if e.file_type()?.is_dir() {
            mirror(&from, &to)?;
        } else if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst).with_context(|| format!("linking {}", dst.display()))
}

#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst)
    } else {
        std::os::windows::fs::symlink_file(src, dst)
    }
    .with_context(|| format!("linking {}", dst.display()))
}
