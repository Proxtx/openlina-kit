//! The entry point of every mod.
//!
//! A mod is a small program with a fixed interface, so the same crate runs natively during
//! development and as `wasm32-wasip1` (sandboxed, no file or network access) when installed:
//!
//! - stdin: the game bytecode (`hlboot.dat` after the previous mods)
//! - `OPENLINA_OPTIONS`: the mod's options as TOML (defaults from `mod.toml`, overridden by the
//!   user's modpack); `OPENLINA_MOD`: the mod id, for messages
//! - stdout: the patched bytecode
//! - stderr + exit code 1: errors (e.g. an anchor that no longer matches the game)
//!
//! ```ignore
//! fn main() {
//!     openlina_sdk::run_mod(|code, cfg| {
//!         let speed = cfg.f64("speed", 2.0)?;
//!         // ... patch `code` ...
//!         Ok(())
//!     })
//! }
//! ```

use std::io::{Read, Write};

use anyhow::{Context, Result};

use crate::{validate, Code, ModConfig};

/// Run a mod's patch function with the standard interface. Never returns.
pub fn run_mod(apply: impl FnOnce(&mut Code, &ModConfig) -> Result<()>) -> ! {
    let id = std::env::var("OPENLINA_MOD").unwrap_or_else(|_| "mod".into());
    match run(apply) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{id}: error: {e:#}");
            std::process::exit(1)
        }
    }
}

fn run(apply: impl FnOnce(&mut Code, &ModConfig) -> Result<()>) -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).context("reading bytecode from stdin")?;
    anyhow::ensure!(!input.is_empty(), "no bytecode on stdin");
    let cfg = ModConfig::from_toml(&std::env::var("OPENLINA_OPTIONS").unwrap_or_default())?;
    let mut code = Code::from_bytes(&input)?;
    apply(&mut code, &cfg)?;
    validate::check_touched(&code)?;
    let out = code.to_bytes()?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&out)?;
    stdout.flush()?;
    Ok(())
}
