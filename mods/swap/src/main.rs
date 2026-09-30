//! `swap`: a new item, **Swap**. Firing it casts a ray from Lina along her aim; the first object
//! the ray hits trades places with her: Lina goes where the object was, the object goes where Lina
//! was. If nothing is hit, nothing happens (the shot is still used).
//!
//! Vanilla: items are dispatched by name in `EvSheet_gameplay.shoot` after the ammo was
//! decremented; nothing in the game swaps positions.
//!
//! What the mod adds:
//! - The item type `swap` (pool entry, HUD icon `assets/images/openlina/swap.png`, label "Swap")
//!   through `openlina_sdk::items::register`, with `ammo` shots and the aim type `aim`.
//! - Firing: an `item_use` handler. The crosshair (`items::crosshair_pos`) only gives the
//!   direction: the ray goes from Lina's position through the crosshair, `range` layout units
//!   long. Returning true skips the game's own item behaviors, also when nothing is hit.
//! - The ray is the game's own Box2D ray cast, the native `world_ray_cast(world, callback, from,
//!   to)` that the Construct "Line of Sight" behavior (`fish.system.beh.LOS.castRay`) uses, in
//!   physics units (layout units × `layout.worldScale`). The callback (`swap/ray_hit`, a static
//!   closure; its state lives in globals) sees every fixture on the ray in any order; it ignores
//!   Lina's own fixtures and returns the hit's fraction, which clips the ray, so the last fixture
//!   it records is the nearest one. The fixture's user data is its `ObjectClass`.
//! - Walls: with `walls_block` (default) static bodies (`physics.immovable`: tiles, level
//!   geometry) take part in the ray and stop it: when the nearest hit is static, nothing is
//!   swapped. Without it the ray passes through static bodies and only movable objects count.
//! - Swapping writes both `sprite.position`s; `Physics.syncPosWithSprite` then moves the Box2D
//!   bodies (a teleport). Velocities are kept, like the portal gun's teleports.
//! - `trace` prints every shot (`[swap] tick T: player (x, y) <-> box (x, y)`, `nothing in line of
//!   sight`, `blocked by <type>`, each with the ammo left) and, `check_ticks` later, where both
//!   really are compared with the other's old spot (`[swap] check: …`), read back from the game
//!   after the physics ran, so tests can see that the swap held.
//!
//! Left alone: every other item, ammo handling (the game decrements before `item_use`), what
//! counts as an object (anything with a Box2D fixture: boxes, fruits, frogs, enemies, the other
//! player in co-op), collisions after the swap (an object can land overlapping geometry and be
//! pushed out by Box2D, as with any teleport).

use anyhow::{bail, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefFun, RefGlobal, Reg};
use openlina_sdk::items::{self, Aim, Item};
use openlina_sdk::{hooks, Code, ModConfig};

const ITEM: &str = "swap";
/// A swap check counts as "at the old spot" within this distance (layout units).
const NEAR: f64 = 6.0;

fn main() {
    openlina_sdk::run_mod(apply)
}

struct Opts {
    range: f64,
    walls_block: bool,
    trace: bool,
    check_ticks: i32,
}

/// Mod state, kept in globals.
struct State {
    /// The ray: who fired it (ignored by the callback), the nearest hit so far and its fraction.
    shooter: RefGlobal,
    best: RefGlobal,
    frac: RefGlobal,
    /// The last swap, for the trace check: tick, both objects and their old positions.
    swap_tick: RefGlobal,
    swapped: RefGlobal,
    player: RefGlobal,
    old: [RefGlobal; 4],
}

fn parse_aim(s: &str) -> Result<Aim> {
    Ok(match s {
        "shoot" => Aim::Shoot,
        "short" => Aim::Short,
        "short2" => Aim::Short2,
        "mid" => Aim::Mid,
        "mid2" => Aim::Mid2,
        "long" => Aim::Long,
        "long2" => Aim::Long2,
        "remote" => Aim::Remote,
        other => bail!("aim `{other}`: expected shoot, short, short2, mid, mid2, long, long2 or remote"),
    })
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let ammo = cfg.i64("ammo", 3)?;
    if !(0..=99).contains(&ammo) {
        bail!("ammo must be 0..99, got {ammo}");
    }
    let aim = parse_aim(cfg.str("aim", "long")?)?;
    let o = Opts {
        range: cfg.f64("range", 400.0)?,
        walls_block: cfg.bool("walls_block", true)?,
        trace: cfg.bool("trace", false)?,
        check_ticks: cfg.i64("check_ticks", 12)? as i32,
    };
    if o.range <= 0.0 {
        bail!("range must be > 0, got {}", o.range);
    }

    items::register(
        code,
        &Item { name: ITEM, label: "Swap", ammo: ammo as i32, aim, second_layer: false, icon: "images/openlina/swap.png", icon_size: (12.0, 12.0) },
    )?;

    let (f64_t, i32_t) = (code.ty_f64(), code.ty_i32());
    let obj_t = code.class("fish.system.ObjectClass")?;
    let g = |code: &mut Code, t| code.add_global(t);
    let st = State {
        shooter: g(code, obj_t),
        best: g(code, obj_t),
        frac: g(code, f64_t),
        swap_tick: g(code, i32_t),
        swapped: g(code, obj_t),
        player: g(code, obj_t),
        old: [g(code, f64_t), g(code, f64_t), g(code, f64_t), g(code, f64_t)],
    };
    let hit = build_ray_hit(code, &st, &o)?;
    build_use(code, &st, hit, &o)?;
    if o.trace {
        build_check(code, &st, &o)?;
    }
    Ok(())
}

fn get_g(f: &mut FnBuilder, g: RefGlobal) -> Reg {
    let t = f.code().bc.globals[g.0];
    let r = f.reg(t);
    f.op(Opcode::GetGlobal { dst: r, global: g });
    r
}

fn set_g(f: &mut FnBuilder, g: RefGlobal, v: Reg) {
    f.op(Opcode::SetGlobal { global: g, src: v });
}

fn set_null(f: &mut FnBuilder, g: RefGlobal) {
    let t = f.code().bc.globals[g.0];
    let n = f.reg(t);
    f.op(Opcode::Null { dst: n });
    set_g(f, g, n);
}

/// `swap/ray_hit(fixture, point, normal, fraction) -> F64`: the `world_ray_cast` callback.
/// Box2D semantics: return -1 to ignore the fixture, the fraction to clip the ray there.
fn build_ray_hit(code: &mut Code, st: &State, o: &Opts) -> Result<RefFun> {
    let cast = code.native("world_ray_cast")?;
    let cb_t = code.func_type(cast)?.args[1];
    let Some(cb) = cb_t.as_fun(&code.bc).cloned() else { bail!("world_ray_cast: argument 2 is not a function type") };
    // (b2Fixture, Vector2Default point, Vector2Default normal, F64 fraction) -> F64
    anyhow::ensure!(cb.args.len() == 4 && cb.ret == code.ty_f64(), "world_ray_cast callback: unexpected signature {}", code.type_name(cb_t));
    let user_data = code.native("fixture_get_user_data")?;
    let obj_t = code.class("fish.system.ObjectClass")?;

    let mut f = FnBuilder::new(code, "swap/ray_hit", &cb.args, cb.ret);
    let (fixture, fraction) = (f.arg(0), f.arg(3));
    let ignore = f.label();
    let ud = f.call_new(user_data, &[fixture])?;
    let obj = f.cast(ud, obj_t);
    f.jnull(obj, ignore);
    let shooter = get_g(&mut f, st.shooter);
    f.jeq(obj, shooter, ignore);
    let sprite = f.get_new(obj, "sprite")?;
    f.jnull(sprite, ignore);
    let destroyed = f.get_new(sprite, "destroyed")?;
    f.jtrue(destroyed, ignore);
    if !o.walls_block {
        let physics = f.get_new(obj, "physics")?;
        f.jnull(physics, ignore);
        let immovable = f.get_new(physics, "immovable")?;
        f.jtrue(immovable, ignore);
    }
    set_g(&mut f, st.best, obj);
    set_g(&mut f, st.frac, fraction);
    f.ret(fraction);
    f.place(ignore);
    let m1 = f.const_f64(-1.0);
    f.ret(m1);
    f.finish()
}

/// `item_use`: fire Swap.
fn build_use(code: &mut Code, st: &State, hit: RefFun, o: &Opts) -> Result<()> {
    let (bool_t, f64_t) = (code.ty_bool(), code.ty_f64());
    let first = code.method("fish.system.Picker", "first")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let obj_t = code.class("fish.system.ObjectClass")?;
    let sqrt = code.native("math_sqrt")?;
    let ray_cast = code.native("world_ray_cast")?;
    let cb_t = code.func_type(ray_cast)?.args[1];
    let v2_new = code.method("hxmath.math.Vector2Default", "__constructor__")?;

    let mut f = hooks::handler(code, "item_use", "swap/use")?;
    let (slot, sheet, player_picker, cross) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3));
    let (not_mine, handled) = (f.label(), f.label());
    items::is_item(&mut f, slot, ITEM, not_mine)?;
    let (cx, cy) = items::crosshair_pos(&mut f, cross, handled)?;
    f.jnull(player_picker, handled);
    let p_dyn = f.call_new(first, &[player_picker])?;
    let player = f.cast(p_dyn, player_t);
    f.jnull(player, handled);
    let player = f.cast(player, obj_t);
    let psprite = f.get_new(player, "sprite")?;
    let ppos = f.get_new(psprite, "position")?;
    let (px, py) = (f.get_new(ppos, "x")?, f.get_new(ppos, "y")?);

    // direction Lina -> crosshair, `range` long
    let (dx, dy, d2, t, len) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    f.sub(dx, cx, px);
    f.sub(dy, cy, py);
    f.mul(d2, dx, dx);
    f.mul(t, dy, dy);
    f.add(d2, d2, t);
    let eps = f.const_f64(0.01);
    f.jle(d2, eps, handled);
    f.call(len, sqrt, &[d2]);
    let range = f.const_f64(o.range);
    let (tx, ty) = (f.reg(f64_t), f.reg(f64_t));
    f.op(Opcode::SDiv { dst: tx, a: dx, b: len });
    f.mul(tx, tx, range);
    f.add(tx, tx, px);
    f.op(Opcode::SDiv { dst: ty, a: dy, b: len });
    f.mul(ty, ty, range);
    f.add(ty, ty, py);

    // the ray in physics units
    let layout = f.get_new(sheet, "layout")?;
    let ws = f.get_new(layout, "worldScale")?;
    let world = f.get_new(layout, "world")?;
    f.jnull(world, handled);
    let v2_t = f.code().func_type(v2_new)?.args[0];
    let vec = |f: &mut FnBuilder, x: Reg, y: Reg| -> Result<Reg> {
        let v = f.reg(v2_t);
        f.op(Opcode::New { dst: v });
        let (sx, sy) = (f.reg(f64_t), f.reg(f64_t));
        f.mul(sx, x, ws);
        f.mul(sy, y, ws);
        f.call_new(v2_new, &[v, sx, sy])?;
        Ok(v)
    };
    let from = vec(&mut f, px, py)?;
    let to = vec(&mut f, tx, ty)?;
    set_g(&mut f, st.shooter, player);
    set_null(&mut f, st.best);
    let one = f.const_f64(1.0);
    set_g(&mut f, st.frac, one);
    let cb = f.reg(cb_t);
    f.op(Opcode::StaticClosure { dst: cb, fun: hit });
    f.call_new(ray_cast, &[world, cb, from, to])?;
    set_null(&mut f, st.shooter);

    let tick = f.get_new(layout, "currentTick")?;
    let ammo = f.get_new(slot, "ammo")?;
    if o.trace {
        f.print(&[
            Print::Str("[swap] tick "), Print::Val(tick), Print::Str(": ray ("), Print::Val(px), Print::Str(", "), Print::Val(py),
            Print::Str(") -> ("), Print::Val(tx), Print::Str(", "), Print::Val(ty), Print::Str(")"),
        ])?;
    }
    let best = get_g(&mut f, st.best);
    let found = f.label();
    f.jnotnull(best, found);
    if o.trace {
        f.print(&[Print::Str("[swap] tick "), Print::Val(tick), Print::Str(": nothing in line of sight (ammo left "), Print::Val(ammo), Print::Str(")")])?;
    }
    f.jmp(handled);
    f.place(found);
    set_null(&mut f, st.best);
    let physics = f.get_new(best, "physics")?;
    f.jnull(physics, handled);
    let btype = f.get_new(best, "type")?;
    if o.walls_block {
        let movable = f.label();
        let immovable = f.get_new(physics, "immovable")?;
        f.jfalse(immovable, movable);
        if o.trace {
            f.print(&[
                Print::Str("[swap] tick "), Print::Val(tick), Print::Str(": blocked by "), Print::Val(btype),
                Print::Str(" (ammo left "), Print::Val(ammo), Print::Str(")"),
            ])?;
        }
        f.jmp(handled);
        f.place(movable);
    }
    // swap the positions
    let bsprite = f.get_new(best, "sprite")?;
    f.jnull(bsprite, handled);
    let bpos = f.get_new(bsprite, "position")?;
    let (bx, by) = (f.get_new(bpos, "x")?, f.get_new(bpos, "y")?);
    f.set(bpos, "x", px)?;
    f.set(bpos, "y", py)?;
    f.set(ppos, "x", bx)?;
    f.set(ppos, "y", by)?;
    if o.trace {
        f.print(&[
            Print::Str("[swap] tick "), Print::Val(tick), Print::Str(": player ("), Print::Val(px), Print::Str(", "), Print::Val(py),
            Print::Str(") <-> "), Print::Val(btype), Print::Str(" ("), Print::Val(bx), Print::Str(", "), Print::Val(by),
            Print::Str(") (ammo left "), Print::Val(ammo), Print::Str(")"),
        ])?;
        set_g(&mut f, st.swap_tick, tick);
        set_g(&mut f, st.swapped, best);
        set_g(&mut f, st.player, player);
        for (g, v) in st.old.iter().zip([px, py, bx, by]) {
            set_g(&mut f, *g, v);
        }
    }

    f.place(handled);
    let t = f.reg(bool_t);
    f.bool(t, true);
    f.ret(t);
    f.place(not_mine);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, "item_use", h)
}

/// `tick` (trace only): `check_ticks` after a swap, print where both objects really are.
fn build_check(code: &mut Code, st: &State, o: &Opts) -> Result<()> {
    let (f64_t, bool_t) = (code.ty_f64(), code.ty_bool());
    let sqrt = code.native("math_sqrt")?;
    let mut f = hooks::handler(code, "tick", "swap/check")?;
    let layout = f.arg(1);
    let end = f.label();
    let tick = f.get_new(layout, "currentTick")?;
    // new level: forget the last swap
    let not_new = f.label();
    let one = f.const_i32(1);
    f.jne(tick, one, not_new);
    set_null(&mut f, st.swapped);
    set_null(&mut f, st.player);
    f.jmp(end);
    f.place(not_new);

    let swapped = get_g(&mut f, st.swapped);
    f.jnull(swapped, end);
    let player = get_g(&mut f, st.player);
    f.jnull(player, end);
    let at = get_g(&mut f, st.swap_tick);
    let ct = f.const_i32(o.check_ticks);
    f.add(at, at, ct);
    f.jne(tick, at, end);
    set_null(&mut f, st.swapped);

    let old: Vec<Reg> = st.old.iter().map(|g| get_g(&mut f, *g)).collect();
    // distance of `obj` now to (x, y); also prints where it is
    let near = f.const_f64(NEAR);
    let report = |f: &mut FnBuilder, who: &str, obj: Reg, x: Reg, y: Reg, whose: &str| -> Result<()> {
        let sprite = f.get_new(obj, "sprite")?;
        let pos = f.get_new(sprite, "position")?;
        let (nx, ny) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
        let (dx, dy, d2, t, d) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
        f.sub(dx, nx, x);
        f.sub(dy, ny, y);
        f.mul(d2, dx, dx);
        f.mul(t, dy, dy);
        f.add(d2, d2, t);
        f.call(d, sqrt, &[d2]);
        let is_near = f.reg(bool_t);
        let (yes, done) = (f.label(), f.label());
        f.jlt(d, near, yes);
        f.bool(is_near, false);
        f.jmp(done);
        f.place(yes);
        f.bool(is_near, true);
        f.place(done);
        let name = f.get_new(obj, "type")?;
        f.print(&[
            Print::Str("[swap] check: "), Print::Str(who), Print::Val(name), Print::Str(" at ("), Print::Val(nx), Print::Str(", "),
            Print::Val(ny), Print::Str(&format!(") near {whose} old spot: ")), Print::Val(is_near), Print::Str(" (distance "),
            Print::Val(d), Print::Str(")"),
        ])?;
        Ok(())
    };
    report(&mut f, "", player, old[2], old[3], "the object's")?;
    report(&mut f, "object ", swapped, old[0], old[1], "the player's")?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}
