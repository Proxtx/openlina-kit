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
//! Left to vanilla (the handler returns false):
//! - coins/fruits unless `coins` (pushing fruits out is how levels are won)
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
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::modifiers::{self, Modifier};
use openlina_sdk::{hooks, Code, ModConfig};

/// Play-field size in layout units, hardcoded in the game's edge test.
const SCREEN_W: f64 = 600.0;
const SCREEN_H: f64 = 338.0;

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

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let coins = cfg.bool("coins", false)?;
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
    let mut f = hooks::handler(code, "edge_exit", "screen-wrap/wrap")?;
    let (pos, ew, margin, sheet, kind) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3), f.arg(4));
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
    wrap_body(&mut f, &w, pos, ew, margin, sheet, Some(kind), fail)?;
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
    let ew = f.const_f64(0.0);
    wrap_body(&mut f, w, p, ew, m, sh, None, fail)?;
    let guard = f.finish()?;
    hooks::subscribe(code, "player_edge", guard)
}

/// Wrap `pos` if it is past the screen's edge by at most `max_overshoot` and return true; jump to
/// `fail` (return false, vanilla goes on) otherwise. `kind` is printed by `trace`; None = the player.
#[allow(clippy::too_many_arguments)]
fn wrap_body(
    f: &mut FnBuilder,
    w: &Wrap,
    pos: Reg,
    ew: Reg,
    margin: Reg,
    sheet: Reg,
    kind: Option<Reg>,
    fail: Label,
) -> Result<()> {
    let f64_t = f.code().ty_f64();
    let bool_t = f.code().ty_bool();
    let (lo, hi, span, over) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let (x, y, nx, ny) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let moved = f.reg(bool_t);
    let max = f.const_f64(w.max_overshoot);

    // Only in levels that rolled the modifier.
    if let Some(id) = w.modifier {
        let active = modifiers::is_active(f, id)?;
        f.jfalse(active, fail);
    }

    let layout = f.get_new(sheet, "layout")?;
    let tick = f.get_new(layout, "currentTick")?;
    let min = f.const_i32(w.min_tick);
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
        f.print(&line)?;
    }
    let t = f.reg(bool_t);
    f.bool(t, true);
    f.ret(t);
    f.place(fail);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    Ok(())
}
