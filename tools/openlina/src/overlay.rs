//! The overlay game directory.
//!
//! The game finds its resources (`./fish/game/res`) and saves (`./userdata`) relative to its
//! working directory. The overlay is a directory of symlinks to every entry of the install, with
//! the patched `hlboot.dat` and mod assets added on top. Running the game there leaves the
//! install untouched, and `userdata` stays shared with the vanilla game.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

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

    let res = Path::new("fish/game/res");
    for (rel, src) in assets {
        let dst_rel = res.join(rel);
        if let Some(parent) = dst_rel.parent() {
            materialize(overlay, parent)?;
        }
        let dst = overlay.join(&dst_rel);
        if dst.symlink_metadata().is_ok() {
            std::fs::remove_file(&dst)?; // a mod asset replaces a game file of the same name
        }
        std::fs::copy(src, &dst).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(())
}

/// Make every directory along `rel` (inside `root`) a real directory, replacing a symlinked
/// directory by a real one holding symlinks to its entries.
fn materialize(root: &Path, rel: &Path) -> Result<()> {
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        cur.push(c);
        match cur.symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() => {
                let target = std::fs::read_link(&cur)?;
                if !target.is_dir() {
                    bail!("{} is not a directory", target.display());
                }
                std::fs::remove_file(&cur)?;
                std::fs::create_dir(&cur)?;
                for e in std::fs::read_dir(&target)? {
                    let e = e?;
                    symlink(&e.path(), &cur.join(e.file_name()))?;
                }
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => bail!("{} is not a directory", cur.display()),
            Err(_) => std::fs::create_dir(&cur)?,
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
