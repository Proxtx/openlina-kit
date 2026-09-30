use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod build;
mod doctor;
mod dump;
mod hub;
mod inspect;
mod scaffold;
mod scenario;
mod sprite;

/// Pristine bytecode copied from the game by `lina setup`.
pub const ORIG: &str = "work/hlboot.orig.dat";
/// Output of `lina build`.
pub const MODDED: &str = "work/hlboot.modded.dat";
/// Overlay game directory used by `lina run`.
pub const OVERLAY: &str = "work/game";

#[derive(Parser)]
#[command(name = "lina", about = "OpenLina mod development CLI", version)]
struct Cli {
    /// Game install directory (default: $MOSA_GAME_DIR, then the default Steam library).
    #[arg(long, global = true)]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Copy the pristine game bytecode to work/hlboot.orig.dat and check the game version.
    Setup,
    /// Decompile the whole game into work/dump (pseudo-Haxe, exact disassembly, index files).
    Dump {
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
        #[arg(long, default_value = "work/dump")]
        out: PathBuf,
    },
    /// Show one function: disassembly, and decompiled pseudo-Haxe with --hx.
    /// FUNC is a findex (`3767`) or `Class.method` (`fish.game.evsheet.EvSheet_gameplay.update`).
    Fn {
        func: String,
        #[arg(long)]
        hx: bool,
        /// Only print ops in this range, e.g. `15300..15400`.
        #[arg(long)]
        ops: Option<String>,
        /// Bytecode file to read (e.g. work/hlboot.modded.dat to inspect a patch).
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// List every function that calls FUNC directly or makes a closure of it.
    Callers {
        func: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// Search the string constant pool and show which functions use matching strings.
    Strings {
        pattern: String,
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
    /// List the mods in mods/ and their options.
    Mods,
    /// List the hooks the `core` mod provides (subscribe with `openlina_sdk::hooks`).
    Hooks,
    /// Create a new mod crate from a template: `lina new items portal-gun`.
    New { section: String, id: String },
    /// Build mods and apply them to the pristine bytecode (+ overlay in work/game).
    Build {
        /// Modpack selecting mods and options.
        #[arg(long, default_value = "modpack.toml")]
        pack: PathBuf,
        /// Build only these mods (their `requires` are added), with default options.
        #[arg(long = "mod")]
        only: Vec<String>,
        /// Run the wasm builds (what players get) instead of native ones.
        #[arg(long)]
        wasm: bool,
        #[arg(long, default_value = MODDED)]
        out: PathBuf,
    },
    /// Launch the game from the work/game overlay (build first with --build).
    Run {
        #[arg(long)]
        build: bool,
        #[arg(long, default_value = "modpack.toml")]
        pack: PathBuf,
        /// Kill the game after this many seconds (useful for automated smoke tests).
        #[arg(long)]
        timeout: Option<u64>,
        /// No window: SDL's offscreen driver (EGL). Combine with the `autostart` mod to reach a
        /// level without input.
        #[arg(long)]
        headless: bool,
    },
    /// Build wasm packages into dist/: one `<id>-<version>.zip` per mod, and with --bundle a
    /// pack zip (helper + modpack.toml + mods) ready for players.
    Pack {
        /// Mods to package (default: all non-dev mods).
        ids: Vec<String>,
        /// Also write dist/<NAME>.zip containing the helper and these mods.
        #[arg(long)]
        bundle: Option<String>,
        /// Take the mods and their options from a modpack (e.g. work/pull/<pack>/modpack.toml);
        /// the bundle keeps the options.
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },
    /// Run test scenarios headless (default: every mods/*/tests/*.toml) and check their logs.
    Test {
        /// Scenario files (default: all).
        files: Vec<PathBuf>,
        /// Only the scenarios of this mod.
        #[arg(long = "mod")]
        only: Option<String>,
        /// Use the wasm builds (what players run).
        #[arg(long)]
        wasm: bool,
    },
    /// Record a scenario's [gif] section: capture frames headless, write the gif (default: the
    /// scenario's gif.out, relative to its mod directory).
    Gif {
        scenario: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Render pixel art from a text sprite file to PNG (see `lina sprite --palette`).
    Sprite {
        file: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Enlarge every pixel (previews).
        #[arg(long, default_value_t = 1)]
        scale: u32,
        /// Print the built-in game palette.
        #[arg(long)]
        palette: bool,
    },
    /// Save an OpenLina site and your upload token (checked against the site) for pull/publish.
    Login {
        /// The site, e.g. `http://127.0.0.1:8080`.
        site: String,
        /// Upload token (`olt_…`); read from $OPENLINA_TOKEN or stdin when omitted.
        token: Option<String>,
    },
    /// Download a pack from an OpenLina site: packages, the source of mods you don't have
    /// (into mods/<id>/), and work/pull/<pack>/ with modpack.toml and REQUESTS.md (change requests).
    Pull {
        /// Pack link (`https://site/api/packs/<id>`) or pack id (on the site you logged in to).
        pack: String,
        /// Where to put the source of pulled mods.
        #[arg(long, default_value = "mods")]
        mods_dir: PathBuf,
        /// Replace local mods of the same id with the pack's version.
        #[arg(long)]
        force: bool,
    },
    /// Upload a mod to the site you logged in to: runs its scenarios (wasm), packages it with its
    /// source, shows what would be uploaded. Uploads only with --yes (ask the user first).
    Publish {
        id: String,
        /// Really upload.
        #[arg(long)]
        yes: bool,
        /// Skip the scenarios.
        #[arg(long)]
        no_test: bool,
    },
    /// Check this machine: toolchain (with or without nix), wasm target, ImageMagick, the game, setup.
    Doctor,
    /// Self-test: roundtrip the pristine bytecode and run the validator on every function.
    Check {
        #[arg(long, default_value = ORIG)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let game_dir = || openlina::game::game_dir(cli.game_dir.clone());
    match cli.cmd {
        Cmd::Setup => build::setup(&game_dir()?),
        Cmd::Dump { input, out } => dump::run(&input, &out),
        Cmd::Fn { func, hx, ops, input } => inspect::show_fn(&input, &func, hx, ops.as_deref()),
        Cmd::Callers { func, input } => inspect::callers(&input, &func),
        Cmd::Strings { pattern, input } => inspect::strings(&input, &pattern),
        Cmd::Mods => build::list_mods(),
        Cmd::Hooks => {
            for h in openlina_sdk::hooks::CORE_HOOKS {
                println!("{}{} -> {}\n    {}\n", h.name, h.args, h.returns, h.doc.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            Ok(())
        }
        Cmd::New { section, id } => scaffold::new_mod(&section, &id),
        Cmd::Build { pack, only, wasm, out } => build::build(&game_dir()?, &pack, &only, wasm, &out),
        Cmd::Run { build, pack, timeout, headless } => {
            let game = game_dir()?;
            if build {
                build::build(&game, &pack, &[], false, &PathBuf::from(MODDED))?;
            }
            build::run(&game, timeout, headless)
        }
        Cmd::Pack { ids, bundle, from, out } => {
            let from = from.map(|p| openlina_sdk::manifest::ModPack::load(&p)).transpose()?;
            let mut ids = ids;
            if let Some(f) = &from {
                anyhow::ensure!(ids.is_empty(), "give either mod ids or --from, not both");
                ids = f.mods.iter().map(|e| e.id.clone()).collect();
            }
            build::pack(&ids, bundle.as_deref(), from.as_ref(), &out)
        }
        Cmd::Test { files, only, wasm } => {
            let files = scenario::find(&files, only.as_deref())?;
            scenario::test(&game_dir()?, &files, wasm)
        }
        Cmd::Gif { scenario, out } => scenario::gif(&game_dir()?, &scenario, out.as_deref()),
        Cmd::Sprite { file, out, scale, palette } => {
            if palette {
                sprite::print_palette();
                return Ok(());
            }
            let file = file.ok_or_else(|| anyhow::anyhow!("give a sprite file (or --palette)"))?;
            let out = out.unwrap_or_else(|| file.with_extension("png"));
            sprite::render(&file, &out, scale)
        }
        Cmd::Check { input } => build::check(&input),
        Cmd::Doctor => doctor::doctor(cli.game_dir.clone()),
        Cmd::Login { site, token } => hub::login(&site, token),
        Cmd::Pull { pack, mods_dir, force } => hub::pull(&pack, &mods_dir, force),
        Cmd::Publish { id, yes, no_test } => hub::publish(game_dir, &id, yes, !no_test),
    }
}
