//! `lina new <section> <id>`: create a mod crate from a template.

use std::path::Path;

use anyhow::{bail, ensure, Result};
use openlina_sdk::manifest::ModManifest;

const MAIN_TICK: &str = r#"//! `{id}`: TODO one-line summary.
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

    // Example: run code at the start of every gameplay tick (`lina hooks` lists all hooks).
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

const MAIN_ITEM: &str = r#"//! `{id}`: TODO one-line summary of the item.
//!
//! TODO: what firing it does, what it leaves alone. Keep this doc comment accurate; it is the
//! mod's design document.
//!
//! The item is registered with `openlina_sdk::items` (pool entry, the game's item object for the
//! tool selection and editor, HUD and large icon, label); its behavior is an `item_use` handler.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::items::{self, Aim, Item};
use openlina_sdk::{hooks, Code, ModConfig};

const ITEM: &str = "{id}";

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;
    items::register(
        code,
        &Item {
            name: ITEM,
            label: "{name}",
            ammo: cfg.i64("ammo", 3)? as i32,
            aim: Aim::Long,
            second_layer: false,
            icon: "images/openlina/{id}.png",
            icon_size: (12.0, 12.0),
            big_icon: Some(("images/openlina/{id}-big.png", (24.0, 24.0))),
        },
    )?;

    // item_use(slot, sheet, player, crosshair) -> Bool: return true when the fired item is ours.
    let mut f = hooks::handler(code, "item_use", "{id}/use")?;
    let slot = f.arg(0);
    let not_mine = f.label();
    items::is_item(&mut f, slot, ITEM, not_mine)?;
    if trace {
        f.print(&[Print::Str("[{id}] fired")])?;
    }
    // TODO: what the item does (items::crosshair_pos gives the aim point).
    let yes = f.reg_bool();
    f.bool(yes, true);
    f.ret(yes);
    f.place(not_mine);
    let no = f.reg_bool();
    f.bool(no, false);
    f.ret(no);
    let handler = f.finish()?;
    hooks::subscribe(code, "item_use", handler)
}
"#;

const MAIN_MODIFIER: &str = r#"//! `{id}`: a modifier. TODO: what changes in levels that roll it.
//!
//! TODO: what vanilla does, what the modifier changes, what it leaves alone. Keep this doc comment
//! accurate; it is the mod's design document.
//!
//! Registered with `openlina_sdk::modifiers` (key `{id}`, HUD icon `assets/images/openlina/{id}.png`
//! from `art/modifier.toml`), rolled like the vanilla modifiers; with `always` it applies in every
//! level instead.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::modifiers::{self, Modifier};
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;
    let always = cfg.bool("always", false)?;
    let modifier = if always {
        None
    } else {
        Some(modifiers::register(
            code,
            &Modifier { key: "{id}", icon: "images/openlina/{id}.png", size: (16.0, 16.0), in_dx: true },
        )?)
    };

    // Every gameplay tick while the modifier is active (`lina hooks` lists other hooks).
    let mut f = hooks::handler(code, "tick", "{id}/tick")?;
    let layout = f.arg(1);
    let end = f.label();
    if let Some(id) = modifier {
        let active = modifiers::is_active(&mut f, id)?;
        f.jfalse(active, end);
    }
    // TODO: the rule.
    if trace {
        let tick = f.get_new(layout, "currentTick")?;
        let one = f.const_i32(1);
        f.jne(tick, one, end);
        f.print(&[Print::Str("[{id}] active")])?;
    }
    f.place(end);
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
{extra_options}
[options.trace]
type = "bool"
default = false
description = "Print what the mod does to stdout."
"#;

const ITEM_OPTIONS: &str = r#"
[options.ammo]
type = "int"
default = 3
description = "Shots per slot."
"#;

const MODIFIER_OPTIONS: &str = r#"
[options.always]
type = "bool"
default = false
description = "Apply in every level instead of as a rolled modifier."
"#;

const SMOKE_TEST: &str = r#"# Run with `lina test --mod {id}`; format: docs/testing.md. Make tests that can fail: assert what
# the mod changes (e.g. positions with the fixture trace-positions), and that it does nothing when off.
name = "{id}: builds, loads a level and runs for 2 seconds"
mods = ["{id}"]
timeout = 60

[options.{id}]
trace = true

[harness]
level = "greendemo 1"
end_tick = 240

[[expect]]
contains = "[harness] level tick 1: greendemo 1"

[[expect]]
contains = "[harness] end at tick 240"

# A showcase gif: add
#   [gif]
#   capture = "1-240/4"
#   out = "media/{id}.gif"
# and run `lina gif mods/{id}/tests/smoke.toml`.
"#;

const SMOKE_ITEM: &str = r#"# Run with `lina test --mod {id}`; format: docs/testing.md. Make tests that can fail: assert what
# the item does (e.g. positions with the fixture trace-positions), not just that it fired.
name = "{id}: the item is rolled and fires"
mods = ["{id}"]
timeout = 60

[options.{id}]
trace = true

[harness]
level = "greendemo 1"
items = ["{id}"]
inputs = ["60:shoot"]
end_tick = 120

[[expect]]
contains = "[harness] slot 0: {id} ammo 3"

[[expect]]
contains = "[{id}] fired"
"#;

const SMOKE_MODIFIER: &str = r#"# Run with `lina test --mod {id}`; format: docs/testing.md. Also test that nothing changes with
# `modifier = 0` (the modifier not rolled), and `roll_until_modifier = "{id}"` (the game can draw it).
name = "{id}: with the modifier forced, it is active"
mods = ["{id}"]
timeout = 60

[options.{id}]
trace = true

[harness]
level = "greendemo 1"
modifier_key = "{id}"
end_tick = 120

[[expect]]
contains = "[{id}] active"
"#;

/// Placeholder pixel art, rendered on creation (edit the .toml, then rerun `lina sprite`).
const ART_ICON: &str = r##"# Item/website icon (12x12). Edit, then:
#   lina sprite mods/{id}/art/icon.toml --out mods/{id}/assets/images/openlina/{id}.png
#   lina sprite mods/{id}/art/icon.toml --scale 2 --out mods/{id}/assets/images/openlina/{id}-big.png
#   lina sprite mods/{id}/art/icon.toml --out mods/{id}/media/icon.png
[palette]
w = "#e8e8e8"
o = "#e0913a"

[[frame]]
pixels = """
............
....oooo....
...oo..oo...
.......oo...
......oo....
.....oo.....
.....oo.....
............
.....oo.....
.....oo.....
............
............
"""
"##;

const ART_MODIFIER: &str = r##"# HUD modifier icon (16x16, vanilla modifier colors). Edit, then:
#   lina sprite mods/{id}/art/modifier.toml --out mods/{id}/assets/images/openlina/{id}.png
[palette]
t = "#83a4a5"
h = "#abc3c3"

[[frame]]
pixels = """
................
......tttt......
.....tt..tt.....
.........tt.....
........tt......
.......tt.......
.......tt.......
................
.......tt.......
.......tt.......
................
hhhhhhhhhhhhhhhh
................
................
................
................
"""
"##;

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
    let (main, smoke, options) = match section {
        "items" => (MAIN_ITEM, SMOKE_ITEM, ITEM_OPTIONS),
        "modifiers" => (MAIN_MODIFIER, SMOKE_MODIFIER, MODIFIER_OPTIONS),
        _ => (MAIN_TICK, SMOKE_TEST, ""),
    };
    let fill = |t: &str| {
        t.replace("{extra_options}", options).replace("{id}", id).replace("{name}", &name).replace("{section}", section)
    };
    ModManifest::parse(&fill(MANIFEST))?; // validates the id
    for sub in ["src", "media", "tests", "art", "assets/images/openlina"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    std::fs::write(dir.join("tests/smoke.toml"), fill(smoke))?;
    std::fs::write(dir.join("Cargo.toml"), fill(CARGO))?;
    std::fs::write(dir.join("mod.toml"), fill(MANIFEST))?;
    std::fs::write(dir.join("src/main.rs"), fill(main))?;
    // Placeholder icons, so the mod runs right away.
    let icons = dir.join("assets/images/openlina");
    match section {
        "items" => {
            let art = dir.join("art/icon.toml");
            std::fs::write(&art, fill(ART_ICON))?;
            crate::sprite::render(&art, &icons.join(format!("{id}.png")), 1)?;
            crate::sprite::render(&art, &icons.join(format!("{id}-big.png")), 2)?;
            crate::sprite::render(&art, &dir.join("media/icon.png"), 1)?;
        }
        "modifiers" => {
            let art = dir.join("art/modifier.toml");
            std::fs::write(&art, fill(ART_MODIFIER))?;
            crate::sprite::render(&art, &icons.join(format!("{id}.png")), 1)?;
            crate::sprite::render(&art, &dir.join("media/icon.png"), 1)?;
        }
        _ => {
            std::fs::remove_dir(&icons)?;
            std::fs::remove_dir(dir.join("assets/images"))?;
            std::fs::remove_dir(dir.join("assets"))?;
        }
    }
    println!(
        "created {} from the {section} template (TODOs in src/main.rs and mod.toml; run `lina test --mod {id}`)",
        dir.display()
    );
    Ok(())
}
