//! `screen-wrap`: objects leaving the screen come back on the opposite side.
//!
//! Subscribes to the core `edge_exit` hook, which fires when vanilla is about to delete an
//! object that left the screen (see `mods/core`). The handler moves the object by one
//! play-field size and returns true ("handled", vanilla skipped).
//!
//! In long levels and boss arenas (`sheet.bossMode`, levels that scroll horizontally) vanilla
//! only tests the top and bottom edges, so only those wrap there.
//!
//! Left to vanilla (the handler returns false):
//! - coins/fruits unless `coins` (pushing fruits out is how levels are won)
//! - objects during the first `min_tick` ticks of a layout (levels place objects off-screen and
//!   rely on the edge test to delete them)
//! - objects more than `max_overshoot` past the edge (the game parks objects at e.g. -1000,-1000)
//!
//! Moving `sprite.position` is enough: `Physics.syncPosWithSprite` teleports the Box2D body and
//! keeps its velocity.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::{hooks, Code, ModConfig};

/// Play-field size in layout units, hardcoded in the game's edge test.
const SCREEN_W: f64 = 600.0;
const SCREEN_H: f64 = 338.0;

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let coins = cfg.bool("coins", false)?;
    let secondary = cfg.bool("secondary", true)?;
    let trace = cfg.bool("trace", false)?;
    let max_overshoot = cfg.f64("max_overshoot", 200.0)?;
    let min_tick = cfg.i64("min_tick", 5)? as i32;

    let f64_t = code.ty_f64();
    let bool_t = code.ty_bool();
    let mut f = hooks::handler(code, "edge_exit", "screen-wrap/wrap")?;
    let (pos, ew, margin, sheet, kind) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3), f.arg(4));
    let (lo, hi, span, over) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let (x, y, nx, ny) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let moved = f.reg(bool_t);
    let max = f.const_f64(max_overshoot);
    let fail = f.label();

    // Which kinds of objects to handle.
    if !coins {
        let one = f.const_i32(1);
        f.jeq(kind, one, fail);
    }
    if !secondary {
        let two = f.const_i32(2);
        f.jeq(kind, two, fail);
    }

    let layout = f.get_new(sheet, "layout")?;
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(min_tick);
    f.jlt(tick, min, fail);

    f.bool(moved, false);
    f.get(x, pos, "x")?;
    f.get(y, pos, "y")?;
    f.mov(nx, x);
    f.mov(ny, y);

    // Same bounds as vanilla: lo = margin - ew, hi = size - margin + ew.
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
    f.jfalse(moved, fail);
    f.set(pos, "x", nx)?;
    f.set(pos, "y", ny)?;
    if trace {
        let p = Print::Str;
        f.print(&[
            p("[screen-wrap] tick "), Print::Val(tick), p(" kind "), Print::Val(kind), p(" ("), Print::Val(x),
            p(", "), Print::Val(y), p(") -> ("), Print::Val(nx), p(", "), Print::Val(ny), p(")"),
        ])?;
    }
    let t = f.reg(bool_t);
    f.bool(t, true);
    f.ret(t);
    f.place(fail);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    let wrap = f.finish()?;
    hooks::subscribe(code, "edge_exit", wrap)
}
