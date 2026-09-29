//! `solid-edges`: the screen border is a wall. Objects bounce off it instead of leaving the
//! screen and being deleted. Always on (idea by a friend of the project).
//!
//! Vanilla deletes objects once they are well past the border (`EvSheet_gameplay.update`, see
//! `mods/core`). This mod never lets them get there: on every `tick`, each object of
//! `physics_obj` and `secondary_physics` whose box (sprite size around its position) crosses the
//! visible play field `[margin, 600 - margin] x [margin, 338 - margin]` (margin 25) is pushed back
//! inside, and the velocity component into the wall is reflected, scaled by `bounce`.
//! Moving `sprite.position` teleports the Box2D body (`Physics.syncPosWithSprite`).
//!
//! Left alone:
//! - the player (it still dies at the edge), frogs, and fruits unless `coins` (pushing fruits
//!   out is how levels are won)
//! - static bodies (`physics.immovable`), destroyed sprites
//! - the first ticks of a layout and objects far off-screen: levels place objects off-screen and
//!   the game parks objects at e.g. -1000,-1000; those stay vanilla (deleted)
//! - x in long levels and boss arenas (`bossMode`): they scroll horizontally, and vanilla only
//!   tests top and bottom there too

use anyhow::{Context, Result};
use openlina_sdk::asm::{FnBuilder, Label, Print};
use openlina_sdk::hlbc::types::{RefType, Reg};
use openlina_sdk::{hooks, Code, ModConfig};

const SCREEN_W: f64 = 600.0;
const SCREEN_H: f64 = 338.0;
const MARGIN: f64 = 25.0;
/// Objects further out than this are the game's parked/placed ones: leave them to vanilla.
const FAR: f64 = 200.0;
/// Don't touch anything during the first ticks of a layout (levels delete off-screen objects).
const MIN_TICK: i32 = 5;

fn main() {
    openlina_sdk::run_mod(apply)
}

struct Opts {
    bounce: f64,
    coins: bool,
    trace: bool,
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let o = Opts { bounce: cfg.f64("bounce", 0.5)?, coins: cfg.bool("coins", false)?, trace: cfg.bool("trace", false)? };

    // Element types of the two pickers, as vanilla's update uses them.
    let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
    let fun = code.func(update)?.clone();
    let elem_type = |code: &Code, field: &str| -> Result<RefType> {
        let at = openlina_sdk::edit::expect_one(openlina_sdk::edit::find_field_access(code, &fun, field, false), field)?;
        let Some(i) = openlina_sdk::edit::next_match(&fun, at, |op| matches!(op, openlina_sdk::hlbc::opcodes::Opcode::ToVirtual { .. }))
        else {
            anyhow::bail!("no element cast after `{field}`")
        };
        let openlina_sdk::hlbc::opcodes::Opcode::ToVirtual { dst, .. } = fun.ops[i] else { unreachable!() };
        Ok(fun.regs[dst.0 as usize])
    };
    // `physics_obj` is read in many places; its element type is the type of the object whose
    // `edgewith` the edge test reads (a unique anchor, the same one `core` uses).
    let ew = openlina_sdk::edit::expect_one(openlina_sdk::edit::find_field_access(code, &fun, "edgewith", false), "`.edgewith` read")?;
    let openlina_sdk::hlbc::opcodes::Opcode::Field { obj: item, .. } = fun.ops[ew] else { anyhow::bail!("`.edgewith` read is not a Field") };
    let obj_t = fun.regs[item.0 as usize];
    let sec_t = elem_type(code, "secondary_physics").context("secondary_physics element type")?;

    let mut f = hooks::handler(code, "tick", "solid-edges/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(MIN_TICK);
    f.jlt(tick, min, end);
    let boss = f.get_new(sheet, "bossMode")?;
    for (picker, t) in [("physics_obj", obj_t), ("secondary_physics", sec_t)] {
        let p = f.get_new(sheet, picker)?;
        let insts = f.get_new(p, "insts")?;
        let n = f.array_len(insts)?;
        f.for_range(n, |f, i| {
            let next = f.label();
            let obj = f.array_get(insts, i, t)?;
            collide(f, &o, obj, boss, tick, next)?;
            f.place(next);
            Ok(())
        })?;
    }
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}

/// Push `obj` back inside the play field and reflect its velocity.
fn collide(f: &mut FnBuilder, o: &Opts, obj: Reg, boss: Reg, tick: Reg, next: Label) -> Result<()> {
    let (f64_t, bool_t) = (f.code().ty_f64(), f.code().ty_bool());
    let get_vx = f.code().method("fish.system.beh.Physics", "getVelocityX")?;
    let get_vy = f.code().method("fish.system.beh.Physics", "getVelocityY")?;
    let set_v = f.code().method("fish.system.beh.Physics", "setVelocity")?;

    f.jnull(obj, next);
    let ty = f.get_new(obj, "type")?;
    for skip in ["player", "frog"].into_iter().chain((!o.coins).then_some("coin")) {
        let other = f.label();
        f.jstr_ne(ty, skip, other)?;
        f.jmp(next);
        f.place(other);
    }
    let sprite = f.get_new(obj, "sprite")?;
    f.jnull(sprite, next);
    let destroyed = f.get_new(sprite, "destroyed")?;
    f.jtrue(destroyed, next);
    let physics = f.get_new(obj, "physics")?;
    f.jnull(physics, next);
    let immovable = f.get_new(physics, "immovable")?;
    f.jtrue(immovable, next);

    let pos = f.get_new(sprite, "position")?;
    let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
    let (w, h) = (f.get_new(sprite, "width")?, f.get_new(sprite, "height")?);
    let half = f.const_f64(0.5);
    let (hw, hh) = (f.reg(f64_t), f.reg(f64_t));
    f.mul(hw, w, half);
    f.mul(hh, h, half);
    let vx = f.call_new(get_vx, &[physics])?;
    let vy = f.call_new(get_vy, &[physics])?;
    let moved = f.reg(bool_t);
    f.bool(moved, false);
    let bounce = f.const_f64(-o.bounce);
    let zero = f.const_f64(0.0);
    let far = f.const_f64(FAR);

    // One axis: keep `v` (center) within [lo + half, hi - half], reflecting `vel`.
    let axis = |f: &mut FnBuilder, v: Reg, half: Reg, vel: Reg, lo: f64, hi: f64| -> Result<()> {
        let (lo_r, hi_r) = (f.const_f64(lo), f.const_f64(hi));
        let (min, max, d) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
        f.add(min, lo_r, half);
        f.sub(max, hi_r, half);
        let (past_max, past_min, done) = (f.label(), f.label(), f.label());
        f.jlt(max, v, past_max);
        f.jlt(v, min, past_min);
        f.jmp(done);

        f.place(past_max);
        f.sub(d, v, max);
        f.jlt(far, d, next); // far outside: the game's own business
        f.mov(v, max);
        f.bool(moved, true);
        f.jle(vel, zero, done); // only reflect motion into the wall
        f.mul(vel, vel, bounce);
        f.jmp(done);

        f.place(past_min);
        f.sub(d, min, v);
        f.jlt(far, d, next);
        f.mov(v, min);
        f.bool(moved, true);
        f.jge(vel, zero, done);
        f.mul(vel, vel, bounce);
        f.place(done);
        Ok(())
    };
    axis(f, y, hh, vy, MARGIN, SCREEN_H - MARGIN)?;
    let skip_x = f.label();
    f.jtrue(boss, skip_x);
    axis(f, x, hw, vx, MARGIN, SCREEN_W - MARGIN)?;
    f.place(skip_x);

    f.jfalse(moved, next);
    f.set(pos, "x", x)?;
    f.set(pos, "y", y)?;
    f.call_new(set_v, &[physics, vx, vy])?;
    if o.trace {
        f.print(&[
            Print::Str("[solid-edges] tick "), Print::Val(tick), Print::Str(" "), Print::Val(ty), Print::Str(" at ("),
            Print::Val(x), Print::Str(", "), Print::Val(y), Print::Str(") v ("), Print::Val(vx), Print::Str(", "),
            Print::Val(vy), Print::Str(")"),
        ])?;
    }
    Ok(())
}
