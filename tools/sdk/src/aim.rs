//! Aiming along the crosshair: the ray from Lina through the game's reticle to the first thing it
//! hits, and the `ray-crosshair` mod that puts the reticle on that point while an item is selected.
//!
//! Vanilla's reticle (`crosshair_point`) stops at level geometry within about 100 units of Lina
//! (an aim point a fixed offset away per held direction, docs/game/input.md). [`AimRay::cast`]
//! follows that direction up to a range with the game's own Box2D ray cast
//! ([`crate::physics::RayCast`]) and returns the nearest object and the point where the ray meets
//! it. For the items that ask ([`show_crosshair`]), the `ray-crosshair` mod places the game's
//! reticle with the same ray, up to the item's range; items that fire with [`AimRay`] from Lina
//! through the reticle act where it is.
//!
//! ```ignore
//! aim::show_crosshair(code, "swap", 400.0)?;          // mod.toml: requires = ["core", "ray-crosshair"]
//! let ray = aim::AimRay::install(code, "swap/ray", true)?;
//! let mut f = hooks::handler(code, "item_use", "swap/use")?;
//! …
//! let (player, px, py) = aim::shooter(&mut f, f.arg(2), handled)?;
//! let (cx, cy) = items::crosshair_pos(&mut f, f.arg(3), handled)?;
//! let range = f.const_f64(400.0);
//! let hit = ray.cast(&mut f, layout, (px, py), (cx, cy), range, Some(player), handled)?;
//! // hit.obj: ObjectClass or null; hit.x, hit.y: where the ray stops
//! ```

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use crate::asm::{FnBuilder, Label};
use crate::physics::RayCast;
use crate::{hooks, Code};

/// The hook the `ray-crosshair` mod defines: `(slot: OClass_b_item) -> F64`, the range of the ray
/// crosshair for the item in `slot`, or negative for none (a number hook, see [`crate::hooks`]).
pub const HOOK: &str = "ray_crosshair_range";

/// Show the ray crosshair while `item` is the selected item, up to `range` layout units. Needs the
/// `ray-crosshair` mod (`requires = ["core", "ray-crosshair"]`).
pub fn show_crosshair(code: &mut Code, item: &str, range: f64) -> Result<()> {
    let mut f = hooks::handler(code, HOOK, &format!("{item}/crosshair_range"))?;
    let slot = f.arg(0);
    let no = f.label();
    crate::items::is_item(&mut f, slot, item, no)?;
    let r = f.const_f64(range);
    f.ret(r);
    f.place(no);
    let r = f.const_f64(-1.0);
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, HOOK, h)
}

/// Where a ray from Lina along the aim stopped.
pub struct AimHit {
    /// The nearest `ObjectClass` on the ray, or null (nothing within range).
    pub obj: Reg,
    /// The point where the ray meets `obj`, or the end of the range (F64).
    pub x: Reg,
    pub y: Reg,
    /// The ray's unit direction (F64), e.g. to step back from the surface: `x - ux * d`.
    pub ux: Reg,
    pub uy: Reg,
}

impl AimHit {
    /// New registers: the point `d` layout units before the hit, back towards the shooter (in
    /// front of the surface, where an object fits without overlapping it).
    pub fn before(&self, f: &mut FnBuilder, d: f64) -> (Reg, Reg) {
        let dist = f.const_f64(-d);
        let (x, y) = (f.reg_f64(), f.reg_f64());
        f.mul(x, self.ux, dist);
        f.add(x, x, self.x);
        f.mul(y, self.uy, dist);
        f.add(y, y, self.y);
        (x, y)
    }
}

pub struct AimRay {
    ray: RayCast,
}

impl AimRay {
    /// Install the ray cast (see [`RayCast::install`]): with `statics`, tiles and other static
    /// bodies stop the ray (the "next surface"); without, it passes through them.
    pub fn install(code: &mut Code, name: &str, statics: bool) -> Result<Self> {
        Ok(Self { ray: RayCast::install(code, name, statics)? })
    }

    /// Cast from `from` (Lina) through `through` (the reticle), `range` long (an F64 register).
    /// `ignore`: an object the ray passes through (the shooter). Jumps to `none` when `through`
    /// is on `from` (no direction).
    #[allow(clippy::too_many_arguments)]
    pub fn cast(
        &self,
        f: &mut FnBuilder,
        layout: Reg,
        from: (Reg, Reg),
        through: (Reg, Reg),
        range: Reg,
        ignore: Option<Reg>,
        none: Label,
    ) -> Result<AimHit> {
        let sqrt = f.code().native("math_sqrt")?;
        let ((px, py), (cx, cy)) = (from, through);
        let (dx, dy, d2, t, len) = (f.reg_f64(), f.reg_f64(), f.reg_f64(), f.reg_f64(), f.reg_f64());
        f.sub(dx, cx, px);
        f.sub(dy, cy, py);
        f.mul(d2, dx, dx);
        f.mul(t, dy, dy);
        f.add(d2, d2, t);
        let eps = f.const_f64(0.01);
        f.jle(d2, eps, none);
        f.call(len, sqrt, &[d2]);
        // unit direction, then * range
        let (ux, uy) = (f.reg_f64(), f.reg_f64());
        f.op(Opcode::SDiv { dst: ux, a: dx, b: len });
        f.op(Opcode::SDiv { dst: uy, a: dy, b: len });
        f.mul(dx, ux, range);
        f.mul(dy, uy, range);
        let (tx, ty) = (f.reg_f64(), f.reg_f64());
        f.add(tx, px, dx);
        f.add(ty, py, dy);
        let obj = self.ray.cast(f, layout, (px, py), (tx, ty), ignore)?;
        // the hit point: from + fraction * (to - from)
        let frac = self.ray.fraction(f);
        let (x, y) = (f.reg_f64(), f.reg_f64());
        f.mul(x, dx, frac);
        f.add(x, x, px);
        f.mul(y, dy, frac);
        f.add(y, y, py);
        Ok(AimHit { obj, x, y, ux, uy })
    }
}

/// In an `item_use` handler: the shooting player (first of the `player` picker argument, as an
/// `ObjectClass`) and her position (F64 registers). Jumps to `none` if there is none.
pub fn shooter(f: &mut FnBuilder, player_picker: Reg, none: Label) -> Result<(Reg, Reg, Reg)> {
    let first = f.code().method("fish.system.Picker", "first")?;
    let player_t = f.code().class("fish.game.oclass.OClass_player")?;
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    f.jnull(player_picker, none);
    let p_dyn = f.call_new(first, &[player_picker])?;
    let player = f.cast(p_dyn, player_t);
    f.jnull(player, none);
    let player = f.cast(player, obj_t);
    let sprite = f.get_new(player, "sprite")?;
    let pos = f.get_new(sprite, "position")?;
    let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
    Ok((player, x, y))
}
