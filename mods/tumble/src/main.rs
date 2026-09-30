//! `tumble`: a level that turns. "Tumble 1" is a drum: a ring of tiles around the screen center
//! that turns a quarter turn every 10 seconds. The player, boxes and the fruit tumble inside;
//! the fruit has to leave through the gap in the ring.
//!
//! The level is a level pack built in code (`openlina_sdk::levels`): the fixed parts are in
//! `levels/tumble.toml`, the ring is generated here (`segments` tiles on a circle of `radius`, each
//! turned to the tangent, one left out as the gap), with angles and positions off the editor's grid.
//!
//! Turning: on each `tick` while a level of this pack is played, during the last
//! `turn_ticks` of every `period` ticks, every static body (`physics.immovable`) in `physics_obj`
//! is rotated around the center by a quarter turn / `turn_ticks`: position and `sprite.angle`,
//! which `Physics.syncPosWithSprite` passes on to the Box2D body. Loose objects are pushed by the
//! moving tiles. Restarting the level restarts the drum.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::levels::{self, LevelPack, Object};
use openlina_sdk::{hooks, Code, ModConfig};

/// Screen center: the drum's axis.
const CX: f64 = 300.0;
const CY: f64 = 169.0;

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let period = cfg.i64("period", 1200)? as i32;
    let turn_ticks = cfg.i64("turn_ticks", 180)? as i32;
    let segments = cfg.i64("segments", 14)? as usize;
    let radius = cfg.f64("radius", 130.0)?;
    let trace = cfg.bool("trace", false)?;
    anyhow::ensure!(turn_ticks > 0 && turn_ticks <= period, "need 0 < turn_ticks <= period");

    let mut pack = LevelPack::from_toml(include_str!("../levels/tumble.toml"))?;
    // The ring; segment 0 (at the top, -90°) is the gap.
    for i in 1..segments {
        let phi = -90.0 + 360.0 * i as f64 / segments as f64;
        let (s, c) = phi.to_radians().sin_cos();
        pack.levels[0].objects.push(Object {
            kind: "tile_long".into(),
            x: CX + radius * c,
            y: CY + radius * s,
            angle: Some(phi + 90.0),
            w: None,
            h: None,
        });
    }
    levels::register(code, &pack)?;

    let obj_t = code.class("fish.system.ObjectClass")?;
    let i32_t = code.ty_i32();
    let f64_t = code.ty_f64();
    let mut f = hooks::handler(code, "tick", "tumble/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    levels::jump_unless_in_pack(&mut f, &pack, end)?;
    let tick = f.get_new(layout, "currentTick")?;
    let per = f.const_i32(period);
    let phase = f.reg(i32_t);
    f.op(Opcode::SMod { dst: phase, a: tick, b: per });
    let start = f.const_i32(period - turn_ticks);
    f.jlt(phase, start, end);
    if trace {
        let not_first = f.label();
        f.jne(phase, start, not_first);
        f.print(&[Print::Str("[tumble] tick "), Print::Val(tick), Print::Str(" turning")])?;
        f.place(not_first);
    }

    let step = std::f64::consts::FRAC_PI_2 / turn_ticks as f64;
    let (cos, sin) = (f.const_f64(step.cos()), f.const_f64(step.sin()));
    let deg = f.const_f64(90.0 / turn_ticks as f64);
    let (cx, cy) = (f.const_f64(CX), f.const_f64(CY));
    let (dx, dy, t, nx, ny) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    let picker = f.get_new(sheet, "physics_obj")?;
    let insts = f.get_new(picker, "insts")?;
    let n = f.array_len(insts)?;
    f.for_range(n, |f, i| {
        let next = f.label();
        let obj = f.array_get(insts, i, obj_t)?;
        f.jnull(obj, next);
        let physics = f.get_new(obj, "physics")?;
        f.jnull(physics, next);
        let immovable = f.get_new(physics, "immovable")?;
        f.jfalse(immovable, next);
        let sprite = f.get_new(obj, "sprite")?;
        f.jnull(sprite, next);
        let destroyed = f.get_new(sprite, "destroyed")?;
        f.jtrue(destroyed, next);
        let pos = f.get_new(sprite, "position")?;
        f.get(dx, pos, "x")?;
        f.sub(dx, dx, cx);
        f.get(dy, pos, "y")?;
        f.sub(dy, dy, cy);
        // (nx, ny) = c + R(step) (dx, dy); y points down, so this turns clockwise like sprite.angle.
        f.mul(nx, dx, cos);
        f.mul(t, dy, sin);
        f.sub(nx, nx, t);
        f.add(nx, nx, cx);
        f.mul(ny, dx, sin);
        f.mul(t, dy, cos);
        f.add(ny, ny, t);
        f.add(ny, ny, cy);
        f.set(pos, "x", nx)?;
        f.set(pos, "y", ny)?;
        let angle = f.get_new(sprite, "angle")?;
        f.add(angle, angle, deg);
        f.set(sprite, "angle", angle)?;
        f.place(next);
        Ok(())
    })?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}
