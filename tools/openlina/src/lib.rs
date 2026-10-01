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

use anyhow::{bail, Context, Result};
use openlina_sdk::kit::{self, KitIssue, KitVersion};
use openlina_sdk::manifest::{option_conflicts, resolve_order, ModManifest, ModPack, PackInfo, PackInfoEntry};

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

/// Who builds a pack, for the wording of what to do when it can't be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// `lina` in a kit checkout.
    Kit,
    /// The players' `openlina`.
    Helper,
}

/// Why mods can't be used by this tool (kit `KIT_VERSION`), worded for `host`; empty if they can.
pub fn kit_problems<'a>(mods: impl IntoIterator<Item = &'a ModManifest>, host: Host) -> Vec<String> {
    let cur = KitVersion::current();
    kit::issues(mods, cur)
        .into_iter()
        .map(|i| match (&i, host) {
            (KitIssue::Port { id, kit, .. }, Host::Kit) => format!(
                "{} was made for openlina-kit {}; this kit is {cur}, which changed what mods rely on. Port it: \
                 CHANGELOG.md lists the changes since {kit}; then set `kit = \"{cur}\"` in mods/{id}/mod.toml",
                i.describe(),
                kit.line_name()
            ),
            (KitIssue::Update { kit, .. }, Host::Kit) => {
                format!("{} needs openlina-kit {kit} or newer; this checkout is {cur}: `git pull`", i.describe())
            }
            (KitIssue::Port { kit, .. }, Host::Helper) => format!(
                "{} was made for an older openlina ({}.x); this openlina is {cur}. An agent with openlina-kit can \
                 update it: give it the pack link (`lina pull <link>`)",
                i.describe(),
                kit.line_name()
            ),
            (KitIssue::Update { kit, .. }, Host::Helper) => format!(
                "{} needs openlina {kit} or newer (this is {cur}): download the pack zip again (it comes with the \
                 current openlina) or get openlina from https://github.com/Proxtx/openlina-kit/releases",
                i.describe()
            ),
        })
        .collect()
}

/// Refuse a pack this tool can't build: mods for another kit version, conflicting options.
pub fn check_pack(packages: &[Package], pack: &ModPack, host: Host) -> Result<()> {
    let kits = kit_problems(packages.iter().map(|p| &p.manifest), host);
    if !kits.is_empty() {
        bail!("{}", kits.join("\n"));
    }
    let user_options = |id: &str| pack.mods.iter().find(|e| e.id == id).map(|e| e.options.clone()).unwrap_or_default();
    let resolved: Vec<toml::Table> = packages
        .iter()
        .map(|p| p.manifest.resolve_options(&user_options(&p.manifest.info.id)))
        .collect::<Result<_>>()?;
    let pairs: Vec<_> = packages.iter().map(|p| &p.manifest).zip(&resolved).collect();
    let clashes = option_conflicts(&pairs);
    if !clashes.is_empty() {
        bail!(
            "these options can't work together:\n  {}\nChange one of the options{}",
            clashes.join("\n  "),
            if host == Host::Helper { " (`openlina set <mod> <option>=<value>`)" } else { "" }
        );
    }
    Ok(())
}

/// Apply `packages` (any order; they are sorted by their dependencies) to `input` with the
/// options from `pack`. Reports one line per mod to `log`. Refuses what [`check_pack`] refuses.
pub fn build(
    input: Vec<u8>,
    packages: &[Package],
    pack: &ModPack,
    host: Host,
    log: &mut dyn FnMut(String),
) -> Result<Built> {
    let manifests: Vec<_> = packages.iter().map(|p| p.manifest.clone()).collect();
    let order = resolve_order(&manifests)?;
    check_pack(packages, pack, host)?;
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
        let after = openlina_sdk::Code::from_bytes(&bytes)
            .with_context(|| format!("the bytecode after `{id}` does not parse"))?;
        let found = openlina_sdk::caps::diff(&snap, &before, &after);
        log(format!("  applied {id} {} ({:.1}s)", p.manifest.info.version, t.elapsed().as_secs_f64()));
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
