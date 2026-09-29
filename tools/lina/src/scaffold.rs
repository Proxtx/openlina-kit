//! `lina new <section> <id>`: create a mod crate from a template.

use std::path::Path;

use anyhow::{bail, ensure, Result};
use openlina_sdk::manifest::ModManifest;

const MAIN: &str = r#"//! `{id}`: TODO one-line summary.
//!
//! TODO: what vanilla does, and what this mod changes. Keep this doc comment accurate; it is
//! the mod's design document.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;

    // Example: run code at the start of every gameplay tick (see `lina hooks`, AGENTS.md).
    let mut f = hooks::handler(code, "tick", "{id}/tick")?;
    let layout = f.arg(1);
    if trace {
        let tick = f.get_new(layout, "currentTick")?;
        f.print(&[Print::Str("[{id}] tick "), Print::Val(tick)])?;
    }
    f.ret_void();
    let handler = f.finish()?;
    hooks::subscribe(code, "tick", handler)
}
"#;

const MANIFEST: &str = r#"[mod]
id = "{id}"
name = "{name}"
version = "0.1.0"
section = "{section}"
description = "TODO"
authors = []
game_builds = ["22056877"]
requires = ["core"]

[options.trace]
type = "bool"
default = false
description = "Print what the mod does to stdout."
"#;

const CARGO: &str = r#"[package]
name = "openlina-mod-{id}"
edition.workspace = true
version = "0.1.0"
publish = false

[[bin]]
name = "{id}"
path = "src/main.rs"

[dependencies]
openlina-sdk.workspace = true
anyhow.workspace = true
"#;

pub fn new_mod(section: &str, id: &str) -> Result<()> {
    let sections = ["items", "modifiers", "levels", "general", "dev"];
    ensure!(sections.contains(&section), "section must be one of {}", sections.join(", "));
    let dir = Path::new("mods").join(id);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    let name: String = id
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let fill = |t: &str| t.replace("{id}", id).replace("{name}", &name).replace("{section}", section);
    ModManifest::parse(&fill(MANIFEST))?; // validates the id
    std::fs::create_dir_all(dir.join("src"))?;
    std::fs::create_dir_all(dir.join("media"))?;
    std::fs::write(dir.join("Cargo.toml"), fill(CARGO))?;
    std::fs::write(dir.join("mod.toml"), fill(MANIFEST))?;
    std::fs::write(dir.join("src/main.rs"), fill(MAIN))?;
    println!("created {} (add it to modpack.toml to include it in `lina build`)", dir.display());
    Ok(())
}
