//! `screen-wrap`: objects leaving the screen come back on the opposite side.
//!
//! ## How vanilla handles the edge
//!
//! `EvSheet_gameplay.update` (source `EvSheet_gameplay.hx` ~L16195-16268) runs, every tick:
//!
//! ```haxe
//! var margin = 25;
//! for (p in player.insts) if (outside(p.pos, 0)) player_death(p);        // unchanged
//! for (o in physics_obj.insts) {
//!     if (o.sprite.destroyed || (isStatic(o) && currentTick != 0)) continue;
//!     var pos = o.sprite.position, ew = o.edgewith;
//!     if (pos.y > 338 - margin + ew || pos.y < margin - ew
//!         || (!bossMode && (pos.x < margin - ew || pos.x > 600 - margin + ew)))
//!         switch (o.type) {
//!             case "coin":   coinedgecheck(o);  // collect = win condition  <- patched if `coins`
//!             case "frog":   frogland_count++; destroy();                           (unchanged)
//!             case "player": // handled above
//!             default:       o.sprite.destroy();                                    <- patched
//!         }
//! }
//! for (o in secondary_physics.insts)   // same test without `edgewith`
//!     if (outside(o.sprite.position, 0)) o.sprite.destroy();                        <- patched
//! ```
//!
//! `bossMode` is set for boss arenas and for custom levels with `longMode` (levels wider than
//! the screen that scroll horizontally), which is why the x test is skipped there. The mod
//! keeps that rule: in those levels only top/bottom wrap.
//!
//! ## What the mod does
//!
//! It injects `mosa_screen_wrap(pos, edgewith, margin, sheet): Bool`, which moves `pos` by one
//! play-field size to the opposite side, and guards each patched call with
//! `if (mosa_screen_wrap(...)) skip the call;`. The vanilla call stays in place as a fallback
//! for objects that are far off-screen (the game parks and discards objects at e.g.
//! -1000,-1000; wrapping those would drop them into the level).
//!
//! Moving `sprite.position` is enough: `Physics.syncPosWithSprite` notices the change and
//! teleports the Box2D body, keeping its velocity.

use anyhow::{bail, ensure, Context, Result};
use mosa_bc::asm::{FnBuilder, Print};
use mosa_bc::edit::{
    add_reg, call, call_target, expect_one, find_calls, find_field_access, guard_op, insert_ops_with_exits,
    next_match, prev_match, Exit, Incoming,
};
use mosa_bc::hlbc::opcodes::Opcode;
use mosa_bc::hlbc::types::{Function, RefFun, RefType, Reg};
use mosa_bc::{Code, Mod, ModConfig};

const SHEET: &str = "fish.game.evsheet.EvSheet_gameplay";
/// Play-field size in layout units, hardcoded in the game's edge test.
const SCREEN_W: f64 = 600.0;
const SCREEN_H: f64 = 338.0;

pub struct ScreenWrap;

impl Mod for ScreenWrap {
    fn id(&self) -> &'static str {
        "screen-wrap"
    }

    fn description(&self) -> &'static str {
        "Objects leaving the screen wrap around to the opposite edge instead of being destroyed \
         (top/bottom only in long and boss levels)"
    }

    fn options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("coins", "false", "wrap coins (fruits) too. Pushing fruits off-screen is how levels are won, so this makes levels unwinnable"),
            ("secondary", "true", "wrap secondary physics objects too"),
            ("trace", "false", "print a line to stdout every time something wraps"),
            (
                "max_overshoot",
                "200",
                "only wrap objects at most this far past the edge; the game parks and discards \
                 objects far off-screen (~1000 units), those keep the vanilla behavior. Physics \
                 objects move at most 100 units per tick, so keep this above 100",
            ),
            (
                "min_tick",
                "5",
                "don't wrap during the first ticks of a layout, when the game deletes objects \
                 placed off-screen in the level data",
            ),
        ]
    }

    fn apply(&self, code: &mut Code, cfg: &ModConfig) -> Result<()> {
        let wrap_coins = cfg.bool("coins", false)?;
        let wrap_secondary = cfg.bool("secondary", true)?;
        let trace = cfg.bool("trace", false)?;
        let max_overshoot = cfg.f64("max_overshoot", 200.0)?;
        let min_tick = cfg.i64("min_tick", 5)? as i32;

        let update = code.method(SHEET, "update")?;
        let destroy = code.method("fish.system.Sprite", "destroy")?;
        let coinedgecheck = code.method(SHEET, "coinedgecheck")?;
        let fun = code.func(update)?.clone();

        // ---- anchors in the physics_obj loop
        let ew_op = expect_one(find_field_access(code, &fun, "edgewith", false), "update reads `.edgewith`")?;
        let Opcode::Field { dst: ew, obj: item, .. } = fun.ops[ew_op] else {
            bail!("`.edgewith` read is not a Field op");
        };
        let pos = position_before(code, &fun, ew_op)?;
        let margin = margin_after(&fun, ew_op)?;
        let sheet = boss_mode_owner_after(code, &fun, ew_op)?;

        let destroy_default = next_match(&fun, ew_op, |op| calls(op, destroy)).context("destroy() after edge test")?;
        ensure_destroys_sprite_of(code, &fun, destroy_default, item)?;
        let coin_call = expect_one(find_calls(&fun, coinedgecheck), "update calls coinedgecheck()")?;
        ensure!(coin_call > ew_op, "coinedgecheck() call is not after the edge test");

        // ---- anchors in the secondary_physics loop
        let secondary = if wrap_secondary {
            let sp = expect_one(find_field_access(code, &fun, "secondary_physics", false), "update reads `.secondary_physics`")?;
            let pos_op = next_match(&fun, sp, |op| is_field(code, &fun, op, "position")).context("secondary position")?;
            let Opcode::Field { dst: pos2, .. } = fun.ops[pos_op] else { unreachable!() };
            let margin2 = margin_after(&fun, pos_op)?;
            let sheet2 = boss_mode_owner_after(code, &fun, pos_op)?;
            let d = next_match(&fun, pos_op, |op| calls(op, destroy)).context("secondary destroy()")?;
            Some((d, pos2, margin2, sheet2))
        } else {
            None
        };

        // ---- inject the helper
        let pos_t = fun.regs[pos.0 as usize];
        let sheet_t = fun.regs[sheet.0 as usize];
        let wrap = build_wrap(code, pos_t, sheet_t, max_overshoot, min_tick, trace)?;

        // ---- patch: guard each vanilla call with `if (mosa_screen_wrap(...)) skip it;`.
        // Sites are patched from the last to the first so earlier indices stay valid.
        let bool_t = code.ty_bool();
        let f64_t = code.ty_f64();
        let zero_c = code.float(0.0);
        let f = code.func_mut(update)?;
        let ok = add_reg(f, bool_t);
        if let Some((d, pos2, margin2, sheet2)) = secondary {
            ensure!(
                fun.regs[pos2.0 as usize] == pos_t && fun.regs[sheet2.0 as usize] == sheet_t,
                "secondary loop registers have unexpected types"
            );
            // This loop has no `edgewith`: pass 0.0 from a fresh register.
            let zero = add_reg(f, f64_t);
            insert_ops_with_exits(
                f,
                d,
                vec![
                    Opcode::Float { dst: zero, ptr: zero_c },
                    call(ok, wrap, &[pos2, zero, margin2, sheet2]),
                    Opcode::JTrue { cond: ok, offset: 0 },
                ],
                &[Exit { op: 2, target: d + 1 }],
                Incoming::ToInserted,
            );
        }
        let mut sites = vec![destroy_default];
        if wrap_coins {
            sites.push(coin_call);
        }
        sites.sort_unstable();
        for &site in sites.iter().rev() {
            guard_op(f, site, ok, wrap, &[pos, ew, margin, sheet]);
        }
        Ok(())
    }
}

/// `mosa_screen_wrap(pos, ew, margin, sheet): Bool` moves an object that just crossed an edge
/// to the opposite side and returns true. It returns false (leaving vanilla behavior to run)
/// during the first `min_tick` ticks of a layout, or if the object is further than
/// `max_overshoot` past the edge: that is how the game parks and discards objects (e.g. at
/// -1000,-1000, or below the screen in level data).
/// ```haxe
/// if (sheet.layout.currentTick < min_tick) return false;
/// var lo = margin - ew, hi = 338 - margin + ew, span = hi - lo, moved = false;
/// var ny = pos.y;
/// if (pos.y > hi) { if (pos.y - hi > max) return false; ny -= span; moved = true; }
/// else if (pos.y < lo) { if (lo - pos.y > max) return false; ny += span; moved = true; }
/// var nx = pos.x;
/// if (!sheet.bossMode) { same for x with 600 }
/// if (!moved) return false;
/// pos.x = nx; pos.y = ny; return true;
/// ```
fn build_wrap(
    code: &mut Code,
    pos_t: RefType,
    sheet_t: RefType,
    max_overshoot: f64,
    min_tick: i32,
    trace: bool,
) -> Result<RefFun> {
    let f64_t = code.ty_f64();
    let bool_t = code.ty_bool();
    let mut f = FnBuilder::new(code, "mosa_screen_wrap", &[pos_t, f64_t, f64_t, sheet_t], bool_t);
    let (pos, ew, margin, sheet) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3));
    let (lo, hi, span, over) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let (x, y, nx, ny) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let moved = f.reg(bool_t);
    let max = f.const_f64(max_overshoot);
    let fail = f.label();

    // Layouts place some objects off-screen and rely on the edge test to delete them on the
    // first tick: leave those alone.
    let layout = f.get_new(sheet, "layout")?;
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(min_tick);
    f.jlt(tick, min, fail);
    f.bool(moved, false);
    f.get(x, pos, "x")?;
    f.get(y, pos, "y")?;
    f.mov(nx, x);
    f.mov(ny, y);

    for (v, nv, size) in [(y, ny, SCREEN_H), (x, nx, SCREEN_W)] {
        let (past_hi, past_lo, done) = (f.label(), f.label(), f.label());
        if size == SCREEN_W {
            let boss = f.reg(bool_t);
            f.get(boss, sheet, "bossMode")?;
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
    f.jfalse(moved, fail);
    f.set(pos, "x", nx)?;
    f.set(pos, "y", ny)?;
    if trace {
        let p = |s| Print::Str(s);
        f.print(&[
            p("[screen-wrap] tick "), Print::Val(tick), p(" ("), Print::Val(x), p(", "), Print::Val(y),
            p(") -> ("), Print::Val(nx), p(", "), Print::Val(ny), p(")"),
        ])?;
    }
    let t = f.reg(bool_t);
    f.bool(t, true);
    f.ret(t);
    f.place(fail);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    f.finish()
}

fn calls(op: &Opcode, target: RefFun) -> bool {
    call_target(op).is_some_and(|(f, _)| f == target)
}

fn is_field(code: &Code, fun: &Function, op: &Opcode, name: &str) -> bool {
    match op {
        Opcode::Field { obj, field, .. } => code
            .field(fun.regs[obj.0 as usize], name)
            .is_ok_and(|f| f.0 == field.0),
        _ => false,
    }
}

/// The `pos = sprite.position` read right before the `edgewith` read.
fn position_before(code: &Code, fun: &Function, at: usize) -> Result<Reg> {
    let i = prev_match(fun, at, |op| is_field(code, fun, op, "position")).context("`.position` read")?;
    ensure!(at - i <= 4, "`.position` read is {} ops before `.edgewith`, expected <= 4", at - i);
    let Opcode::Field { dst, .. } = fun.ops[i] else { unreachable!() };
    Ok(dst)
}

/// The margin register: the edge test starts with `hi = 338; hi = hi - margin`.
fn margin_after(fun: &Function, at: usize) -> Result<Reg> {
    let i = next_match(fun, at, |op| matches!(op, Opcode::Sub { .. })).context("`338 - margin`")?;
    ensure!(i - at <= 6, "edge test `Sub` is {} ops away, expected <= 6", i - at);
    let Opcode::Sub { b, .. } = fun.ops[i] else { unreachable!() };
    Ok(b)
}

/// The register holding the event sheet, found through its `.bossMode` read.
fn boss_mode_owner_after(code: &Code, fun: &Function, at: usize) -> Result<Reg> {
    let i = next_match(fun, at, |op| is_field(code, fun, op, "bossMode")).context("`.bossMode` read")?;
    ensure!(i - at <= 20, "`.bossMode` read is {} ops away, expected <= 20", i - at);
    let Opcode::Field { obj, .. } = fun.ops[i] else { unreachable!() };
    Ok(obj)
}

/// Check that `destroy(x)` at `at` destroys `item.sprite`.
fn ensure_destroys_sprite_of(code: &Code, fun: &Function, at: usize, item: Reg) -> Result<()> {
    let (_, args) = call_target(&fun.ops[at]).unwrap();
    let load = prev_match(fun, at, |op| matches!(op, Opcode::Field { dst, .. } if *dst == args[0]))
        .context("sprite load before destroy()")?;
    ensure!(is_field(code, fun, &fun.ops[load], "sprite"), "destroy() arg is not a `.sprite`");
    let Opcode::Field { obj, .. } = fun.ops[load] else { unreachable!() };
    ensure!(obj == item, "destroy() is not called on the edge-tested object");
    Ok(())
}
