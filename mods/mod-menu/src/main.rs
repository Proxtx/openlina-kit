//! `mod-menu`: an OPENLINA MODS entry in the pause menu, listing the installed mods.
//!
//! The pause menu (`fish.system.PauseMenu`, a `bib.Menu`) is built in its constructor with
//! `addItem(text)` calls (CONTINUE, REROLL, …, MODDING MENU, …, QUIT) and submenus added with
//! `addSub(parentText, text)` (fn@2657): it finds the parent by its text, makes it open the submenu
//! (`exec = selectSub`) and adds the item to `items`, hidden until its parent is selected. Submenus
//! nest (the game's CHANGE > SPEEDRUN > SEEDED RUN), added depth-first, each level ending with
//! BACK. This mod runs right after `this.modding = addItem("MODDING MENU")` and adds:
//!
//! ```text
//! OPENLINA MODS (12)         <- addItem; opens the mod list
//!    SCREEN WRAP 0.5.1       <- one line per mod (dev mods are left out); opens its options
//!       ALWAYS: TRUE         <- its option values (`show_options`)
//!       BACK
//!    …
//!    MORE (2/2)              <- after `PAGE - 1` lines, the rest on a next page
//!    BACK
//! ```
//!
//! An open submenu shows about 15 lines below the HUD (16 units each), so every page holds at most
//! `PAGE` lines plus BACK, and lines are cut to `MAX_LINE` characters (the screen is 600 wide). The
//! menu font only has capitals, digits and a few signs: other characters (brackets, quotes, commas)
//! would show as `?` and become spaces.
//!
//! Every leaf item gets `exec = back` like the game's own back buttons
//! (`MenuExtension.toBackButton`), because `Menu.exec` calls `item.exec` unconditionally; a line
//! that gets a submenu afterwards is switched to `selectSub` by `addSub`. Parents are found by
//! text, so parent texts are kept unique (a repeated MORE line gets a trailing space, invisible).
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

/// Lines per page before a MORE line (an open submenu fits about 15 lines with BACK).
const PAGE: usize = 10;
/// Characters per line (wider runs off the 600 wide screen).
const MAX_LINE: usize = 34;

/// Menu text: capitals, digits and the few signs the menu font has; anything else becomes a
/// space; cut to `MAX_LINE`.
fn menu_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_uppercase().chars() {
        let c = if c.is_ascii_uppercase() || c.is_ascii_digit() || " .:-()+/!%".contains(c) { c } else { ' ' };
        if c == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_end().to_string();
    if out.chars().count() > MAX_LINE {
        out.chars().take(MAX_LINE - 3).collect::<String>().trim_end().to_string() + "..."
    } else {
        out
    }
}

/// A menu line and, for a parent, the lines of its submenu.
struct Entry {
    text: String,
    children: Option<Vec<Entry>>,
}

/// One `addSub(parent, text)` call.
struct Line {
    parent: String,
    text: String,
}

/// Lay out `entries` under `parent`, depth-first as `addSub` needs them: at most `PAGE` lines a
/// page, then MORE (opening the next page), then BACK.
fn layout(parent: &str, entries: &[Entry], page: usize, pages: usize, used: &mut Vec<String>, out: &mut Vec<Line>) {
    let here = if entries.len() > PAGE { PAGE - 1 } else { entries.len() };
    for e in &entries[..here] {
        out.push(Line { parent: parent.into(), text: e.text.clone() });
        if let Some(children) = &e.children {
            layout(&e.text, children, 1, pages_of(children.len()), used, out);
        }
    }
    if here < entries.len() {
        let mut more = format!("MORE ({}/{})", page + 1, pages);
        while used.contains(&more) {
            more.push(' ');
        }
        used.push(more.clone());
        out.push(Line { parent: parent.into(), text: more.clone() });
        layout(&more, &entries[here..], page + 1, pages, used, out);
    }
    out.push(Line { parent: parent.into(), text: "BACK".into() });
}

/// How many pages `n` lines take (`layout`'s split: `PAGE - 1` lines and MORE while more remain).
fn pages_of(n: usize) -> usize {
    let mut pages = 1;
    let mut left = n;
    while left > PAGE {
        left -= PAGE - 1;
        pages += 1;
    }
    pages
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
    let mut used = vec![title.clone()];
    let mut entries = Vec::new();
    for m in &mods {
        let mut text = menu_text(&format!("{} {}", m.name, m.version));
        while used.contains(&text) {
            text.push(' ');
        }
        used.push(text.clone());
        let mut options = Vec::new();
        for (k, v) in &m.options {
            if k == "trace" {
                continue;
            }
            let v = match v {
                openlina_sdk::toml::Value::String(s) => s.clone(),
                openlina_sdk::toml::Value::Array(a) => a
                    .iter()
                    .map(|x| match x {
                        openlina_sdk::toml::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                other => other.to_string(),
            };
            options.push(Entry { text: menu_text(&format!("{k}: {v}")), children: None });
        }
        let children = show_options.then(|| {
            if options.is_empty() {
                vec![Entry { text: "NO OPTIONS".into(), children: None }]
            } else {
                options
            }
        });
        entries.push(Entry { text, children });
    }
    let mut lines = Vec::new();
    layout(&title, &entries, 1, pages_of(entries.len()), &mut used, &mut lines);

    // build(menu): menu.addItem(title); for line: sub = menu.addSub(parent, text); sub.exec = sub.back
    // (a parent's exec becomes selectSub when its first child is added)
    let mut f = FnBuilder::new(code, "mod-menu/build", &[menu_t], void);
    let menu = f.arg(0);
    let t = f.string_obj(&title)?;
    f.call_new(add_item, &[menu, t])?;
    for line in &lines {
        let p = f.string_obj(&line.parent)?;
        let l = f.string_obj(&line.text)?;
        let item = f.call_new(add_sub, &[menu, p, l])?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_fits_the_font_and_the_screen() {
        assert_eq!(menu_text("skip_layouts: [\"help\", \"main\"]"), "SKIP LAYOUTS: HELP MAIN");
        let long = menu_text(&"x".repeat(80));
        assert_eq!(long.chars().count(), MAX_LINE);
        assert!(long.ends_with("..."));
    }

    #[test]
    fn long_lists_get_pages() {
        let entries: Vec<Entry> = (0..25).map(|i| Entry { text: format!("M{i}"), children: None }).collect();
        let mut lines = Vec::new();
        layout("T", &entries, 1, pages_of(entries.len()), &mut vec![], &mut lines);
        let page = |p: &str| lines.iter().filter(|l| l.parent == p).count();
        // 9 mods + MORE + BACK, twice, then the last 7 + BACK
        assert_eq!(page("T"), PAGE + 1);
        assert_eq!(page("MORE (2/3)"), PAGE + 1);
        assert_eq!(page("MORE (3/3)"), 8);
        assert_eq!(lines.iter().filter(|l| l.text.starts_with('M') && !l.text.starts_with("MORE")).count(), 25);
    }
}
