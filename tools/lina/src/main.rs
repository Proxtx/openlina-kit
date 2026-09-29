use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod build;
mod dump;
mod inspect;
mod scaffold;

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
        #[arg(long, default_value = "dist")]
        out: PathBuf,
    },
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
        Cmd::New { section, id } => scaffold::new_mod(&section, &id),
        Cmd::Build { pack, only, wasm, out } => build::build(&game_dir()?, &pack, &only, wasm, &out),
        Cmd::Run { build, pack, timeout, headless } => {
            let game = game_dir()?;
            if build {
                build::build(&game, &pack, &[], false, &PathBuf::from(MODDED))?;
            }
            build::run(&game, timeout, headless)
        }
        Cmd::Pack { ids, bundle, out } => build::pack(&ids, bundle.as_deref(), &out),
        Cmd::Check { input } => build::check(&input),
    }
}
