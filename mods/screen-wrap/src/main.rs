//! `screen-wrap`: a modifier. In levels that roll it, objects leaving the screen come back on the
//! opposite side.
//!
//! It registers a modifier (`openlina_sdk::modifiers`, key `screen-wrap`) with its own HUD icon
//! (`assets/images/openlina/screen-wrap.png`), rolled like the vanilla modifiers. With the option
//! `always`, it applies in every level instead and no modifier is registered.
//!
//! Subscribes to the core `edge_exit` hook, which fires when vanilla is about to delete an
//! object that left the screen (see `mods/core`). The handler moves the object by one
//! play-field size and returns true ("handled", vanilla skipped).
//!
//! In long levels and boss arenas (`sheet.bossMode`, levels that scroll horizontally) vanilla
//! only tests the top and bottom edges, so only those wrap there.
//!
//! What leaving the screen does in vanilla still happens; the object then wraps instead of
//! disappearing:
//! - Fruits (`coins`, kind 1): a level is won by collecting every fruit. Touching one collects it
//!   (`tryTouchCoin` → `coin_fn`, `state = 1`); so does deleting it, by leaving the screen
//!   (`coinedgecheck` destroys it) or with the delete tool: the fruit's destroy listener (closure in
//!   `setupEvents`, L9490) calls `coin_fn(touched = 0)` and sets `state = 1` when it wasn't collected
//!   yet and isn't glitched. The mod does the same for a fruit that wraps, so pushing a fruit off the
//!   screen still collects it; it just comes back on the other side, already collected. Glitched
//!   fruits stay vanilla: until every player touched them they go back to their spawn.
//! - Frogs (`frogs`, kind 3): vanilla raises `frogland_count` for a live frog (they show up in
//!   "greenfrogs 1") and then deletes it; the count still rises on every crossing. The frog the
//!   player steers in frog mode (`sheet.frogMode`, `frog.playerControlled`, `is_fat == 0`) is left to
//!   vanilla: deleting it is how that level is lost (its destroy listener calls `end_level`).
//!
//! Joined objects (`joined`): step ladders, bamboo, tentacles, vines… are several bodies held by
//! Box2D joints. Wrapping one piece alone would tear it across the screen, so the handler walks the
//! joints (`body_get_joint_list`, `joint_get_body_a/b`) to the whole group (owners found among
//! `physics_obj` and `secondary_physics`, at most `MAX_GROUP` pieces). Pieces past the edge are kept
//! while the group's centre is on screen; when the centre crosses, every piece moves by the same
//! offset, so the joints stay as they were. Groups held by something that can't move (a static or
//! kinematic body: a vine hanging from the ceiling, a bridge between tiles), by Lina, or by a body
//! the mod can't find are left to vanilla (the piece outside is deleted).
//!
//! Left to vanilla (the handler returns false):
//! - fruits unless `coins`, frogs unless `frogs`, secondary physics objects unless `secondary`
//! - objects during the first `min_tick` ticks of a layout (levels place objects off-screen and
//!   rely on the edge test to delete them)
//! - objects more than `max_overshoot` past the edge (the game parks objects at e.g. -1000,-1000)
//!
//! Moving `sprite.position` is enough: `Physics.syncPosWithSprite` teleports the Box2D body and
//! keeps its velocity.
//!
//! Lina (option `player`): vanilla doesn't delete her at the edge but calls `player_death`
//! (`EvSheet_gameplay.update`, source L16200-16213; no `edgewith`; x only outside boss mode).
//! The mod subscribes `screen-wrap/player` to the core `player_edge` hook, the same wrap with
//! `edgewith` 0: in a pit she falls back in from the top, off a side she comes back on the
//! other one. Her other death (the explosion) stays vanilla, and so does the hub (layout
//! `help`), where walking off the right edge starts a run. Off by default: falling off the
//! screen is how vanilla levels are lost.

use anyhow::Result;
use openlina_sdk::asm::{FnBuilder, Label, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::modifiers::{self, Modifier};
use openlina_sdk::{hooks, Code, ModConfig};

/// Play-field size in layout units, hardcoded in the game's edge test.
const SCREEN_W: f64 = 600.0;
const SCREEN_H: f64 = 338.0;
/// The most pieces a joined group may have (bigger ones are left to vanilla).
const MAX_GROUP: i32 = 64;
/// `b2BodyType`: only dynamic bodies move.
const DYNAMIC_BODY: i32 = 2;

fn main() {
    openlina_sdk::run_mod(apply)
}

/// What both wraps share.
struct Wrap {
    modifier: Option<i32>,
    max_overshoot: f64,
    min_tick: i32,
    trace: bool,
}

/// The edge-exit kinds of the core hook.
const KIND_COIN: i32 = 1;
const KIND_SECONDARY: i32 = 2;
const KIND_FROG: i32 = 3;

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let coins = cfg.bool("coins", true)?;
    let frogs = cfg.bool("frogs", true)?;
    let joined = cfg.bool("joined", true)?;
    let secondary = cfg.bool("secondary", true)?;
    let player = cfg.bool("player", false)?;
    let always = cfg.bool("always", false)?;
    let modifier = if always {
        None
    } else {
        Some(modifiers::register(
            code,
            &Modifier { key: "screen-wrap", icon: "images/openlina/screen-wrap.png", size: (16.0, 16.0), in_dx: true },
        )?)
    };
    let w = Wrap {
        modifier,
        max_overshoot: cfg.f64("max_overshoot", 200.0)?,
        min_tick: cfg.i64("min_tick", 5)? as i32,
        trace: cfg.bool("trace", false)?,
    };

    // Objects: the core `edge_exit` hook.
    let coin_t = code.class("fish.game.oclass.OClass_coin")?;
    let frog_t = code.class("fish.game.oclass.OClass_frog")?;
    let mut f = hooks::handler(code, "edge_exit", "screen-wrap/wrap")?;
    let (pos, ew, margin, sheet, kind, physics) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3), f.arg(4), f.arg(5));
    let fail = f.label();
    let tick = guards(&mut f, &w, sheet, fail)?;

    // Which kinds of objects to handle.
    let (kinds_ok, not_coin, not_frog) = (f.label(), f.label(), f.label());
    let c = f.const_i32(KIND_COIN);
    f.jne(kind, c, not_coin);
    if coins {
        // glitched fruits: vanilla (back to the spawn until every player touched them)
        f.jnull(physics, fail);
        let owner = f.get_new(physics, "owner")?;
        let coin = f.cast(owner, coin_t);
        f.jnull(coin, fail);
        let glitched = f.get_new(coin, "glitched")?;
        f.jtrue(glitched, fail);
        // leaving the screen collects a fruit (vanilla: its destroy listener), so collect it as
        // that listener does before it wraps: `coin_fn(touched = 0)`, `state = 1`
        let state = f.get_new(coin, "state")?;
        let zero = f.const_f64(0.0);
        f.jne(state, zero, kinds_ok); // already collected (touched, or wrapped before)
        let coin_fn = f.code().method("fish.game.evsheet.EvSheet_gameplay", "coin_fn")?;
        let f64_t = f.code().ty_f64();
        let ref_t = f.code().intern_type(openlina_sdk::hlbc::types::Type::Ref(f64_t));
        let (touched, touched_ref) = (f.reg_f64(), f.reg(ref_t));
        f.float(touched, 0.0);
        f.op(Opcode::Ref { dst: touched_ref, src: touched });
        f.call_new(coin_fn, &[sheet, touched_ref])?;
        let one = f.const_f64(1.0);
        f.set(coin, "state", one)?;
        if w.trace {
            f.print(&[Print::Str("[screen-wrap] tick "), Print::Val(tick), Print::Str(" fruit collected")])?;
        }
        f.jmp(kinds_ok);
    } else {
        f.jmp(fail);
    }
    f.place(not_coin);
    if !secondary {
        let s = f.const_i32(KIND_SECONDARY);
        f.jeq(kind, s, fail);
    }
    let fr = f.const_i32(KIND_FROG);
    f.jne(kind, fr, not_frog);
    if frogs {
        // the frog the player steers in frog mode: deleting it loses the level, leave it to vanilla
        let frog_mode = f.get_new(sheet, "frogMode")?;
        f.jfalse(frog_mode, kinds_ok);
        f.jnull(physics, fail);
        let owner = f.get_new(physics, "owner")?;
        let frog = f.cast(owner, frog_t);
        f.jnull(frog, kinds_ok);
        let steered = f.get_new(frog, "playerControlled")?;
        f.jfalse(steered, kinds_ok);
        let fat = f.get_new(frog, "is_fat")?;
        let zero = f.const_f64(0.0);
        f.jeq(fat, zero, fail);
        f.jmp(kinds_ok);
    } else {
        f.jmp(fail);
    }
    f.place(not_frog);
    f.place(kinds_ok);

    if joined {
        let single = f.label();
        wrap_group(&mut f, &w, tick, margin, sheet, physics, fail, single)?;
        f.place(single);
    }
    let otype = if w.trace {
        let string_t = f.code().class("String")?;
        let (t, none, have) = (f.reg(string_t), f.label(), f.label());
        f.op(Opcode::Null { dst: t });
        f.jnull(physics, none);
        let owner = f.get_new(physics, "owner")?;
        f.jnull(owner, none);
        f.get(t, owner, "type")?;
        f.jmp(have);
        f.place(none);
        f.place(have);
        Some(t)
    } else {
        None
    };
    wrap_one(&mut f, &w, tick, pos, ew, margin, sheet, Some(kind), otype, fail)?;
    f.place(fail);
    ret_bool(&mut f, false);
    let wrap = f.finish()?;
    hooks::subscribe(code, "edge_exit", wrap)?;

    if player {
        wrap_player(code, &w)?;
    }
    Ok(())
}

/// `screen-wrap/player`, subscribed to the core `player_edge(pos, margin, sheet, player)` hook.
fn wrap_player(code: &mut Code, w: &Wrap) -> Result<()> {
    let mut f = hooks::handler(code, "player_edge", "screen-wrap/player")?;
    let (p, m, sh) = (f.arg(0), f.arg(1), f.arg(2));
    let fail = f.label();
    // The hub (layout `help`): walking off its right edge is how a run starts.
    let (layout, go) = (f.get_new(sh, "layout")?, f.label());
    let name = f.get_new(layout, "name")?;
    f.jstr_ne(name, "help", go)?;
    f.jmp(fail);
    f.place(go);
    let tick = guards(&mut f, w, sh, fail)?;
    let ew = f.const_f64(0.0);
    wrap_one(&mut f, w, tick, p, ew, m, sh, None, None, fail)?;
    f.place(fail);
    ret_bool(&mut f, false);
    let guard = f.finish()?;
    hooks::subscribe(code, "player_edge", guard)
}

/// Jump to `fail` unless the mod applies now: the modifier is active (if rolled) and the layout is
/// past `min_tick`. Returns the layout's tick.
fn guards(f: &mut FnBuilder, w: &Wrap, sheet: Reg, fail: Label) -> Result<Reg> {
    if let Some(id) = w.modifier {
        let active = modifiers::is_active(f, id)?;
        f.jfalse(active, fail);
    }
    let layout = f.get_new(sheet, "layout")?;
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(w.min_tick);
    f.jlt(tick, min, fail);
    Ok(tick)
}

fn ret_bool(f: &mut FnBuilder, v: bool) {
    let bool_t = f.code().ty_bool();
    let r = f.reg(bool_t);
    f.bool(r, v);
    f.ret(r);
}

/// Where (`x`, `y`) wraps to: `(nx, ny, moved)`. Same bounds as vanilla: lo = margin - ew,
/// hi = size - margin + ew (x only outside boss mode). Jumps to `fail` when it is more than
/// `max_overshoot` past an edge.
#[allow(clippy::too_many_arguments)]
fn wrapped(
    f: &mut FnBuilder,
    w: &Wrap,
    x: Reg,
    y: Reg,
    ew: Reg,
    margin: Reg,
    sheet: Reg,
    fail: Label,
) -> Result<(Reg, Reg, Reg)> {
    let (lo, hi, span, over) = (f.reg_f64(), f.reg_f64(), f.reg_f64(), f.reg_f64());
    let (nx, ny) = (f.reg_f64(), f.reg_f64());
    let moved = f.reg_bool();
    let max = f.const_f64(w.max_overshoot);
    f.bool(moved, false);
    f.mov(nx, x);
    f.mov(ny, y);
    for (v, nv, size) in [(y, ny, SCREEN_H), (x, nx, SCREEN_W)] {
        let (past_hi, past_lo, done) = (f.label(), f.label(), f.label());
        if size == SCREEN_W {
            let boss = f.get_new(sheet, "bossMode")?;
            f.jtrue(boss, done);
        }
        f.float(hi, size);
        f.sub(hi, hi, margin);
        f.add(hi, hi, ew);
        f.mov(lo, margin);
        f.sub(lo, lo, ew);
        f.mov(span, hi);
        f.sub(span, span, lo);
        f.jlt(hi, v, past_hi);
        f.jlt(v, lo, past_lo);
        f.jmp(done);

        f.place(past_hi); // v > hi
        f.mov(over, v);
        f.sub(over, over, hi);
        f.jlt(max, over, fail);
        f.sub(nv, v, span);
        f.bool(moved, true);
        f.jmp(done);

        f.place(past_lo); // v < lo
        f.mov(over, lo);
        f.sub(over, over, v);
        f.jlt(max, over, fail);
        f.add(nv, v, span);
        f.bool(moved, true);
        f.place(done);
    }
    Ok((nx, ny, moved))
}

/// Wrap `pos` if it is past the screen's edge by at most `max_overshoot` and return true; jump to
/// `fail` (return false, vanilla goes on) otherwise. `kind` is printed by `trace`; None = the player.
#[allow(clippy::too_many_arguments)]
fn wrap_one(
    f: &mut FnBuilder,
    w: &Wrap,
    tick: Reg,
    pos: Reg,
    ew: Reg,
    margin: Reg,
    sheet: Reg,
    kind: Option<Reg>,
    otype: Option<Reg>,
    fail: Label,
) -> Result<()> {
    let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
    let (nx, ny, moved) = wrapped(f, w, x, y, ew, margin, sheet, fail)?;
    f.jfalse(moved, fail);
    f.set(pos, "x", nx)?;
    f.set(pos, "y", ny)?;
    if w.trace {
        let p = Print::Str;
        let who = match kind {
            Some(k) => vec![p(" kind "), Print::Val(k)],
            None => vec![p(" player")],
        };
        let mut line = vec![p("[screen-wrap] tick "), Print::Val(tick)];
        line.extend(who);
        line.extend([
            p(" ("),
            Print::Val(x),
            p(", "),
            Print::Val(y),
            p(") -> ("),
            Print::Val(nx),
            p(", "),
            Print::Val(ny),
            p(")"),
        ]);
        if let Some(t) = otype {
            line.extend([p(" "), Print::Val(t)]);
        }
        f.print(&line)?;
    }
    ret_bool(f, true);
    Ok(())
}

/// The object leaving is joined to others: wrap the whole group, or keep the piece while the
/// group's centre is on screen (returns true either way), or `fail` (vanilla) when the group is
/// held by something that can't move. Jumps to `single` when the object has no joints.
#[allow(clippy::too_many_arguments)]
fn wrap_group(
    f: &mut FnBuilder,
    w: &Wrap,
    tick: Reg,
    margin: Reg,
    sheet: Reg,
    physics: Reg,
    fail: Label,
    single: Label,
) -> Result<()> {
    let code = f.code();
    let joint_list = code.native("body_get_joint_list")?;
    let body_a = code.native("joint_get_body_a")?;
    let body_b = code.native("joint_get_body_b")?;
    let body_type = code.native("body_get_type")?;
    let alloc_array = code.native("alloc_array")?;
    let obj_t = code.class("fish.system.ObjectClass")?;

    f.jnull(physics, single);
    let body = f.get_new(physics, "body")?;
    f.jnull(body, single);
    let first = f.call_new(joint_list, &[body])?;
    let joint_list_t = f.reg_type(first);
    f.jnull(first, single);
    let owner = f.get_new(physics, "owner")?;
    f.jnull(owner, fail);

    // Breadth-first over the joints: members[0..n].
    let ty = f.type_value(obj_t);
    let cap = f.const_i32(MAX_GROUP);
    let members = f.call_new(alloc_array, &[ty, cap])?;
    let (n, i, zero) = (f.reg_i32(), f.reg_i32(), f.const_i32(0));
    f.op(Opcode::SetArray { array: members, index: zero, src: owner });
    f.int(n, 1);
    f.int(i, 0);
    let held = f.label(); // something that can't move holds the group
    let (outer, collected) = (f.label(), f.label());
    let m = f.reg(obj_t);
    let edge = f.reg(joint_list_t);
    f.place(outer);
    f.jge(i, n, collected);
    f.op(Opcode::GetArray { dst: m, array: members, index: i });
    f.op(Opcode::Incr { dst: i });
    let mphys = f.get_new(m, "physics")?;
    f.jnull(mphys, outer);
    let mbody = f.get_new(mphys, "body")?;
    f.jnull(mbody, outer);
    f.call(edge, joint_list, &[mbody]);
    let inner = f.label();
    f.place(inner);
    f.jnull(edge, outer);
    let joint = f.get_new(edge, "joint")?;
    let next = f.get_new(edge, "next")?;
    f.mov(edge, next);
    f.jnull(joint, inner);
    for end in [body_a, body_b] {
        let skip = f.label();
        let other = f.call_new(end, &[joint])?;
        f.jnull(other, skip);
        let t = f.call_new(body_type, &[other])?;
        let dynamic = f.const_i32(DYNAMIC_BODY);
        f.jne(t, dynamic, held);
        let o = owner_of(f, sheet, other, held)?;
        let otype = f.get_new(o, "type")?;
        let not_player = f.label();
        f.jstr_ne(otype, "player", not_player)?;
        f.jmp(held);
        f.place(not_player);
        // already a member?
        let (k, scan, add) = (f.reg_i32(), f.label(), f.label());
        let km = f.reg(obj_t);
        f.int(k, 0);
        f.place(scan);
        f.jge(k, n, add);
        f.op(Opcode::GetArray { dst: km, array: members, index: k });
        f.op(Opcode::Incr { dst: k });
        f.jeq(km, o, skip);
        f.jmp(scan);
        f.place(add);
        f.jge(n, cap, held);
        f.op(Opcode::SetArray { array: members, index: n, src: o });
        f.op(Opcode::Incr { dst: n });
        f.place(skip);
    }
    f.jmp(inner);

    // The centre of the group.
    f.place(collected);
    let (sx, sy, cx, cy, nf) = (f.reg_f64(), f.reg_f64(), f.reg_f64(), f.reg_f64(), f.reg_f64());
    f.float(sx, 0.0);
    f.float(sy, 0.0);
    f.for_range(n, |f, k| {
        let o = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: o, array: members, index: k });
        let p = f.get_new(o, "sprite")?;
        let p = f.get_new(p, "position")?;
        let (x, y) = (f.get_new(p, "x")?, f.get_new(p, "y")?);
        f.add(sx, sx, x);
        f.add(sy, sy, y);
        Ok(())
    })?;
    f.op(Opcode::ToSFloat { dst: nf, src: n });
    f.op(Opcode::SDiv { dst: cx, a: sx, b: nf });
    f.op(Opcode::SDiv { dst: cy, a: sy, b: nf });
    let no_slack = f.const_f64(0.0);
    let (nx, ny, moved) = wrapped(f, w, cx, cy, no_slack, margin, sheet, fail)?;
    let shift = f.label();
    f.jtrue(moved, shift);
    // the centre is still on screen: keep the piece, the group stays together
    if w.trace {
        let otype = f.get_new(owner, "type")?;
        let first_member = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: first_member, array: members, index: zero });
        let k1 = f.const_i32(1);
        let other = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: other, array: members, index: k1 });
        let other_t = f.get_new(other, "type")?;
        f.print(&[
            Print::Str("[screen-wrap] tick "),
            Print::Val(tick),
            Print::Str(" "),
            Print::Val(otype),
            Print::Str(" kept: its group of "),
            Print::Val(n),
            Print::Str(" (with "),
            Print::Val(other_t),
            Print::Str(") is still on screen"),
        ])?;
    }
    ret_bool(f, true);

    // Move every piece by the same offset.
    f.place(shift);
    let (dx, dy) = (f.reg_f64(), f.reg_f64());
    f.sub(dx, nx, cx);
    f.sub(dy, ny, cy);
    f.for_range(n, |f, k| {
        let o = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: o, array: members, index: k });
        let p = f.get_new(o, "sprite")?;
        let p = f.get_new(p, "position")?;
        let (x, y) = (f.get_new(p, "x")?, f.get_new(p, "y")?);
        f.add(x, x, dx);
        f.add(y, y, dy);
        f.set(p, "x", x)?;
        f.set(p, "y", y)?;
        Ok(())
    })?;
    if w.trace {
        let otype = f.get_new(owner, "type")?;
        f.print(&[
            Print::Str("[screen-wrap] tick "),
            Print::Val(tick),
            Print::Str(" group of "),
            Print::Val(n),
            Print::Str(" with "),
            Print::Val(otype),
            Print::Str(", centre ("),
            Print::Val(cx),
            Print::Str(", "),
            Print::Val(cy),
            Print::Str(") -> ("),
            Print::Val(nx),
            Print::Str(", "),
            Print::Val(ny),
            Print::Str(")"),
        ])?;
    }
    ret_bool(f, true);

    f.place(held);
    if w.trace {
        let otype = f.get_new(owner, "type")?;
        f.print(&[
            Print::Str("[screen-wrap] tick "),
            Print::Val(tick),
            Print::Str(" "),
            Print::Val(otype),
            Print::Str(" is held by something that can't move: left to vanilla"),
        ])?;
    }
    f.jmp(fail);
    Ok(())
}

/// The object whose physics body is `body`, among `physics_obj` and `secondary_physics`; jumps to
/// `none` when there is none.
fn owner_of(f: &mut FnBuilder, sheet: Reg, body: Reg, none: Label) -> Result<Reg> {
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    let found = f.reg(obj_t);
    let done = f.label();
    for picker in ["physics_obj", "secondary_physics"] {
        let next_picker = f.label();
        let p = f.get_new(sheet, picker)?;
        f.jnull(p, next_picker);
        let insts = f.get_new(p, "insts")?;
        f.jnull(insts, next_picker);
        let len = f.array_len(insts)?;
        f.for_range(len, |f, k| {
            let miss = f.label();
            let o = f.array_get(insts, k, obj_t)?;
            f.jnull(o, miss);
            let ph = f.get_new(o, "physics")?;
            f.jnull(ph, miss);
            let b = f.get_new(ph, "body")?;
            f.jne(b, body, miss);
            f.mov(found, o);
            f.jmp(done);
            f.place(miss);
            Ok(())
        })?;
        f.place(next_picker);
    }
    f.jmp(none);
    f.place(done);
    Ok(found)
}
