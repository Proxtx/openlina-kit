//! `ray-crosshair`: a utility for item mods. While Lina holds an item that asks for it, a marker
//! shows where her aim ray meets the next surface: the first tile, wall or object along the
//! direction of the game's reticle, within the item's range.
//!
//! Vanilla: the reticle (`crosshair_point`) is a fixed offset from Lina per held direction
//! (docs/game/input.md), so it shows the direction of a shot, not where it lands. Items that act
//! at a distance (swap, the portal gun) hit wherever the ray stops.
//!
//! What the mod adds:
//! - The hook `ray_crosshair_range(slot) -> F64` (`openlina_sdk::aim::HOOK`, a number hook): item
//!   mods subscribe with `openlina_sdk::aim::show_crosshair(code, item, range)` and list this mod in
//!   `requires`; the hook returns the range for their item, -1 for every other.
//! - A `tick` handler (levels only, `world::is_level`): the selected slot is the `b_item` whose `nr`
//!   is Lina's `item_selected`. When the hook gives it a range, the mod casts the ray with
//!   `openlina_sdk::aim::AimRay` (static bodies included, Lina excluded) from Lina through the reticle
//!   and puts the marker (a `Sprite15` with the animation `openlina_ray_crosshair`, from
//!   `assets/images/openlina/ray-crosshair.png`) on the hit point, half its size back along the
//!   ray: the game draws level objects over sprites that overlap them, whatever their layer. With nothing in range, or an item
//!   that didn't ask, the marker is hidden (`Sprite.setVisible`). Items firing with the same
//!   `AimRay` act exactly where the marker is.
//! - The marker belongs to the layout it was created in: a new layout (level, retry) gets a new one.
//!
//! Left alone: the game's reticle (it still shows the direction), firing, every item that doesn't
//! ask. Only the first player gets a marker (co-op: player 2 has none).

use anyhow::Result;
use openlina_sdk::aim::{AimRay, HOOK};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefGlobal, Reg};
use openlina_sdk::{anims, hooks, world, Code, ModConfig};

const SPRITE_TYPE: &str = "Sprite15";
const ANIM: &str = "openlina_ray_crosshair";
const IMAGE: &str = "images/openlina/ray-crosshair.png";
const SIZE: f64 = 9.0;
/// The level's objects' layer ("display").
const LAYER: i32 = 2;

fn main() {
    openlina_sdk::run_mod(apply)
}

/// Mod state, kept in globals.
struct State {
    /// The layout the marker was created in.
    layout: RefGlobal,
    /// The marker object (null until shown in this layout).
    marker: RefGlobal,
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;
    let trace_every = cfg.i64("trace_every", 30)?.max(1) as i32;

    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let f64_t = code.ty_f64();
    hooks::define(code, HOOK, &[b_item_t], f64_t)?;

    let layout_t = code.class("fish.system.Layout")?;
    let obj_t = code.class("fish.system.ObjectClass")?;
    let st = State { layout: code.add_global(layout_t), marker: code.add_global(obj_t) };
    let ray = AimRay::install(code, "ray-crosshair/ray", true)?;
    build_tick(code, &st, &ray, trace.then_some(trace_every))
}

fn build_tick(code: &mut Code, st: &State, ray: &AimRay, trace_every: Option<i32>) -> Result<()> {
    let hook = hooks::find(code, HOOK)?;
    let first = code.method("fish.system.Picker", "first")?;
    let set_visible = code.method("fish.system.Sprite", "setVisible")?;
    let set_anim = code.method("fish.system.Sprite", "set_anim")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let cross_t = code.class("fish.game.oclass.OClass_crosshair_point")?;
    let obj_t = code.class("fish.system.ObjectClass")?;

    let mut f = hooks::handler(code, "tick", "ray-crosshair/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let (hide, end) = (f.label(), f.label());

    // A new layout: the old marker went with the old one.
    let same = f.label();
    let last = f.get_global(st.layout);
    f.jeq(last, layout, same);
    f.set_global(st.layout, layout);
    f.clear_global(st.marker);
    f.place(same);
    world::jump_unless_level(&mut f, layout, hide)?;
    let tick = f.get_new(layout, "currentTick")?;

    // Lina and her selected slot (the b_item whose nr is her item_selected).
    let picker = f.get_new(sheet, "player")?;
    f.jnull(picker, hide);
    let p_dyn = f.call_new(first, &[picker])?;
    let lina = f.cast(p_dyn, player_t);
    f.jnull(lina, hide);
    let selected = f.get_new(lina, "item_selected")?;
    let slot = f.reg(b_item_t);
    f.op(Opcode::Null { dst: slot });
    let slots = f.get_new(sheet, "b_item")?;
    f.jnull(slots, hide);
    let insts = f.get_new(slots, "insts")?;
    f.jnull(insts, hide);
    let n = f.array_len(insts)?;
    f.for_range(n, |f, i| {
        let next = f.label();
        let o = f.array_get(insts, i, obj_t)?;
        let s = f.cast(o, b_item_t);
        f.jnull(s, next);
        let nr = f.get_new(s, "nr")?;
        f.jne(nr, selected, next);
        f.mov(slot, s);
        f.place(next);
        Ok(())
    })?;
    f.jnull(slot, hide);
    let range = f.call_new(hook, &[slot])?;
    let zero = f.const_f64(0.0);
    f.jlt(range, zero, hide);

    // The ray from Lina through the reticle.
    let crosshairs = f.get_new(sheet, "crosshair_point")?;
    f.jnull(crosshairs, hide);
    let c_dyn = f.call_new(first, &[crosshairs])?;
    let cross = f.cast(c_dyn, cross_t);
    f.jnull(cross, hide);
    let csprite = f.get_new(cross, "sprite")?;
    let cpos = f.get_new(csprite, "position")?;
    let (cx, cy) = (f.get_new(cpos, "x")?, f.get_new(cpos, "y")?);
    let me = f.cast(lina, obj_t);
    let lsprite = f.get_new(me, "sprite")?;
    let lpos = f.get_new(lsprite, "position")?;
    let (px, py) = (f.get_new(lpos, "x")?, f.get_new(lpos, "y")?);
    let hit = ray.cast(&mut f, layout, (px, py), (cx, cy), range, Some(me), hide)?;
    f.jnull(hit.obj, hide);
    // just in front of the surface: level objects are drawn over sprites that overlap them
    let (mx, my) = hit.before(&mut f, SIZE / 2.0 + 2.0);

    // Show the marker there (create it the first time in this layout).
    let marker = f.get_global(st.marker);
    let have = f.label();
    f.jnotnull(marker, have);
    let created = world::spawn_on(&mut f, layout, SPRITE_TYPE, LAYER, mx, my)?;
    let created = f.cast(created, obj_t);
    f.mov(marker, created);
    f.set_global(st.marker, marker);
    f.place(have);
    let sprite = f.get_new(marker, "sprite")?;
    let pos = f.get_new(sprite, "position")?;
    f.set(pos, "x", mx)?;
    f.set(pos, "y", my)?;
    let yes = f.const_i32(1);
    f.call_new(set_visible, &[sprite, yes])?;
    let anim_done = f.label();
    let anim =
        anims::ensure(&mut f, &format!("fish.game.oclass.OClass_{SPRITE_TYPE}"), ANIM, IMAGE, (SIZE, SIZE), anim_done)?;
    f.call_new(set_anim, &[sprite, anim])?;
    let s = f.const_f64(SIZE);
    f.set(sprite, "width", s)?;
    f.set(sprite, "height", s)?;
    f.place(anim_done);
    if let Some(every) = trace_every {
        every_ticks(&mut f, tick, every, end)?;
        let kind = f.get_new(hit.obj, "type")?;
        f.print(&[
            Print::Str("[ray-crosshair] tick "),
            Print::Val(tick),
            Print::Str(": aim hits "),
            Print::Val(kind),
            Print::Str(" at ("),
            Print::Val(hit.x),
            Print::Str(", "),
            Print::Val(hit.y),
            Print::Str("), Lina at ("),
            Print::Val(px),
            Print::Str(", "),
            Print::Val(py),
            Print::Str(")"),
        ])?;
        // the marker in the format of the `trace-positions` fixture, for `position` expectations
        f.print(&[
            Print::Str("[pos] tick "),
            Print::Val(tick),
            Print::Str(" ray-crosshair "),
            Print::Val(mx),
            Print::Str(" "),
            Print::Val(my),
        ])?;
    }
    f.jmp(end);

    // Hide it (if there is one).
    f.place(hide);
    let marker = f.get_global(st.marker);
    f.jnull(marker, end);
    let sprite = f.get_new(marker, "sprite")?;
    f.jnull(sprite, end);
    let no = f.const_i32(0);
    f.call_new(set_visible, &[sprite, no])?;
    if let Some(every) = trace_every {
        let tick = f.get_new(layout, "currentTick")?;
        every_ticks(&mut f, tick, every, end)?;
        f.print(&[Print::Str("[ray-crosshair] tick "), Print::Val(tick), Print::Str(": marker hidden")])?;
    }
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}

/// Jump to `skip` unless `tick` is a multiple of `every`.
fn every_ticks(f: &mut FnBuilder, tick: Reg, every: i32, skip: openlina_sdk::asm::Label) -> Result<()> {
    let (e, r, zero) = (f.const_i32(every), f.reg_i32(), f.const_i32(0));
    f.op(Opcode::SMod { dst: r, a: tick, b: e });
    f.jne(r, zero, skip);
    Ok(())
}
