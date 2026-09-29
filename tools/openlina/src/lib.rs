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
use openlina_sdk::manifest::{resolve_order, ModPack};

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

/// Apply `packages` (any order; they are sorted by their dependencies) to `input` with the
/// options from `pack`. Prints one line per mod.
pub fn build(input: Vec<u8>, packages: &[Package], pack: &ModPack) -> Result<Vec<u8>> {
    let manifests: Vec<_> = packages.iter().map(|p| p.manifest.clone()).collect();
    let order = resolve_order(&manifests)?;
    let mut bytes = input;
    for i in order {
        let p = &packages[i];
        let id = &p.manifest.info.id;
        let user = pack.mods.iter().find(|e| &e.id == id).map(|e| e.options.clone()).unwrap_or_default();
        let options = p.manifest.resolve_options(&user)?;
        let t = std::time::Instant::now();
        bytes = patch::run(&p.patch, &bytes, id, &toml::to_string(&options)?)
            .with_context(|| format!("applying `{id}`"))?;
        println!("  applied {id} {} ({:.1}s)", p.manifest.info.version, t.elapsed().as_secs_f64());
    }
    // The result must still be valid bytecode.
    openlina_sdk::Code::from_bytes(&bytes).context("the patched bytecode does not parse")?;
    Ok(bytes)
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
