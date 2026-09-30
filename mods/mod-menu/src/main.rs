//! `mod-menu`: an OPENLINA MODS entry in the pause menu, listing the installed mods.
//!
//! The pause menu (`fish.system.PauseMenu`, a `bib.Menu`) is built in its constructor with
//! `addItem(text)` calls (CONTINUE, REROLL, …, MODDING MENU, …, QUIT) and submenus added with
//! `addSub(parentText, text)`: sub items live in `items` too, hidden until their parent is
//! selected. This mod runs right after `this.modding = addItem("MODDING MENU")` and adds:
//!
//! ```text
//! OPENLINA MODS (3)          <- addItem; selecting it opens the submenu
//!    SCREEN WRAP 0.3.0       <- addSub(title, …), one per mod (dev mods are left out)
//!       COINS: FALSE         <- its option values (`show_options`)
//!    BACK
//! ```
//!
//! Every sub item gets `exec = back` like the game's own back buttons
//! (`MenuExtension.toBackButton`), because `Menu.exec` calls `item.exec` unconditionally.
//! The mod list comes from the host at build time (`openlina_sdk::runner::pack_info`).

use anyhow::Result;
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::{add_reg, call, find_field_access, insert_ops, Incoming};
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::manifest::Section;
use openlina_sdk::{edit, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

/// Menu text: the menu font only has capitals.
fn menu_text(s: &str) -> String {
    s.to_uppercase().replace('_', " ")
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let show_options = cfg.bool("show_options", true)?;
    let trace = cfg.bool("trace", false)?;
    let pack = openlina_sdk::runner::pack_info()?;
    let mods: Vec<_> = pack.mods.iter().filter(|m| m.section != Section::Dev).collect();

    let ctor = code.method("fish.system.PauseMenu", "__constructor__")?;
    let add_item = code.method("fish.system.PauseMenu", "addItem")?;
    let add_sub = code.method("bib.Menu", "addSub")?;
    let menu_t = code.class("fish.system.PauseMenu")?;
    let void = code.ty_void();

    let title = format!("OPENLINA MODS ({})", mods.len());
    let mut lines = Vec::new();
    for m in &mods {
        lines.push(menu_text(&format!("{} {}", m.name, m.version)));
        if show_options {
            for (k, v) in &m.options {
                if k == "trace" {
                    continue;
                }
                let v = match v {
                    openlina_sdk::toml::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                lines.push(menu_text(&format!("   {k}: {v}")));
            }
        }
    }
    lines.push("BACK".into());

    // build(menu): item = menu.addItem(title); for line: sub = menu.addSub(title, line); sub.exec = sub.back
    let mut f = FnBuilder::new(code, "mod-menu/build", &[menu_t], void);
    let menu = f.arg(0);
    let t = f.string_obj(&title)?;
    f.call_new(add_item, &[menu, t])?;
    for line in &lines {
        let l = f.string_obj(line)?;
        let item = f.call_new(add_sub, &[menu, t, l])?;
        let back = f.get_new(item, "back")?;
        f.set(item, "exec", back)?;
    }
    if trace {
        f.print(&[Print::Str(&format!("[mod-menu] added `{title}` with {} lines", lines.len()))])?;
    }
    f.ret_void();
    let build = f.finish()?;

    // In the constructor, right after `this.modding = addItem("MODDING MENU")`.
    let fun = code.func(ctor)?.clone();
    let at = edit::expect_one(find_field_access(code, &fun, "modding", true), "PauseMenu constructor sets `modding`")?;
    let f = code.func_mut(ctor)?;
    let r = add_reg(f, void);
    insert_ops(f, at + 1, vec![call(r, build, &[Reg(0)])], Incoming::ToOriginal);
    Ok(())
}
