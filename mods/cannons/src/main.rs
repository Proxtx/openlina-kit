//! `cannons`: every level gets `count` (3) of the game's own cannons.
//!
//! Vanilla cannons are `cannon_base` objects (a dynamic physics body; `Layout.createObject`
//! also creates its container parts `cannon_rohr`, `spr_cannon_face`, `spr_cannon_nody` and
//! `spr_cannonrohr`). They sleep until Lina makes noise near them: when she lands or bumps into
//! something hard, `EvSheet_gameplay.stealth(x, y)` (source L4086) sets `awake = 1` on every
//! cannon within 111 units. An awake cannon turns its barrel towards the nearest player, charges
//! `power` and fires a `heavy_shot` (L17084-17160), which explodes.
//!
//! The mod subscribes to the core `tick` hook. At layout tick `tick` of every gameplay layout
//! whose name isn't in `skip_layouts` (the hub, title and tool selection run the gameplay sheet
//! too), it creates `count` cannons with `layout.createObject("cannon_base", 0, x, y, …)`, the
//! same call levels and the editor use for physics objects. The span [`x_min`, `x_max`] is cut
//! into `count` equal bands and each cannon gets a random x in its own band (`Math.random`: new
//! spots on every attempt; the harness seeds it in tests), all at height `y`; they fall onto
//! whatever is below them. They drop in one after another, `stagger` ticks apart, and start at
//! y 4, hidden behind the HUD bar (which covers y < 23), so they fall into view instead of popping
//! up in the play field; the game's edge test would only delete them above y -7 (margin 25 minus
//! `edgewith` 32). A cannon that would land on Lina wakes up at once and shoots her point-blank, so an x closer than
//! `player_distance` to her (horizontally, at that tick) is rolled again, up to 8 times, then
//! moved `player_distance` to her right (or left, if that leaves the span). With `awake` they
//! start awake and shoot without being woken first.
//!
//! Left alone: the cannons' behavior (aiming, charging, shots, explosions) is vanilla, and so is
//! what happens to a cannon that falls off the screen (deleted, or wrapped with `screen-wrap`).
//! Retrying a level after a death reloads its layout from the level data, so the cannons come
//! back at their tick, not twice (`trace` prints how many were already there).

use anyhow::{ensure, Result};
use openlina_sdk::asm::Print;
use openlina_sdk::{hooks, world, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let count = cfg.i64("count", 3)?;
    ensure!((0..=20).contains(&count), "count must be 0 to 20, got {count}");
    let tick = cfg.i64("tick", 10)? as i32;
    ensure!(tick >= 1, "tick must be at least 1, got {tick}");
    let stagger = cfg.i64("stagger", 45)? as i32;
    ensure!(stagger >= 0, "stagger must be 0 or more, got {stagger}");
    let (x_min, x_max, y) = (cfg.f64("x_min", 60.0)?, cfg.f64("x_max", 540.0)?, cfg.f64("y", 4.0)?);
    ensure!(x_min <= x_max, "x_min ({x_min}) is larger than x_max ({x_max})");
    let awake = cfg.bool("awake", false)?;
    let distance = cfg.f64("player_distance", 120.0)?;
    let skip = cfg.list("skip_layouts")?;
    let trace = cfg.bool("trace", false)?;

    let cannon_t = code.class("fish.game.oclass.OClass_cannon_base")?;
    code.field(cannon_t, "awake")?;

    // tick(sheet, layout):
    //   if (currentTick isn't one of the cannons' ticks || skip.contains(layout.name)) return;
    //   for each band i, at tick + i * stagger: spawn("cannon_base", lo_i + random() * band, y), away from Lina
    let mut f = hooks::handler(code, "tick", "cannons/spawn")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let done = f.label();
    let now = f.get_new(layout, "currentTick")?;
    let (first, last) = (f.const_i32(tick), f.const_i32(tick + (count.max(1) as i32 - 1) * stagger));
    f.jlt(now, first, done);
    f.jlt(last, now, done);
    let name = f.get_new(layout, "name")?;
    for s in &skip {
        let next = f.label();
        f.jstr_ne(name, s, next)?;
        f.jmp(done);
        f.place(next);
    }

    let band = if count > 0 { (x_max - x_min) / count as f64 } else { 0.0 };
    let (yr, width, one) = (f.const_f64(y), f.const_f64(band), f.const_f64(1.0));
    let x = f.reg_f64();
    let (has, px, _) = world::player_pos(&mut f, sheet)?;

    if trace {
        // cannons already in the layout (a retry that kept them would show 3 here)
        let not_first = f.label();
        f.jne(now, first, not_first);
        let n = world::count(&mut f, sheet, "cannon_base")?;
        f.print(&[
            Print::Str("[cannons] "),
            Print::Val(name),
            Print::Str(" tick "),
            Print::Val(now),
            Print::Str(": "),
            Print::Val(n),
            Print::Str(" cannons already there"),
        ])?;
        f.place(not_first);
    }

    let (dist, max_x) = (f.const_f64(distance), f.const_f64(x_max));
    let (dx, tries, alt) = (f.reg_f64(), f.reg_i32(), f.reg_f64());
    let (max_tries, inc) = (f.const_i32(8), f.const_i32(1));
    for i in 0..count {
        let (roll, placed, later) = (f.label(), f.label(), f.label());
        let at = f.const_i32(tick + i as i32 * stagger);
        f.jne(now, at, later);
        f.int(tries, 0);
        f.place(roll);
        let r = f.random()?;
        f.mul(x, r, width);
        let lo = f.const_f64(x_min + band * i as f64);
        f.add(x, x, lo);
        // far enough from Lina?
        f.jfalse(has, placed);
        f.sub(dx, x, px);
        f.abs(dx, dx);
        f.jge(dx, dist, placed);
        f.add(tries, tries, inc);
        f.jlt(tries, max_tries, roll);
        // still too close: `distance` to her right, or to her left past the span's end
        f.add(x, px, dist);
        f.jle(x, max_x, placed);
        f.sub(alt, px, dist);
        f.mov(x, alt);
        f.place(placed);
        let obj = world::spawn(&mut f, layout, "cannon_base", x, yr)?;
        if awake {
            let skip_wake = f.label();
            let c = f.cast(obj, cannon_t);
            f.jnull(c, skip_wake);
            f.set(c, "awake", one)?;
            f.place(skip_wake);
        }
        if trace {
            f.print(&[
                Print::Str("[cannons] "),
                Print::Val(name),
                Print::Str(" tick "),
                Print::Val(now),
                Print::Str(&format!(": cannon {} at (", i + 1)),
                Print::Val(x),
                Print::Str(", "),
                Print::Val(yr),
                Print::Str(")"),
            ])?;
        }
        f.place(later);
    }
    f.place(done);
    f.ret_void();
    let spawn = f.finish()?;
    hooks::subscribe(code, "tick", spawn)
}
