//! `lina setup`, `mods`, `build`, `run`, `pack`, `check`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use openlina::patch::Patch;
use openlina::{game, overlay, package, Package};
use openlina_sdk::manifest::{ModManifest, ModPack, PackEntry, Section};
use openlina_sdk::{validate, Code};

use crate::{ORIG, OVERLAY};

pub fn setup(game_dir: &Path) -> Result<()> {
    let bytes = std::fs::read(game_dir.join("hlboot.dat"))?;
    let hash = game::sha256(&bytes);
    std::fs::create_dir_all("work")?;
    std::fs::write(ORIG, &bytes)?;
    println!("copied {} -> {ORIG}", game_dir.join("hlboot.dat").display());
    println!("sha256 {hash}");
    match game::known_version(&hash) {
        Some(build) => println!("known game version (Steam build {build})"),
        None => println!("WARNING: unknown game version. Run `lina check`, and test mods carefully."),
    }
    Ok(())
}

/// Every mod crate under mods/: (directory, manifest).
fn all_mods() -> Result<Vec<(PathBuf, ModManifest)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir("mods").context("no mods/ directory (run lina from the kit root)")? {
        let dir = e?.path();
        if dir.join("mod.toml").is_file() {
            let m = ModManifest::load(&dir.join("mod.toml"))?;
            let name = dir.file_name().unwrap().to_string_lossy();
            ensure!(m.info.id == name, "mods/{name}: mod id is `{}`, must match the directory", m.info.id);
            out.push((dir, m));
        }
    }
    out.sort_by(|a, b| (a.1.info.section, &a.1.info.id).cmp(&(b.1.info.section, &b.1.info.id)));
    Ok(out)
}

pub fn list_mods() -> Result<()> {
    for (_, m) in all_mods()? {
        println!("{} {} [{:?}]\n    {}", m.info.id, m.info.version, m.info.section, m.info.description);
        for (k, o) in &m.options {
            println!("    - {k} ({:?}, default {}): {}", o.kind, o.default, o.description);
        }
    }
    Ok(())
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("target"))
}

/// `cargo build --release` the given mod crates, natively or for wasm32-wasip1.
fn cargo_build(ids: &[String], wasm: bool) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "--quiet"]);
    if wasm {
        cmd.args(["--target", "wasm32-wasip1"]);
    }
    for id in ids {
        cmd.args(["-p", &format!("openlina-mod-{id}")]);
    }
    let status = cmd.status().context("running cargo")?;
    ensure!(
        status.success(),
        "cargo build failed (see the compiler output above){}",
        if wasm { "; wasm builds need the wasm32-wasip1 target: run inside `nix develop`" } else { "" }
    );
    Ok(())
}

fn patch_path(id: &str, wasm: bool) -> PathBuf {
    if wasm {
        target_dir().join("wasm32-wasip1/release").join(format!("{id}.wasm"))
    } else {
        target_dir().join("release").join(id)
    }
}

/// Mods selected by a modpack, or by `--mod` (plus what they require).
fn select(pack_path: &Path, only: &[String]) -> Result<(ModPack, Vec<(PathBuf, ModManifest)>)> {
    let all = all_mods()?;
    let pack = if only.is_empty() {
        ModPack::load(pack_path)?
    } else {
        let mut want: BTreeSet<String> = only.iter().cloned().collect();
        loop {
            let before = want.len();
            for (_, m) in &all {
                if want.contains(&m.info.id) {
                    want.extend(m.info.requires.iter().cloned());
                }
            }
            if want.len() == before {
                break;
            }
        }
        ModPack { openlina: 1, mods: want.into_iter().map(|id| PackEntry { id, ..Default::default() }).collect(), ..Default::default() }
    };
    let mut chosen = Vec::new();
    for e in &pack.mods {
        let found = all.iter().find(|(_, m)| m.info.id == e.id).with_context(|| format!("no mod `{}` in mods/", e.id))?;
        chosen.push(found.clone());
    }
    Ok((pack, chosen))
}

pub fn build(game_dir: &Path, pack_path: &Path, only: &[String], wasm: bool, out: &Path) -> Result<()> {
    let (pack, _) = select(pack_path, only)?;
    let bytes = build_pack(game_dir, &pack, wasm, Path::new(OVERLAY))?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(out, &bytes)?;
    println!("wrote {} and the overlay {OVERLAY}", out.display());
    Ok(())
}

/// Build the mods of `pack` from mods/, apply them to the pristine bytecode and create the
/// overlay game directory `overlay`. Returns the patched bytecode.
pub fn build_pack(game_dir: &Path, pack: &ModPack, wasm: bool, overlay: &Path) -> Result<Vec<u8>> {
    let all = all_mods()?;
    let mut chosen = Vec::new();
    for e in &pack.mods {
        let found = all.iter().find(|(_, m)| m.info.id == e.id).with_context(|| format!("no mod `{}` in mods/", e.id))?;
        chosen.push(found.clone());
    }
    let ids: Vec<String> = chosen.iter().map(|(_, m)| m.info.id.clone()).collect();
    cargo_build(&ids, wasm)?;
    let packages: Vec<Package> = chosen
        .into_iter()
        .map(|(dir, manifest)| {
            let id = manifest.info.id.clone();
            let patch = if wasm { Patch::Wasm(patch_path(&id, true)) } else { Patch::Native(patch_path(&id, false)) };
            Package { dir, manifest, patch }
        })
        .collect();
    let input = std::fs::read(ORIG).context("run `lina setup` first")?;
    println!("building {} mod(s){}", packages.len(), if wasm { " (wasm)" } else { "" });
    let bytes = openlina::build(input, &packages, pack)?;
    let mut assets = Vec::new();
    for p in &packages {
        assets.extend(p.assets()?);
    }
    overlay::create(game_dir, overlay, &bytes, &assets)?;
    Ok(bytes)
}

/// Add every mod `requires`d by the pack's mods (with default options).
pub fn add_requirements(pack: &mut ModPack) -> Result<()> {
    let all = all_mods()?;
    loop {
        let mut missing = Vec::new();
        for e in &pack.mods {
            let (_, m) = all.iter().find(|(_, m)| m.info.id == e.id).with_context(|| format!("no mod `{}` in mods/", e.id))?;
            for r in &m.info.requires {
                if !pack.mods.iter().any(|x| &x.id == r) && !missing.contains(r) {
                    missing.push(r.clone());
                }
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        pack.mods.extend(missing.into_iter().map(|id| PackEntry { id, ..Default::default() }));
    }
}

pub fn run(game_dir: &Path, timeout: Option<u64>, headless: bool) -> Result<()> {
    let overlay = Path::new(OVERLAY);
    ensure!(overlay.join("hlboot.dat").is_file(), "run `lina build` first");
    let bc = std::fs::canonicalize(overlay.join("hlboot.dat"))?;
    let status = game::launch(game_dir, &std::fs::canonicalize(overlay)?, Some(&bc), timeout, headless)?;
    // 124 = killed by `timeout`, which is what we asked for.
    if !status.success() && status.code() != Some(124) {
        bail!("game exited with {status}");
    }
    Ok(())
}

pub fn pack(ids: &[String], bundle: Option<&str>, from: Option<&ModPack>, out: &Path) -> Result<()> {
    let all = all_mods()?;
    let chosen: Vec<_> = if ids.is_empty() {
        all.iter().filter(|(_, m)| m.info.section != Section::Dev).cloned().collect()
    } else {
        ids.iter()
            .map(|id| all.iter().find(|(_, m)| &m.info.id == id).cloned().with_context(|| format!("no mod `{id}`")))
            .collect::<Result<_>>()?
    };
    // A bundle must be installable on its own: add what the chosen mods require.
    let mut chosen = chosen;
    if bundle.is_some() {
        loop {
            let missing: Vec<String> = chosen
                .iter()
                .flat_map(|(_, m)| m.info.requires.clone())
                .filter(|r| !chosen.iter().any(|(_, c)| &c.info.id == r))
                .collect();
            if missing.is_empty() {
                break;
            }
            for r in missing {
                let found = all.iter().find(|(_, m)| m.info.id == r).cloned().with_context(|| format!("no mod `{r}`"))?;
                if !chosen.iter().any(|(_, c)| c.info.id == r) {
                    println!("adding required mod `{r}` to the bundle");
                    chosen.push(found);
                }
            }
        }
    }
    let id_list: Vec<String> = chosen.iter().map(|(_, m)| m.info.id.clone()).collect();
    cargo_build(&id_list, true)?;
    std::fs::create_dir_all(out)?;

    // Stage each package: mod.toml, patch.wasm, assets/, media/.
    let stage = out.join(".stage");
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }
    for (dir, m) in &chosen {
        let id = &m.info.id;
        let s = stage.join("mods").join(id);
        std::fs::create_dir_all(&s)?;
        std::fs::copy(dir.join("mod.toml"), s.join("mod.toml"))?;
        std::fs::copy(patch_path(id, true), s.join("patch.wasm"))?;
        for sub in ["assets", "media"] {
            if dir.join(sub).is_dir() {
                openlina::copy_dir(&dir.join(sub), &s.join(sub))?;
            }
        }
        copy_source(dir, &s.join("source"))?;
        let zip_path = out.join(format!("{id}-{}.zip", m.info.version));
        let mut zip = openlina::zip::ZipWriter::new(std::fs::File::create(&zip_path)?);
        package::zip_dir(&s, id, &mut zip)?;
        zip.finish()?;
        println!("wrote {}", zip_path.display());
    }

    if let Some(name) = bundle {
        let root = stage.join(name);
        std::fs::create_dir_all(&root)?;
        openlina::copy_dir(&stage.join("mods"), &root.join("mods"))?;
        let pack = ModPack {
            openlina: 1,
            mods: chosen
                .iter()
                .map(|(_, m)| {
                    let given = from.and_then(|f| f.mods.iter().find(|e| e.id == m.info.id));
                    PackEntry {
                        id: m.info.id.clone(),
                        version: Some(m.info.version.clone()),
                        options: given.map(|e| e.options.clone()).unwrap_or_default(),
                        request: given.and_then(|e| e.request.clone()),
                        ..Default::default()
                    }
                })
                .collect(),
            ..Default::default()
        };
        pack.save(&root.join("modpack.toml"))?;
        let status = Command::new("cargo").args(["build", "--release", "--quiet", "-p", "openlina"]).status()?;
        ensure!(status.success(), "building the openlina helper failed");
        std::fs::copy(target_dir().join("release/openlina"), root.join("openlina"))?;
        let zip_path = out.join(format!("{name}.zip"));
        let mut zip = openlina::zip::ZipWriter::new(std::fs::File::create(&zip_path)?);
        package::zip_dir(&root, name, &mut zip)?;
        zip.finish()?;
        println!("wrote {} (run `./openlina install .` inside it)", zip_path.display());
    }
    std::fs::remove_dir_all(&stage)?;
    Ok(())
}

/// The mod's crate (Cargo.toml, src/, tests/, art/, levels/, …) into a package's `source/`, so
/// whoever pulls the package can change and rebuild it (`lina pull` puts it back into mods/<id>/).
/// mod.toml, assets/ and media/ are at the package root already; build outputs are skipped.
fn copy_source(dir: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        let name = name.to_string_lossy();
        if ["mod.toml", "assets", "media", "target"].contains(&name.as_ref()) || name.starts_with('.') {
            continue;
        }
        if e.file_type()?.is_dir() {
            openlina::copy_dir(&e.path(), &dst.join(&*name))?;
        } else {
            std::fs::copy(e.path(), dst.join(&*name))?;
        }
    }
    Ok(())
}

pub fn check(input: &Path) -> Result<()> {
    let code = Code::load(input)?;
    let bytes = code.to_bytes()?;
    let again = Code::from_bytes(&bytes)?;
    if again.bc.functions.len() != code.bc.functions.len()
        || again.bc.types.len() != code.bc.types.len()
        || again.to_bytes()?.len() != bytes.len()
    {
        bail!("serializer roundtrip changed the bytecode");
    }
    println!("roundtrip ok ({} bytes)", bytes.len());
    let mut problems = 0;
    for f in &code.bc.functions {
        for e in validate::check_function(&code, f) {
            if problems < 20 {
                println!("  {e}");
            }
            problems += 1;
        }
    }
    println!(
        "validator: {problems} problem(s) in {} vanilla functions (should be 0; anything else is a validator false positive)",
        code.bc.functions.len()
    );
    Ok(())
}
