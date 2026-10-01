//! `openlina`: install and play OpenLina mod packs for Mosa Lina.
//!
//! ```text
//! openlina install openlina-pack.zip     install the mods of a pack (or single mod zips/dirs)
//! openlina launch-option                 print the Steam launch option to play modded
//! openlina run                           play without Steam's launcher
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use openlina::{data_dir, game, overlay, package, Package};
use openlina_sdk::manifest::{ModPack, PackEntry};

#[derive(Parser)]
#[command(name = "openlina", about = "Install and play OpenLina mod packs for Mosa Lina", version)]
struct Cli {
    /// Game install directory (default: $MOSA_GAME_DIR, then the default Steam library).
    #[arg(long, global = true)]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install a pack zip, a mod zip, or a directory holding either, then build. Adds to what is
    /// installed; with --replace, the installed mods become exactly these.
    Install {
        paths: Vec<PathBuf>,
        /// Install mods the site has not reviewed yet without asking.
        #[arg(long)]
        yes: bool,
        /// Uninstall the mods that are not in these packs ("play exactly this pack").
        #[arg(long)]
        replace: bool,
    },
    /// Allow an installed mod to reach outside the game (files, programs, network, Steam), after
    /// the build refused it. Only for mods you trust; the allowance ends when the mod changes.
    Allow { id: String },
    /// Remove an installed mod, then build.
    Uninstall { id: String },
    /// List installed mods and their options.
    List,
    /// Set a mod option, e.g. `openlina set screen-wrap coins=true`, then build.
    Set { id: String, assignment: String },
    /// Patch the game's bytecode with the installed mods (done automatically by install/set).
    Build,
    /// Play the modded game.
    Run {
        #[arg(long)]
        timeout: Option<u64>,
        /// No window (SDL offscreen driver), e.g. for automated checks.
        #[arg(long)]
        headless: bool,
    },
    /// Steam launch wrapper: `openlina steam %command%`. Plays modded if built, else runs the
    /// original command.
    Steam {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Print the Steam launch option.
    LaunchOption,
    /// Bundle what a mod author (or their agent) needs to look into a problem: installed mods,
    /// versions and options, the game version, the last runs' output and the game's crash report.
    Report {
        /// Where to write the zip (default: ./openlina-report-<date>.zip).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data = data_dir()?;
    let game_dir = || game::game_dir(cli.game_dir.clone());
    match cli.cmd {
        Cmd::Install { paths, yes, replace } => {
            // every given pack counts for the others' requirements, whatever the order
            let mut given = Vec::new();
            for p in &paths {
                given.extend(ids_in(&data, p)?);
            }
            let mut incoming = Vec::new();
            for p in &paths {
                incoming.extend(install(&data, p, yes, &given)?);
            }
            if replace {
                let pack = load_state(&data)?;
                for e in pack.mods.iter().filter(|e| !incoming.contains(&e.id)) {
                    remove_mod(&data, &e.id)?;
                    println!("uninstalled {} (not in the pack)", e.id);
                }
            }
            print_installed(&data, &incoming)?;
            install_self(&data)?;
            build(&data, &game_dir()?)?;
            print_launch_option(&data);
            Ok(())
        }
        Cmd::Uninstall { id } => {
            remove_mod(&data, &id)?;
            build(&data, &game_dir()?)
        }
        Cmd::List => {
            let pack = load_state(&data)?;
            if pack.mods.is_empty() {
                println!("no mods installed");
            }
            for e in &pack.mods {
                let p = Package::load(&data.join("mods").join(&e.id))?;
                println!("{} {}  {}", e.id, p.manifest.info.version, p.manifest.info.name);
                for (k, v) in p.manifest.resolve_options(&e.options)? {
                    println!("    {k} = {v}");
                }
            }
            Ok(())
        }
        Cmd::Set { id, assignment } => {
            let (k, v) = assignment.split_once('=').context("expected key=value")?;
            let value: toml::Value = toml::from_str::<toml::Table>(&format!("v = {v}"))
                .ok()
                .and_then(|mut t| t.remove("v"))
                .unwrap_or_else(|| toml::Value::String(v.to_string()));
            let mut pack = load_state(&data)?;
            let e = pack.mods.iter_mut().find(|e| e.id == id).with_context(|| format!("`{id}` is not installed"))?;
            e.options.insert(k.trim().to_string(), value);
            let p = Package::load(&data.join("mods").join(&id))?;
            p.manifest.resolve_options(&e.options)?;
            // Refuse option combinations the mods declare impossible before saving anything.
            let packages: Vec<Package> =
                pack.mods.iter().map(|e| Package::load(&data.join("mods").join(&e.id))).collect::<Result<_>>()?;
            openlina::check_pack(&packages, &pack, openlina::Host::Helper)?;
            pack.save(&data.join("modpack.toml"))?;
            build(&data, &game_dir()?)
        }
        Cmd::Build => build(&data, &game_dir()?),
        Cmd::Allow { id } => {
            let pack = load_state(&data)?;
            if !pack.mods.iter().any(|e| e.id == id) {
                bail!("`{id}` is not installed");
            }
            let mut trust = load_trust(&data)?;
            trust.insert(id.clone(), toml::Value::String(patch_hash(&data, &id)?));
            save_trust(&data, &trust)?;
            println!("allowed {id} (until it changes)");
            build(&data, &game_dir()?)
        }
        Cmd::Run { timeout, headless } => {
            let game = game_dir()?;
            let overlay = data.join("game");
            if !overlay.join("hlboot.dat").is_file() {
                bail!("nothing built yet: run `openlina install <pack>` first");
            }
            game::launch_logged(
                &game,
                &overlay,
                Some(&overlay.join("hlboot.dat")),
                timeout,
                headless,
                &data.join("last-run.log"),
            )?;
            Ok(())
        }
        Cmd::Report { out } => report(&data, game_dir().ok(), out),
        Cmd::Steam { command } => {
            let overlay = data.join("game");
            let modded = overlay.join("hlboot.dat").is_file() && std::env::var_os("MOSA_VANILLA").is_none();
            match (modded, game_dir()) {
                (true, Ok(game)) => {
                    let status = game::launch_logged(
                        &game,
                        &overlay,
                        Some(&overlay.join("hlboot.dat")),
                        None,
                        false,
                        &data.join("last-run.log"),
                    )?;
                    std::process::exit(status.code().unwrap_or(1));
                }
                _ => {
                    let (exe, args) = command.split_first().context("no command to run")?;
                    let status = std::process::Command::new(exe).args(args).status()?;
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        Cmd::LaunchOption => {
            print_launch_option(&data);
            Ok(())
        }
    }
}

/// Remove an installed mod (files, state, trust).
fn remove_mod(data: &Path, id: &str) -> Result<()> {
    let mut pack = load_state(data)?;
    let before = pack.mods.len();
    pack.mods.retain(|e| e.id != id);
    if pack.mods.len() == before {
        bail!("`{id}` is not installed");
    }
    let dir = data.join("mods").join(id);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    pack.save(&data.join("modpack.toml"))?;
    let mut trust = load_trust(data)?;
    if trust.remove(id).is_some() {
        save_trust(data, &trust)?;
    }
    Ok(())
}

/// What is installed now; mods that were there before and are not in `incoming` are marked.
fn print_installed(data: &Path, incoming: &[String]) -> Result<()> {
    let pack = load_state(data)?;
    println!("\ninstalled now:");
    let mut others = 0;
    for e in &pack.mods {
        let version =
            Package::load(&data.join("mods").join(&e.id)).map(|p| p.manifest.info.version).unwrap_or_default();
        let mark = if incoming.contains(&e.id) {
            ""
        } else {
            others += 1;
            "   (installed earlier, not in this pack)"
        };
        println!("  {} {version}{mark}", e.id);
    }
    if others > 0 {
        println!("To play exactly the pack, install it with `--replace` (or `openlina uninstall <id>`).");
    }
    Ok(())
}

fn load_state(data: &Path) -> Result<ModPack> {
    let p = data.join("modpack.toml");
    if p.exists() {
        ModPack::load(&p)
    } else {
        Ok(ModPack { openlina: 1, ..Default::default() })
    }
}

/// Mods allowed to reach outside the game: id → sha256 of the patch.wasm that was allowed.
fn load_trust(data: &Path) -> Result<toml::Table> {
    let p = data.join("trust.toml");
    if !p.exists() {
        return Ok(toml::Table::new());
    }
    toml::from_str(&std::fs::read_to_string(&p)?).with_context(|| format!("parsing {}", p.display()))
}

fn save_trust(data: &Path, t: &toml::Table) -> Result<()> {
    std::fs::write(data.join("trust.toml"), toml::to_string(t)?)?;
    Ok(())
}

fn patch_hash(data: &Path, id: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(data.join("mods").join(id).join("patch.wasm"))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// Ask a yes/no question on the terminal; false when there is no terminal.
fn confirm(question: &str) -> Result<bool> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "j" | "ja"))
}

/// The mod folders of a pack (`mods/<id>/`) or the single mod at `root`.
fn mod_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if root.join("mods").is_dir() {
        for e in std::fs::read_dir(root.join("mods"))? {
            let p = e?.path();
            if p.join("mod.toml").is_file() {
                dirs.push(p);
            }
        }
    } else if root.join("mod.toml").is_file() {
        dirs.push(root.to_path_buf());
    }
    Ok(dirs)
}

/// The ids of the mods in a pack or mod (zip or directory), without installing anything.
fn ids_in(data: &Path, path: &Path) -> Result<Vec<String>> {
    let scan = data.join("tmp-scan");
    if scan.exists() {
        std::fs::remove_dir_all(&scan)?;
    }
    let src = if path.is_dir() {
        path.to_path_buf()
    } else {
        std::fs::create_dir_all(&scan)?;
        let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        package::unzip(f, &scan).with_context(|| format!("extracting {}", path.display()))?;
        scan.clone()
    };
    let ids = mod_dirs(&find_root(&src)?)?
        .iter()
        .map(|d| Package::load(d).map(|p| p.manifest.info.id))
        .collect::<Result<_>>();
    if scan.exists() {
        std::fs::remove_dir_all(&scan)?;
    }
    ids
}

/// Install a pack or a single mod from a zip or a directory; returns the ids it installed.
fn install(data: &Path, path: &Path, yes: bool, given: &[String]) -> Result<Vec<String>> {
    let tmp = data.join("tmp");
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let src = if path.is_dir() {
        path.to_path_buf()
    } else {
        let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        package::unzip(f, &tmp).with_context(|| format!("extracting {}", path.display()))?;
        tmp.clone()
    };
    // A pack: modpack.toml + mods/<id>/. Else one mod, at the root or in a single folder.
    let root = find_root(&src)?;
    let mut incoming = if root.join("modpack.toml").is_file() {
        ModPack::load(&root.join("modpack.toml"))?
    } else {
        ModPack::default()
    };
    let dirs = mod_dirs(&root)?;
    if dirs.is_empty() {
        bail!("{}: no mods found (expected mod.toml or mods/<id>/mod.toml)", path.display());
    }

    // Refuse before changing anything: mods for another openlina, missing requirements.
    let manifests: Vec<_> = dirs.iter().map(|d| Package::load(d).map(|p| p.manifest)).collect::<Result<_>>()?;
    let kits = openlina::kit_problems(&manifests, openlina::Host::Helper);
    if !kits.is_empty() {
        bail!("{}: can't be installed:\n  {}", path.display(), kits.join("\n  "));
    }
    let state_before = load_state(data)?;
    let incoming_ids: Vec<String> =
        dirs.iter().map(|d| Package::load(d).map(|p| p.manifest.info.id)).collect::<Result<_>>()?;
    for d in &dirs {
        let p = Package::load(d)?;
        for r in &p.manifest.info.requires {
            if !incoming_ids.contains(r) && !given.contains(r) && !state_before.mods.iter().any(|e| &e.id == r) {
                bail!(
                    "`{}` requires `{r}`, which is neither installed nor in {}. Install a pack that \
                     contains it (e.g. `lina pack {r} {} --bundle my-pack`), or install `{r}` first.",
                    p.manifest.info.id,
                    path.display(),
                    p.manifest.info.id
                );
            }
        }
    }
    // Mods the site has not reviewed: say so and ask.
    let unreviewed: Vec<String> = incoming
        .mods
        .iter()
        .filter(|e| incoming_ids.contains(&e.id) && matches!(e.status.as_deref(), Some("unreviewed" | "rejected")))
        .map(|e| format!("{} {} ({})", e.id, e.version.as_deref().unwrap_or("?"), e.status.as_deref().unwrap_or("?")))
        .collect();
    if !unreviewed.is_empty() && !yes {
        println!("Not reviewed by the site's maintainers yet:\n  {}", unreviewed.join("\n  "));
        println!("Mods are code that runs in your game. Only install them if you trust their authors.");
        if !confirm("Install anyway?")? {
            bail!("not installed (to install unreviewed mods without asking: `openlina install --yes …`)");
        }
    }
    let mut state = state_before;
    for dir in dirs {
        let pkg = Package::load(&dir)?;
        let id = pkg.manifest.info.id.clone();
        let dst = data.join("mods").join(&id);
        if dst.exists() {
            std::fs::remove_dir_all(&dst)?;
        }
        openlina::copy_dir(&dir, &dst)?;
        let entry = incoming.mods.iter().position(|e| e.id == id).map(|i| incoming.mods.remove(i));
        match state.mods.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                if let Some(new) = entry {
                    e.options = new.options;
                    e.status = new.status;
                }
            }
            None => state.mods.push(entry.unwrap_or(PackEntry { id: id.clone(), ..Default::default() })),
        }
        // The state records the installed version, whatever the pack said.
        if let Some(e) = state.mods.iter_mut().find(|e| e.id == id) {
            e.version = Some(pkg.manifest.info.version.clone());
        }
        println!("installed {id} {}", pkg.manifest.info.version);
    }
    std::fs::create_dir_all(data)?;
    state.save(&data.join("modpack.toml"))?;
    std::fs::remove_dir_all(&tmp).ok();
    Ok(incoming_ids)
}

fn find_root(dir: &Path) -> Result<PathBuf> {
    if dir.join("modpack.toml").is_file() || dir.join("mod.toml").is_file() || dir.join("mods").is_dir() {
        return Ok(dir.to_path_buf());
    }
    let subdirs: Vec<_> =
        std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    match subdirs.as_slice() {
        [one] => Ok(one.clone()),
        _ => Ok(dir.to_path_buf()),
    }
}

/// Keep a copy of the helper in the data dir, so the Steam launch option stays valid.
fn install_self(data: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;
    let dst = data.join("bin").join(if cfg!(windows) { "openlina.exe" } else { "openlina" });
    if std::fs::canonicalize(&exe).ok() == std::fs::canonicalize(&dst).ok() {
        return Ok(());
    }
    std::fs::create_dir_all(dst.parent().unwrap())?;
    let tmp = dst.with_extension("new");
    std::fs::copy(&exe, &tmp)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(())
}

fn print_launch_option(data: &Path) {
    let exe = data.join("bin").join("openlina");
    println!("\nTo play modded from Steam, set Mosa Lina > Properties > Launch Options to:\n");
    println!("    \"{}\" steam %command%\n", exe.display());
    println!("Clear the launch options to play vanilla again.");
}

fn build(data: &Path, game: &Path) -> Result<()> {
    let pack = load_state(data)?;
    let packages: Vec<Package> =
        pack.mods.iter().map(|e| Package::load(&data.join("mods").join(&e.id))).collect::<Result<_>>()?;
    let input = game::read_bytecode(game)?;
    println!("building {} mod(s)", packages.len());
    let built = openlina::build(input, &packages, &pack, openlina::Host::Helper, &mut |l| println!("{l}"))?;
    // Mods that make the game reach outside the game need the player's explicit allowance.
    let trust = load_trust(data)?;
    let mut refused = String::new();
    for (id, findings) in &built.caps {
        let list: String = findings.iter().map(|f| format!("      {f}\n")).collect();
        if trust.get(id).and_then(|v| v.as_str()) == Some(patch_hash(data, id)?.as_str()) {
            println!("  note: {id} is allowed to reach outside the game:\n{list}");
        } else {
            refused.push_str(&format!("  {id}:\n{list}"));
        }
    }
    if !refused.is_empty() {
        bail!(
            "not built: these mods make the game reach outside the game (files, programs, network, Steam, …):\n{refused}\n\
             Game mods don't normally need this. Such a mod could read, change or delete your files or run programs.\n\
             If you trust it (you know its author, or someone reviewed its code): `openlina allow <id>`.\n\
             Otherwise: `openlina uninstall <id>`. Until then the last working build stays in place."
        );
    }
    let bytes = built.bytes;
    let mut assets = Vec::new();
    for p in &packages {
        assets.extend(p.assets()?);
    }
    for note in overlay::create(game, &data.join("game"), &bytes, &assets)? {
        println!("  note: {note}");
    }
    println!("ready: {} ({} asset file(s))", data.join("game").display(), assets.len());
    Ok(())
}

/// `openlina report`: a zip with everything needed to look into a problem.
fn report(data: &Path, game: Option<PathBuf>, out: Option<PathBuf>) -> Result<()> {
    use std::io::Write;
    let date = std::process::Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "now".into());
    let out = out.unwrap_or_else(|| PathBuf::from(format!("openlina-report-{date}.zip")));
    let mut info = format!("openlina {}\ndate {date}\n", env!("CARGO_PKG_VERSION"));
    if let Ok(o) = std::process::Command::new("uname").arg("-a").output() {
        info.push_str(&format!("system {}", String::from_utf8_lossy(&o.stdout)));
    }
    if let Some(g) = &game {
        match std::fs::read(g.join("hlboot.dat")) {
            Ok(b) => {
                let h = game::sha256(&b);
                let build = game::known_version(&h).unwrap_or("unknown");
                info.push_str(&format!("game {} (hlboot.dat sha256 {h}, Steam build {build})\n", g.display()));
            }
            Err(e) => info.push_str(&format!("game {}: {e}\n", g.display())),
        }
    }
    // Installed mods with versions and resolved options.
    let mut mods = String::new();
    if let Ok(pack) = load_state(data) {
        for e in &pack.mods {
            match Package::load(&data.join("mods").join(&e.id)) {
                Ok(p) => {
                    mods.push_str(&format!("{} {}  {}\n", e.id, p.manifest.info.version, p.manifest.info.name));
                    if let Ok(opts) = p.manifest.resolve_options(&e.options) {
                        for (k, v) in opts {
                            mods.push_str(&format!("    {k} = {v}\n"));
                        }
                    }
                }
                Err(err) => mods.push_str(&format!("{}: {err:#}\n", e.id)),
            }
        }
    }
    let file = std::fs::File::create(&out).with_context(|| format!("creating {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(name, opts)?;
        zip.write_all(bytes)?;
        Ok(())
    };
    add("info.txt", info.as_bytes())?;
    add("mods.txt", mods.as_bytes())?;
    let mut included = vec!["info.txt", "mods.txt"];
    for (name, path) in [
        ("modpack.toml", data.join("modpack.toml")),
        ("trust.toml", data.join("trust.toml")),
        ("last-run.log", data.join("last-run.log")),
        ("last-run.log.prev", data.join("last-run.log.prev")),
    ] {
        if let Ok(b) = std::fs::read(&path) {
            add(name, &b)?;
            included.push(name);
        }
    }
    if let Some(g) = &game {
        if let Ok(b) = std::fs::read(g.join("crash_stackdump.txt")) {
            add("crash_stackdump.txt", &b)?;
            included.push("crash_stackdump.txt");
        }
    }
    zip.finish()?;
    println!("wrote {} ({})", out.display(), included.join(", "));
    if !included.contains(&"last-run.log") {
        println!("no log of a modded run yet: play once with the Steam launch option (or `openlina run`), then report");
    }
    println!(
        "It holds file paths of this computer (your user name) and the game's output; nothing else is sent anywhere."
    );
    Ok(())
}
