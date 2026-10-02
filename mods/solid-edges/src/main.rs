//! `solid-edges`: a modifier. In levels that roll it, the screen border is a wall: objects bounce
//! off it instead of leaving the screen and being deleted (idea by a friend of the project).
//!
//! It registers a modifier (`openlina_sdk::modifiers`, key `solid-edges`) with its own HUD icon
//! (`assets/images/openlina/solid-edges.png`, from `art/modifier.toml`), rolled like the vanilla
//! modifiers. With the option `always`, it applies in every level instead.
//!
//! With `screen-wrap` in the same pack (the other border rule): a level has one modifier, so two
//! rolled ones never meet. An `always` one steps aside in levels that roll the other (a rolled
//! solid-edges pushes objects back before screen-wrap sees them leave; an `always` solid-edges
//! skips levels that rolled screen-wrap). Both `always` is contradictory: mod.toml declares it as a
//! `[[conflict]]`, so builds, `openlina set` and the website refuse it with the reason.
//!
//! Vanilla deletes objects once they are well past the border (`EvSheet_gameplay.update`, see
//! `mods/core`). This mod never lets them get there: on every `tick`, each object of
//! `physics_obj` and `secondary_physics` whose box (sprite size around its position) crosses the
//! visible play field `[margin, 600 - margin] x [margin, 338 - margin]` (margin 25) is pushed back
//! inside, and the velocity component into the wall is reflected, scaled by `bounce`. Frogs too
//! (always): the level's `frog`s and the frog item's `s_frog` bounce instead of being lost; in frog
//! mode that also keeps the frog you steer (losing it at the edge loses the level).
//! Moving `sprite.position` teleports the Box2D body (`Physics.syncPosWithSprite`).
//!
//! Lina (option `player`): pushed back like an object in levels, so she can't fall or walk off the
//! screen; the core `player_edge` hook keeps her alive if a step still carried her past the edge
//! line within one tick. The hub (layout `help`) and the other non-level layouts stay vanilla:
//! walking off the hub's right edge is how a run starts.
//!
//! Left alone:
//! - Lina unless `player` (she dies at the edge as in vanilla), and fruits unless `coins`
//!   (pushing a fruit off the screen collects it; a fruit that bounces back can only be collected
//!   by touching it)
//! - static bodies (`physics.immovable`), destroyed sprites
//! - the first ticks of a layout and objects far off-screen: levels place objects off-screen and
//!   the game parks objects at e.g. -1000,-1000; those stay vanilla (deleted)
//! - x in long levels and boss arenas (`bossMode`): they scroll horizontally, and vanilla only
//!   tests top and bottom there too

use anyhow::{Context, Result};
use openlina_sdk::asm::{FnBuilder, Label, Print};
use openlina_sdk::hlbc::types::{RefType, Reg};
use openlina_sdk::modifiers::{self, Modifier};
use openlina_sdk::{hooks, world, Code, ModConfig};

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
    friction: f64,
    rest_speed: f64,
    coins: bool,
    player: bool,
    trace: bool,
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let o = Opts {
        bounce: cfg.f64("bounce", 0.5)?,
        friction: cfg.f64("friction", 1.0)?,
        rest_speed: cfg.f64("rest_speed", 40.0)?,
        coins: cfg.bool("coins", false)?,
        player: cfg.bool("player", false)?,
        trace: cfg.bool("trace", false)?,
    };
    let always = cfg.bool("always", false)?;
    // screen-wrap in the same pack: rolled (Some(false)), always on (Some(true)) or absent.
    let wrap_always = openlina_sdk::runner::pack_info()?
        .mods
        .iter()
        .find(|m| m.id == "screen-wrap")
        .map(|m| m.options.get("always").and_then(|v| v.as_bool()).unwrap_or(false));
    // Both `always` is refused before any mod runs: `[[conflict]]` in mod.toml.
    let modifier = if always {
        None
    } else {
        Some(modifiers::register(
            code,
            &Modifier { key: "solid-edges", icon: "images/openlina/solid-edges.png", size: (16.0, 16.0), in_dx: true },
        )?)
    };

    // Element types of the two pickers, as vanilla's update uses them.
    let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
    let fun = code.func(update)?.clone();
    let elem_type = |code: &Code, field: &str| -> Result<RefType> {
        let at =
            openlina_sdk::edit::expect_one(openlina_sdk::edit::find_field_access(code, &fun, field, false), field)?;
        let Some(i) = openlina_sdk::edit::next_match(&fun, at, |op| {
            matches!(op, openlina_sdk::hlbc::opcodes::Opcode::ToVirtual { .. })
        }) else {
            anyhow::bail!("no element cast after `{field}`")
        };
        let openlina_sdk::hlbc::opcodes::Opcode::ToVirtual { dst, .. } = fun.ops[i] else { unreachable!() };
        Ok(fun.regs[dst.0 as usize])
    };
    // `physics_obj` is read in many places; its element type is the type of the object whose
    // `edgewith` the edge test reads (a unique anchor, the same one `core` uses).
    let ew = openlina_sdk::edit::expect_one(
        openlina_sdk::edit::find_field_access(code, &fun, "edgewith", false),
        "`.edgewith` read",
    )?;
    let openlina_sdk::hlbc::opcodes::Opcode::Field { obj: item, .. } = fun.ops[ew] else {
        anyhow::bail!("`.edgewith` read is not a Field")
    };
    let obj_t = fun.regs[item.0 as usize];
    let sec_t = elem_type(code, "secondary_physics").context("secondary_physics element type")?;

    let mut f = hooks::handler(code, "tick", "solid-edges/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    jump_unless_active(&mut f, modifier, wrap_always, end)?;
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(MIN_TICK);
    f.jlt(tick, min, end);
    let boss = f.get_new(sheet, "bossMode")?;
    // Lina only in levels: walking off the hub's right edge is how a run starts
    let level = world::is_level(&mut f, layout)?;
    for (picker, t) in [("physics_obj", obj_t), ("secondary_physics", sec_t)] {
        let p = f.get_new(sheet, picker)?;
        let insts = f.get_new(p, "insts")?;
        let n = f.array_len(insts)?;
        f.for_range(n, |f, i| {
            let next = f.label();
            let obj = f.array_get(insts, i, t)?;
            collide(f, &o, obj, boss, level, tick, next)?;
            f.place(next);
            Ok(())
        })?;
    }
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)?;
    if o.player {
        keep_player(code, modifier, wrap_always)?;
    }
    Ok(())
}

/// Jump to `end` unless the border is solid now: the modifier rolled (`modifier` Some), or always
/// on but not in a level that rolled screen-wrap (when screen-wrap is in the pack, rolled).
fn jump_unless_active(f: &mut FnBuilder, modifier: Option<i32>, wrap_always: Option<bool>, end: Label) -> Result<()> {
    match modifier {
        // Only in levels that rolled the modifier.
        Some(id) => {
            let active = modifiers::is_active(f, id)?;
            f.jfalse(active, end);
        }
        // Always on, but not in levels that rolled screen-wrap.
        None if wrap_always == Some(false) => {
            let wrap = modifiers::is_active(f, modifiers::id_of("screen-wrap"))?;
            f.jtrue(wrap, end);
        }
        None => {}
    }
    Ok(())
}

/// `player`: the core `player_edge` hook (Lina about to die at the edge) keeps her alive while the
/// border is solid, in levels. The tick handler pushes her back inside first, so this only catches
/// a step that carried her past the line within one tick.
fn keep_player(code: &mut Code, modifier: Option<i32>, wrap_always: Option<bool>) -> Result<()> {
    let mut f = hooks::handler(code, "player_edge", "solid-edges/player")?;
    let sheet = f.arg(2);
    let vanilla = f.label();
    jump_unless_active(&mut f, modifier, wrap_always, vanilla)?;
    let layout = f.get_new(sheet, "layout")?;
    world::jump_unless_level(&mut f, layout, vanilla)?;
    let bool_t = f.code().ty_bool();
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.ret(yes);
    f.place(vanilla);
    let no = f.reg(bool_t);
    f.bool(no, false);
    f.ret(no);
    let h = f.finish()?;
    hooks::subscribe(code, "player_edge", h)
}

/// Push `obj` back inside the play field and reflect its velocity.
#[allow(clippy::too_many_arguments)]
fn collide(f: &mut FnBuilder, o: &Opts, obj: Reg, boss: Reg, level: Reg, tick: Reg, next: Label) -> Result<()> {
    let (f64_t, bool_t) = (f.code().ty_f64(), f.code().ty_bool());
    let get_vx = f.code().method("fish.system.beh.Physics", "getVelocityX")?;
    let get_vy = f.code().method("fish.system.beh.Physics", "getVelocityY")?;
    let set_v = f.code().method("fish.system.beh.Physics", "setVelocity")?;

    f.jnull(obj, next);
    let ty = f.get_new(obj, "type")?;
    for skip in (!o.coins).then_some("coin").into_iter().chain((!o.player).then_some("player")) {
        let other = f.label();
        f.jstr_ne(ty, skip, other)?;
        f.jmp(next);
        f.place(other);
    }
    if o.player {
        // Lina: only in levels
        let other = f.label();
        f.jstr_ne(ty, "player", other)?;
        f.jfalse(level, next);
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
    // A real bounce (fast impact) vs. resting contact (gravity pressing into the wall each tick).
    let bounced = f.reg(bool_t);
    f.bool(bounced, false);
    let bounce = f.const_f64(-o.bounce);
    let friction = f.const_f64(o.friction);
    let rest = f.const_f64(o.rest_speed);
    let neg_rest = f.const_f64(-o.rest_speed);
    let zero = f.const_f64(0.0);
    let far = f.const_f64(FAR);

    // One axis: keep `v` (center) within [lo + half, hi - half]. Motion into the wall is
    // reflected (scaled by `bounce`) or, if slow, stopped; motion along the wall (`along`) gets
    // `friction`.
    let axis = |f: &mut FnBuilder, v: Reg, half: Reg, vel: Reg, along: Reg, lo: f64, hi: f64| -> Result<()> {
        let (lo_r, hi_r) = (f.const_f64(lo), f.const_f64(hi));
        let (min, max, d) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
        f.add(min, lo_r, half);
        f.sub(max, hi_r, half);
        let (past_max, past_min, contact, stop, done) = (f.label(), f.label(), f.label(), f.label(), f.label());
        f.jlt(max, v, past_max);
        f.jlt(v, min, past_min);
        f.jmp(done);

        f.place(past_max);
        f.sub(d, v, max);
        f.jlt(far, d, next); // far outside: the game's own business
        f.mov(v, max);
        f.jle(vel, zero, contact); // already moving away
        f.jlt(vel, rest, stop); // slow: come to rest
        f.mul(vel, vel, bounce);
        f.bool(bounced, true);
        f.jmp(contact);

        f.place(past_min);
        f.sub(d, min, v);
        f.jlt(far, d, next);
        f.mov(v, min);
        f.jge(vel, zero, contact);
        f.jlt(neg_rest, vel, stop);
        f.mul(vel, vel, bounce);
        f.bool(bounced, true);
        f.jmp(contact);

        f.place(stop);
        f.mov(vel, zero);
        f.place(contact);
        f.bool(moved, true);
        f.mul(along, along, friction);
        f.place(done);
        Ok(())
    };
    axis(f, y, hh, vy, vx, MARGIN, SCREEN_H - MARGIN)?;
    let skip_x = f.label();
    f.jtrue(boss, skip_x);
    axis(f, x, hw, vx, vy, MARGIN, SCREEN_W - MARGIN)?;
    f.place(skip_x);

    f.jfalse(moved, next);
    f.set(pos, "x", x)?;
    f.set(pos, "y", y)?;
    f.call_new(set_v, &[physics, vx, vy])?;
    if o.trace {
        f.jfalse(bounced, next);
        f.print(&[
            Print::Str("[solid-edges] tick "),
            Print::Val(tick),
            Print::Str(" "),
            Print::Val(ty),
            Print::Str(" at ("),
            Print::Val(x),
            Print::Str(", "),
            Print::Val(y),
            Print::Str(") v ("),
            Print::Val(vx),
            Print::Str(", "),
            Print::Val(vy),
            Print::Str(")"),
        ])?;
    }
    Ok(())
}
