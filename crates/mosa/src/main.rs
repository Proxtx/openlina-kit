use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod build;
mod dump;
mod game;
mod inspect;

#[derive(Parser)]
#[command(name = "mosa", about = "Mosa Lina modding toolkit", version)]
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
        #[arg(long, default_value = game::ORIG)]
        input: PathBuf,
        #[arg(long, default_value = "work/dump")]
        out: PathBuf,
    },
    /// Show one function: disassembly, and decompiled pseudo-Haxe with --hx.
    /// FUNC is a findex (`3767`) or `Class.method` (`fish.game.evsheet.EvSheet_gameplay.update`).
    Fn {
        func: String,
        /// Also print decompiled pseudo-Haxe.
        #[arg(long)]
        hx: bool,
        /// Only print ops in this range, e.g. `15300..15400`.
        #[arg(long)]
        ops: Option<String>,
        /// Bytecode file to read (e.g. work/hlboot.modded.dat to inspect a patch).
        #[arg(long, default_value = game::ORIG)]
        input: PathBuf,
    },
    /// List every function that calls FUNC directly or makes a closure of it.
    Callers {
        func: String,
        #[arg(long, default_value = game::ORIG)]
        input: PathBuf,
    },
    /// Search the string constant pool and show which functions use matching strings.
    Strings {
        pattern: String,
        #[arg(long, default_value = game::ORIG)]
        input: PathBuf,
    },
    /// List available mods and their options.
    Mods,
    /// Apply the mods enabled in the config to the pristine bytecode.
    Build {
        #[arg(long, default_value = "mods.toml")]
        config: PathBuf,
        /// Apply only these mods (ignores `enabled` in the config).
        #[arg(long = "mod")]
        only: Vec<String>,
        #[arg(long, default_value = game::MODDED)]
        out: PathBuf,
    },
    /// Launch the game through the HashLink JIT with a bytecode file (default: the modded one).
    Run {
        #[arg(long, default_value = game::MODDED)]
        bytecode: PathBuf,
        /// Build first (with mods.toml).
        #[arg(long)]
        build: bool,
        /// Kill the game after this many seconds (useful for automated smoke tests).
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Self-test: roundtrip the pristine bytecode and run the validator on every function.
    Check {
        #[arg(long, default_value = game::ORIG)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let game_dir = || game::game_dir(cli.game_dir.clone());
    match cli.cmd {
        Cmd::Setup => game::setup(&game_dir()?),
        Cmd::Dump { input, out } => dump::run(&input, &out),
        Cmd::Fn { func, hx, ops, input } => inspect::show_fn(&input, &func, hx, ops.as_deref()),
        Cmd::Callers { func, input } => inspect::callers(&input, &func),
        Cmd::Strings { pattern, input } => inspect::strings(&input, &pattern),
        Cmd::Mods => build::list_mods(),
        Cmd::Build { config, only, out } => build::build(&config, &only, &out),
        Cmd::Run { bytecode, build, timeout } => {
            if build {
                build::build(&PathBuf::from("mods.toml"), &[], &bytecode)?;
            }
            game::run(&game_dir()?, &bytecode, timeout)
        }
        Cmd::Check { input } => build::check(&input),
    }
}
