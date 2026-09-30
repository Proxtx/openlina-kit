//! Install, build and run OpenLina mod packs. Used by the `openlina` helper that players run
//! and by `lina`, the development CLI.
//!
//! The game install is never modified:
//! - [`build`] runs every mod's patch over a copy of the game's `hlboot.dat`
//! - [`overlay`] makes a directory of symlinks to the install, with the patched bytecode and
//!   mod assets on top; the game runs from there (it resolves `fish/game/res` and `userdata`
//!   relative to its working directory)

pub mod game;
pub mod overlay;
pub mod package;
pub mod patch;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use openlina_sdk::manifest::{resolve_order, ModPack, PackInfo, PackInfoEntry};

pub use package::Package;
pub use zip;

/// Where the helper keeps its state: `$XDG_DATA_HOME/openlina` or `~/.local/share/openlina`.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("OPENLINA_HOME") {
        return Ok(PathBuf::from(d));
    }
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME not set")?).join(".local/share"),
    };
    Ok(base.join("openlina"))
}

/// The patched bytecode and, per mod, what it makes the game able to do outside the game
/// (see `openlina_sdk::caps`). Callers decide whether to accept those mods.
pub struct Built {
    pub bytes: Vec<u8>,
    pub caps: Vec<(String, Vec<openlina_sdk::caps::Finding>)>,
}

/// Apply `packages` (any order; they are sorted by their dependencies) to `input` with the
/// options from `pack`. Prints one line per mod.
pub fn build(input: Vec<u8>, packages: &[Package], pack: &ModPack) -> Result<Built> {
    let manifests: Vec<_> = packages.iter().map(|p| p.manifest.clone()).collect();
    let order = resolve_order(&manifests)?;
    let user_options = |id: &str| pack.mods.iter().find(|e| e.id == id).map(|e| e.options.clone()).unwrap_or_default();
    let mut info = PackInfo::default();
    for &i in &order {
        let m = &packages[i].manifest;
        info.mods.push(PackInfoEntry {
            id: m.info.id.clone(),
            name: m.info.name.clone(),
            version: m.info.version.clone(),
            section: m.info.section,
            options: m.resolve_options(&user_options(&m.info.id))?,
        });
    }
    let info_toml = toml::to_string(&info)?;
    let mut bytes = input;
    let mut before = openlina_sdk::Code::from_bytes(&bytes).context("the game's bytecode does not parse")?;
    let mut snap = openlina_sdk::caps::Snapshot::of(&before);
    let mut caps = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let p = &packages[i];
        let id = &p.manifest.info.id;
        let options = toml::to_string(&info.mods[k].options)?;
        let t = std::time::Instant::now();
        bytes = patch::run(&p.patch, &bytes, id, &options, &info_toml).with_context(|| format!("applying `{id}`"))?;
        // The result must still be valid bytecode.
        let after = openlina_sdk::Code::from_bytes(&bytes).with_context(|| format!("the bytecode after `{id}` does not parse"))?;
        let found = openlina_sdk::caps::diff(&snap, &before, &after);
        println!("  applied {id} {} ({:.1}s)", p.manifest.info.version, t.elapsed().as_secs_f64());
        if !found.is_empty() {
            caps.push((id.clone(), found));
        }
        snap = openlina_sdk::caps::Snapshot::of(&after);
        before = after;
    }
    Ok(Built { bytes, caps })
}

/// Copy `src` to `dst` recursively.
pub fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}
