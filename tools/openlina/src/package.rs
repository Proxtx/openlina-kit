//! Mod packages.
//!
//! A package is a directory (or a zip of one):
//!
//! ```text
//! <id>/
//!   mod.toml      metadata, options (see openlina_sdk::manifest)
//!   patch.wasm    the patch, wasm32-wasip1
//!   assets/       files overlaid onto the game's `fish/game/res/` (e.g. assets/images/x.png)
//!   media/        icon and showcase gifs for the website (not installed into the game)
//! ```
//!
//! A pack zip holds `modpack.toml` and `mods/<id>/` package directories, plus the helper.

use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use openlina_sdk::manifest::ModManifest;

use crate::patch::Patch;

#[derive(Debug, Clone)]
pub struct Package {
    pub dir: PathBuf,
    pub manifest: ModManifest,
    pub patch: Patch,
}

impl Package {
    /// Load an installed package directory.
    pub fn load(dir: &Path) -> Result<Self> {
        let manifest = ModManifest::load(&dir.join("mod.toml"))?;
        let wasm = dir.join("patch.wasm");
        if !wasm.is_file() {
            bail!("{}: missing patch.wasm", dir.display());
        }
        Ok(Self { dir: dir.to_path_buf(), manifest, patch: Patch::Wasm(wasm) })
    }

    /// Files to overlay onto `fish/game/res`: (path relative to res, source file).
    pub fn assets(&self) -> Result<Vec<(PathBuf, PathBuf)>> {
        let root = self.dir.join("assets");
        let mut out = Vec::new();
        if root.is_dir() {
            walk(&root, &root, &mut out)?;
        }
        Ok(out)
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(root, &p, out)?;
        } else {
            out.push((p.strip_prefix(root)?.to_path_buf(), p));
        }
    }
    Ok(())
}

/// Extract a zip into `dst`, refusing entries that would escape it.
pub fn unzip(reader: impl Read + Seek, dst: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(reader)?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { bail!("unsafe path in zip: {}", f.name()) };
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            bail!("unsafe path in zip: {}", f.name());
        }
        let out = dst.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            std::fs::write(&out, buf).with_context(|| format!("writing {}", out.display()))?;
        }
    }
    Ok(())
}

/// Zip the contents of `src` under the prefix `prefix/` (empty for none).
pub fn zip_dir(src: &Path, prefix: &str, zip: &mut zip::ZipWriter<impl Write + Seek>) -> Result<()> {
    let mut files = Vec::new();
    walk(src, src, &mut files)?;
    files.sort();
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (rel, path) in files {
        let name = if prefix.is_empty() { rel.to_string_lossy().to_string() } else { format!("{prefix}/{}", rel.to_string_lossy()) };
        let mode = if is_executable(&path) { 0o755 } else { 0o644 };
        zip.start_file(name.replace('\\', "/"), opts.unix_permissions(mode))?;
        zip.write_all(&std::fs::read(&path)?)?;
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_: &Path) -> bool {
    false
}
