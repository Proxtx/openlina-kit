//! `ray-crosshair`: a utility for item mods. While Lina holds an item that asks for it, the game's
//! own reticle (the "+") reaches as far as the item does: it sits on the first tile, wall or object
//! along the aim, within the item's range, or at the end of the range.
//!
//! Vanilla (`EvSheet_gameplay.update`, L10879-10936): for the aim types short, mid and long
//! (`aimType` 1, 3, 5) the game picks an aim point (`short_aim_point`, `mid_aim_point`,
//! `max_aim_point`: fixed offsets from Lina per held direction, docs/game/input.md) and casts its
//! line of sight from the `gun` to it (`LOS.castRay`, obstacles: the `solid` family). The reticle
//! (`crosshair_point`) goes to the hit, or to the aim point when nothing is in between. So the
//! reticle already is a ray-cast crosshair, only short (about 100 units) and blind to objects.
//! The other aim types put the reticle on the aim point without a ray.
//!
//! What the mod changes:
//! - The hook `ray_crosshair_range(slot) -> F64` (`openlina_sdk::aim::HOOK`, a number hook): item
//!   mods subscribe with `openlina_sdk::aim::show_crosshair(code, item, range)` and list this mod in
//!   `requires`; the hook returns the range for their item, -1 for every other.
//! - Right before vanilla's ray (the cast whose result places the reticle), a guard asks the hook
//!   for the selected slot (the `b_item` the update just read the aim type from). With a range, the
//!   mod casts its own ray (`openlina_sdk::aim::AimRay`: tiles, walls and objects, Lina excluded)
//!   from the gun along the direction to the aim point, puts the reticle on the hit or at the end
//!   of the range, and skips vanilla's ray. Without, vanilla goes on.
//!
//! Items firing with the same `AimRay` from Lina through the reticle act where it is.
//!
//! Left alone: every item that doesn't ask, the aim types without a ray (short2, mid2, long2,
//! shoot, remote), the reticle's looks, the aim direction. Each player's reticle is placed for
//! her own selected item (the update does it per player).

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::aim::{AimRay, HOOK};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::{
    add_reg, call, call_target, find_calls, insert_ops_with_exits, is_field, next_match, prev_match, Exit, Incoming,
};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;
    let trace_every = cfg.i64("trace_every", 30)?.max(1) as i32;

    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let f64_t = code.ty_f64();
    hooks::define(code, HOOK, &[b_item_t], f64_t)?;
    let ray = AimRay::install(code, "ray-crosshair/ray", true)?;

    let site = reticle_site(code)?;
    let place = build_place(code, &ray, &site, trace.then_some(trace_every))?;

    let update = code.find_fn("fish.game.evsheet.EvSheet_gameplay.update")?;
    let bool_t = code.ty_bool();
    let f = code.func_mut(update)?;
    let handled = add_reg(f, bool_t);
    let args = [Reg(1), site.slot, site.player, site.gx, site.gy, site.ax, site.ay, site.cross];
    insert_ops_with_exits(
        f,
        site.cast,
        vec![call(handled, place, &args), Opcode::JTrue { cond: handled, offset: 0 }],
        &[Exit { op: 1, target: site.end }],
        Incoming::ToInserted,
    );
    Ok(())
}

/// Vanilla's ray that places the reticle, in `EvSheet_gameplay.update`.
struct Site {
    /// The `castRay(los, gx, gy, ax, ay, true)` call.
    cast: usize,
    /// Where both of its outcomes go on (the reticle is placed).
    end: usize,
    gx: Reg,
    gy: Reg,
    ax: Reg,
    ay: Reg,
    /// The selected `OClass_b_item` (its `item.type.aimType` chose the aim point).
    slot: Reg,
    /// The `OClass_player` aiming.
    player: Reg,
    /// The `OClass_crosshair_point`.
    cross: Reg,
}

/// Find the cast by meaning: of the update's `LOS.castRay` calls, the one whose hit goes straight
/// into the reticle's position (`if (los.rayIntersected) cross.sprite.position = (hitX, hitY)
/// else cross.sprite.position = aim point`).
fn reticle_site(code: &Code) -> Result<Site> {
    let update = code.find_fn("fish.game.evsheet.EvSheet_gameplay.update")?;
    let cast_ray = code.method("fish.system.beh.LOS", "castRay")?;
    let cross_t = code.class("fish.game.oclass.OClass_crosshair_point")?;
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let fun = code.func(update)?;
    let reg_t = |r: Reg| fun.regs[r.0 as usize];

    let mut found = vec![];
    for at in find_calls(fun, cast_ray) {
        // next: `if (!los.rayIntersected) goto else`, then `cross.sprite` within a few ops
        let Some(test) = next_match(fun, at, |op| is_field(code, fun, op, "rayIntersected")) else { continue };
        let Some(jf) = next_match(fun, test, |op| matches!(op, Opcode::JFalse { .. })) else { continue };
        let Some(spr) = next_match(fun, jf, |op| is_field(code, fun, op, "sprite")) else { continue };
        let Opcode::Field { obj, .. } = fun.ops[spr] else { continue };
        if test - at <= 4 && spr - jf <= 2 && reg_t(obj) == cross_t {
            found.push((at, jf, obj));
        }
    }
    let [(at, jf, cross)] = found[..] else {
        bail!("expected one castRay whose hit places the reticle in EvSheet_gameplay.update, found {}", found.len())
    };
    let (_, args) = call_target(&fun.ops[at]).context("castRay is a call")?;
    ensure!(args.len() == 6, "castRay(los, x1, y1, x2, y2, b) has {} args here", args.len());
    let else_at = match fun.ops[jf] {
        Opcode::JFalse { offset, .. } => (jf as i64 + 1 + offset as i64) as usize,
        _ => unreachable!(),
    };
    // the then-branch ends with a jump past the else-branch: where both go on
    let end = match fun.ops[else_at - 1] {
        Opcode::JAlways { offset } => (else_at as i64 + offset as i64) as usize,
        ref op => bail!("the reticle's then-branch doesn't end with a jump: {op:?}"),
    };
    let slot_read = prev_match(fun, at, |op| {
        is_field(code, fun, op, "item") && matches!(op, Opcode::Field { obj, .. } if reg_t(*obj) == b_item_t)
    })
    .context("the selected b_item's `.item` read before the cast")?;
    let Opcode::Field { obj: slot, .. } = fun.ops[slot_read] else { unreachable!() };
    let look = prev_match(fun, at, |op| {
        is_field(code, fun, op, "look_input") && matches!(op, Opcode::Field { obj, .. } if reg_t(*obj) == player_t)
    })
    .context("the player's `.look_input` read before the cast")?;
    let Opcode::Field { obj: player, .. } = fun.ops[look] else { unreachable!() };
    Ok(Site { cast: at, end, gx: args[1], gy: args[2], ax: args[3], ay: args[4], slot, player, cross })
}

/// `ray-crosshair/place(layout, slot, player, gx, gy, ax, ay, cross) -> Bool`: true when it placed
/// the reticle (vanilla's ray is skipped), false for items that didn't ask.
fn build_place(
    code: &mut Code,
    ray: &AimRay,
    site: &Site,
    trace_every: Option<i32>,
) -> Result<openlina_sdk::hlbc::types::RefFun> {
    let hook = hooks::find(code, HOOK)?;
    let update = code.find_fn("fish.game.evsheet.EvSheet_gameplay.update")?;
    let fun = code.func(update)?;
    let t = |r: Reg| fun.regs[r.0 as usize];
    let args = [t(Reg(1)), t(site.slot), t(site.player), t(site.gx), t(site.gy), t(site.ax), t(site.ay), t(site.cross)];
    let obj_t = code.class("fish.system.ObjectClass")?;
    let bool_t = code.ty_bool();

    let mut f = FnBuilder::new(code, "ray-crosshair/place", &args, bool_t);
    let (layout, slot, player, gx, gy, ax, ay, cross) =
        (f.arg(0), f.arg(1), f.arg(2), f.arg(3), f.arg(4), f.arg(5), f.arg(6), f.arg(7));
    let vanilla = f.label();
    f.jnull(slot, vanilla);
    let range = f.call_new(hook, &[slot])?;
    let zero = f.const_f64(0.0);
    f.jlt(range, zero, vanilla);
    f.jnull(cross, vanilla);
    let lina = f.cast(player, obj_t);
    let hit = ray.cast(&mut f, layout, (gx, gy), (ax, ay), range, Some(lina), vanilla)?;
    // on the hit, or at the end of the range: kept on screen (where the ray leaves it), so the
    // reticle still shows the direction when the aim meets nothing
    let clipped = f.label();
    f.jnotnull(hit.obj, clipped);
    clip_to_screen(&mut f, (gx, gy), &hit)?;
    f.place(clipped);
    let sprite = f.get_new(cross, "sprite")?;
    let pos = f.get_new(sprite, "position")?;
    f.set(pos, "x", hit.x)?;
    f.set(pos, "y", hit.y)?;
    if let Some(every) = trace_every {
        let (quiet, nothing, printed) = (f.label(), f.label(), f.label());
        let tick = f.get_new(layout, "currentTick")?;
        let (e, r, z) = (f.const_i32(every), f.reg_i32(), f.const_i32(0));
        f.op(Opcode::SMod { dst: r, a: tick, b: e });
        f.jne(r, z, quiet);
        f.jnull(hit.obj, nothing);
        let kind = f.get_new(hit.obj, "type")?;
        f.print(&[
            Print::Str("[ray-crosshair] tick "),
            Print::Val(tick),
            Print::Str(": reticle on "),
            Print::Val(kind),
            Print::Str(" at ("),
            Print::Val(hit.x),
            Print::Str(", "),
            Print::Val(hit.y),
            Print::Str(")"),
        ])?;
        f.jmp(printed);
        f.place(nothing);
        f.print(&[
            Print::Str("[ray-crosshair] tick "),
            Print::Val(tick),
            Print::Str(": reticle at the end of the range ("),
            Print::Val(hit.x),
            Print::Str(", "),
            Print::Val(hit.y),
            Print::Str(")"),
        ])?;
        f.place(printed);
        // the reticle in the format of the `trace-positions` fixture, for `position` expectations
        f.print(&[
            Print::Str("[pos] tick "),
            Print::Val(tick),
            Print::Str(" ray-crosshair "),
            Print::Val(hit.x),
            Print::Str(" "),
            Print::Val(hit.y),
        ])?;
        f.place(quiet);
    }
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.ret(yes);
    f.place(vanilla);
    let no = f.reg(bool_t);
    f.bool(no, false);
    f.ret(no);
    f.finish()
}

/// Where a clipped reticle stays visible (layout units, `(lo, hi)` per axis): inside the screen,
/// below the HUD bar at the top and above the level name at the bottom.
const VISIBLE_X: (f64, f64) = (10.0, 590.0);
const VISIBLE_Y: (f64, f64) = (32.0, 312.0);

/// Pull `hit` (x, y) back along the ray towards `from` until it is visible: per axis, the largest
/// t with `from + t * u` inside `VISIBLE_X` / `VISIBLE_Y`, and the point at the smallest of them if
/// that is shorter than the ray.
fn clip_to_screen(f: &mut FnBuilder, (gx, gy): (Reg, Reg), hit: &openlina_sdk::aim::AimHit) -> Result<()> {
    let (len, t, lim) = (f.reg_f64(), f.reg_f64(), f.reg_f64());
    // the ray's length so far: (hit - from) . u
    f.sub(len, hit.x, gx);
    f.mul(len, len, hit.ux);
    f.sub(t, hit.y, gy);
    f.mul(t, t, hit.uy);
    f.add(len, len, t);
    let zero = f.const_f64(0.0);
    for (p, u, (lo, hi)) in [(gx, hit.ux, VISIBLE_X), (gy, hit.uy, VISIBLE_Y)] {
        let (neg, apply, next) = (f.label(), f.label(), f.label());
        f.jlt(u, zero, neg);
        f.jle(u, zero, next);
        f.float(lim, hi);
        f.jmp(apply);
        f.place(neg);
        f.float(lim, lo);
        f.place(apply);
        // t = (lim - p) / u; shorter: use it
        f.sub(t, lim, p);
        f.op(Opcode::SDiv { dst: t, a: t, b: u });
        f.jge(t, len, next);
        f.mov(len, t);
        f.place(next);
    }
    // never behind the gun (Lina herself off the screen)
    let ahead = f.label();
    f.jge(len, zero, ahead);
    f.mov(len, zero);
    f.place(ahead);
    f.mul(t, hit.ux, len);
    f.add(hit.x, gx, t);
    f.mul(t, hit.uy, len);
    f.add(hit.y, gy, t);
    Ok(())
}
