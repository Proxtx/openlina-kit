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
//!     icon: "images/openlina/portal-gun.png", icon_size: (12.0, 12.0) })?;
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
    /// 12×12.
    pub icon: &'a str,
    pub icon_size: (f64, f64),
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
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "item_pool", h)?;

    // tick: make sure the HUD icon animation exists (the icon object's animation map is built
    // when the first icon is created, so this can't happen earlier).
    let mut f = hooks::handler(code, "tick", &format!("item/{}/icon", item.name))?;
    let done = f.label();
    crate::anims::ensure(&mut f, "fish.game.oclass.OClass_item_icon", item.name, item.icon, item.icon_size, done)?;
    f.place(done);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)?;

    // HUD label
    crate::text::set(code, &format!("TOOL_{}", item.name.to_uppercase()), item.label)
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
