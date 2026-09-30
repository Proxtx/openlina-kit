//! Items: the tools the player picks up and fires (box, bomb, …).
//!
//! The game's item types are records `{aimType, baseAmmo, name, secondLayer}` in
//! `itemManager.itemPool`, rolled into the HUD slots (`OClass_b_item`, `slot.item.type`). The HUD
//! icon is the `item_icon` object showing the animation named after the item. Firing runs the
//! closure in `EvSheet_gameplay.shoot`, which decrements ammo and then dispatches on the item name.
//!
//! [`register`] adds an item type (pool entry + HUD icon) through the core hooks `item_pool` and
//! `tick`; the mod implements its behavior in an `item_use` handler:
//!
//! ```ignore
//! items::register(code, &Item { name: "portal", label: "Portal Gun", ammo: 4, aim: Aim::Long, second_layer: false,
//!     icon: "images/openlina/portal-gun.png", icon_size: (12.0, 12.0),
//!     big_icon: Some(("images/openlina/portal-gun-big.png", (24.0, 24.0))) })?;
//! let mut f = hooks::handler(code, "item_use", "portal-gun/use")?;
//! let not_mine = f.label();
//! items::is_item(&mut f, f.arg(0), "portal", not_mine)?;
//! let (x, y) = items::crosshair_pos(&mut f, f.arg(3), not_mine)?;
//! … // do it, return true; at not_mine: return false
//! ```

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use crate::asm::{FnBuilder, Label};
use crate::{hooks, Code};

/// How the player aims the item (the game's `aim_type`).
#[derive(Clone, Copy, Debug)]
pub enum Aim {
    Shoot = 0,
    Short = 1,
    Short2 = 2,
    Mid = 3,
    Mid2 = 4,
    Long = 5,
    Long2 = 6,
    Remote = 7,
}

pub struct Item<'a> {
    /// Item name: the pool entry and the HUD icon's animation name.
    pub name: &'a str,
    /// HUD label (the game looks it up as `TOOL_<NAME>`).
    pub label: &'a str,
    /// Starting ammo (`baseAmmo`).
    pub ammo: i32,
    pub aim: Aim,
    /// Only in second-layer runs.
    pub second_layer: bool,
    /// HUD icon image under `fish/game/res/` (ship it in the mod's `assets/`); vanilla icons are
    /// 14×14.
    pub icon: &'a str,
    pub icon_size: (f64, f64),
    /// Large icon for the run start's tool selection and the editor (vanilla: 28×28), e.g. the
    /// icon art rendered with `lina sprite --scale 2`. `None` shows the HUD icon there, small.
    pub big_icon: Option<(&'a str, (f64, f64))>,
}

/// Add an item type to the game: its pool entry and its HUD icon. Requires the `core` mod.
pub fn register(code: &mut Code, item: &Item) -> Result<()> {
    // item_pool(itemManager): itemPool.push({aimType, baseAmmo, name, secondLayer})
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let slot_t = code.field_type(b_item_t, code.field(b_item_t, "item")?)?;
    let rec_t = code.field_type(slot_t, code.field(slot_t, "type")?)?;
    let push = code.method("hl.types.ArrayObj", "push")?;
    let bool_t = code.ty_bool();
    let mut f = hooks::handler(code, "item_pool", &format!("item/{}/pool", item.name))?;
    let im = f.arg(0);
    let pool = f.get_new(im, "itemPool")?;
    let rec = f.reg(rec_t);
    f.op(Opcode::New { dst: rec });
    let name = f.string_obj(item.name)?;
    f.set(rec, "name", name)?;
    let ammo = f.const_i32(item.ammo);
    f.set(rec, "baseAmmo", ammo)?;
    let aim = f.const_i32(item.aim as i32);
    f.set(rec, "aimType", aim)?;
    let second = f.reg(bool_t);
    f.bool(second, item.second_layer);
    f.set(rec, "secondLayer", second)?;
    f.call_new(push, &[pool, rec])?;
    item_object(&mut f, item)?;
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "item_pool", h)?;

    // The icon animation (frame 0: HUD; frame 1: tool selection grid and editor, which set
    // `animFrame = 1`). The icon class's animation map is built when the first icon is created,
    // so this runs on every tick until it exists, and before the editor reads the map
    // (`Editor.loadTiles`).
    let big = item.big_icon.unwrap_or((item.icon, item.icon_size));
    let frames = [(item.icon, item.icon_size), big];
    let mut f = hooks::handler(code, "tick", &format!("item/{}/icon", item.name))?;
    let done = f.label();
    crate::anims::ensure_frames(&mut f, "fish.game.oclass.OClass_item_icon", item.name, &frames, done)?;
    f.place(done);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)?;
    let load_tiles = code.method("editor.Editor", "loadTiles")?;
    let editor_t = code.class("editor.Editor")?;
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, &format!("item/{}/editor_icon", item.name), &[editor_t], void);
    let done = f.label();
    crate::anims::ensure_frames(&mut f, "fish.game.oclass.OClass_item_icon", item.name, &frames, done)?;
    f.place(done);
    f.ret_void();
    let h = f.finish()?;
    crate::edit::prepend_call(code, load_tiles, h, &[Reg(0)])?;

    // HUD label
    crate::text::set(code, &format!("TOOL_{}", item.name.to_uppercase()), item.label)
}

/// The game's name for an aim type (`OClass_item.aim_type`).
fn aim_name(aim: Aim) -> &'static str {
    match aim {
        Aim::Shoot => "shoot",
        Aim::Short => "short",
        Aim::Short2 => "short2",
        Aim::Mid => "mid",
        Aim::Mid2 => "mid2",
        Aim::Long => "long",
        Aim::Long2 => "long2",
        Aim::Remote => "remote",
    }
}

/// Emit: give the item a global `item` object, like the game's own 48. The run start's tool
/// selection (the MANAGER screen: its grid icons and the unlock animation look items up by
/// `NAME`) and the editor's tool override list both go through these objects; without one, a
/// run whose roll drew the item waits forever. The objects form a 12-column grid from (150, 140)
/// with 30 units spacing; mod items continue it (the 49th object goes to (150, 260)).
fn item_object(f: &mut FnBuilder, item: &Item) -> Result<()> {
    let item_t = f.code().class("fish.game.oclass.OClass_item")?;
    let create = f.code().method("fish.system.Layout", "createObject")?;
    let cb_t = f.code().func_type(create)?.args[7];
    let bool_t = f.code().ty_bool();
    let done = f.label();
    let st = f.static_obj("fish.system.Main")?;
    let main = f.get_new(st, "i")?;
    let game = f.get_new(main, "game")?;
    let ev = f.get_new(game, "ev_instancing_ev")?;
    let picker = f.get_new(ev, "item")?;
    let insts = f.get_new(picker, "insts")?;
    let n = f.array_len(insts)?;
    let zero = f.const_i32(0);
    f.jle(n, zero, done); // no vanilla objects to join
                          // Already there (the pool gets rebuilt)?
    f.for_range(n, |f, i| {
        let o = f.array_get(insts, i, item_t)?;
        let name = f.get_new(o, "NAME")?;
        let other = f.label();
        f.jstr_ne(name, item.name, other)?;
        f.jmp(done);
        f.place(other);
        Ok(())
    })?;
    let first = f.array_get(insts, zero, item_t)?;
    let layout = f.get_new(first, "layout")?;
    // Grid slot n: x = 150 + 30 * (n % 12), y = 140 + 30 * (n / 12).
    let i32_t = f.code().ty_i32();
    let (col, row) = (f.reg(i32_t), f.reg(i32_t));
    let twelve = f.const_i32(12);
    f.op(Opcode::SMod { dst: col, a: n, b: twelve });
    f.op(Opcode::SDiv { dst: row, a: n, b: twelve });
    let f64_t = f.code().ty_f64();
    let (x, y) = (f.reg(f64_t), f.reg(f64_t));
    f.op(Opcode::ToSFloat { dst: x, src: col });
    f.op(Opcode::ToSFloat { dst: y, src: row });
    let (step, x0, y0) = (f.const_f64(30.0), f.const_f64(150.0), f.const_f64(140.0));
    f.mul(x, x, step);
    f.add(x, x, x0);
    f.mul(y, y, step);
    f.add(y, y, y0);
    let ty = f.string_obj("item")?;
    let layer = f.const_i32(0);
    let no = f.reg(bool_t);
    f.bool(no, false);
    let template = f.string_obj("")?;
    let cb = f.reg(cb_t);
    f.op(Opcode::Null { dst: cb });
    let obj = f.call_new(create, &[layout, ty, layer, x, y, no, template, cb])?;
    let obj = f.cast(obj, item_t);
    let name = f.string_obj(item.name)?;
    f.set(obj, "NAME", name)?;
    let ammo = f.const_i32(item.ammo);
    f.set(obj, "ammo", ammo)?;
    let aim = f.string_obj(aim_name(item.aim))?;
    f.set(obj, "aim_type", aim)?;
    let one = f.const_f64(1.0);
    f.set(obj, "unlocked", one)?;
    let second = f.reg(bool_t);
    f.bool(second, item.second_layer);
    f.set(obj, "secondLayer", second)?;
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.set(obj, "isGlobal", yes)?;
    let sprite = f.get_new(obj, "sprite")?;
    f.set(sprite, "visible", no)?;
    f.place(done);
    Ok(())
}

/// Emit: jump to `not` unless `slot` (an `OClass_b_item`) holds the item called `name`.
pub fn is_item(f: &mut FnBuilder, slot: Reg, name: &str, not: Label) -> Result<()> {
    f.jnull(slot, not);
    let rec = f.get_new(slot, "item")?;
    f.jnull(rec, not);
    let ty = f.get_new(rec, "type")?;
    f.jnull(ty, not);
    let n = f.get_new(ty, "name")?;
    f.jstr_ne(n, name, not)
}

/// Emit: the aim point from `item_use`'s `crosshair` argument (an array holding the player's
/// `crosshair_point` picker): new F64 registers (x, y). Jumps to `none` if there is no crosshair.
pub fn crosshair_pos(f: &mut FnBuilder, crosshair: Reg, none: Label) -> Result<(Reg, Reg)> {
    let n = f.array_len(crosshair)?;
    let zero = f.const_i32(0);
    f.jle(n, zero, none);
    let picker_t = f.code().class("fish.system.Picker")?;
    let picker = f.array_get(crosshair, zero, picker_t)?;
    f.jnull(picker, none);
    let first = f.code().method("fish.system.Picker", "first")?;
    let c_dyn = f.call_new(first, &[picker])?;
    let t = f.code().class("fish.game.oclass.OClass_crosshair_point")?;
    let c = f.cast(c_dyn, t);
    f.jnull(c, none);
    let sprite = f.get_new(c, "sprite")?;
    let pos = f.get_new(sprite, "position")?;
    Ok((f.get_new(pos, "x")?, f.get_new(pos, "y")?))
}
