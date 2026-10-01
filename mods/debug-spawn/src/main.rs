//! `debug-spawn`: spawn objects at fixed ticks of every gameplay layout.
//!
//! One object with `object`/`tick`/`x`/`y`/`layer`, or several with `spawns`, a list of
//! `"<type>@<tick>:<x>,<y>"` (e.g. `["box@60:300,60", "s_ball@90:420,40"]`, layer 0).
//!
//! `pushes`, a list of `"<type>@<tick>:<vx>,<vy>"`: at that tick, every physics object of the type
//! gets that velocity (`Physics.setVelocity`), e.g. to throw something the player built off the
//! screen.
//!
//! A test fixture for other mods, and a minimal example of a `tick` hook handler calling game
//! code. With `screen-wrap`, a box spawned mid-air falls through the floor and keeps wrapping
//! from bottom to top forever.

use anyhow::Result;
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

struct Spawn {
    object: String,
    tick: i32,
    x: f64,
    y: f64,
    layer: i32,
}

fn parse_spawn(s: &str) -> Result<Spawn> {
    let err = || anyhow::anyhow!("spawn `{s}`: expected \"<type>@<tick>:<x>,<y>\"");
    let (object, rest) = s.split_once('@').ok_or_else(err)?;
    let (tick, pos) = rest.split_once(':').ok_or_else(err)?;
    let (x, y) = pos.split_once(',').ok_or_else(err)?;
    Ok(Spawn {
        object: object.trim().to_string(),
        tick: tick.trim().parse().map_err(|_| err())?,
        x: x.trim().parse().map_err(|_| err())?,
        y: y.trim().parse().map_err(|_| err())?,
        layer: 0,
    })
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let list = cfg.list("spawns")?;
    let spawns: Vec<Spawn> = if list.is_empty() {
        vec![Spawn {
            object: cfg.str("object", "box")?.to_string(),
            tick: cfg.i64("tick", 60)? as i32,
            x: cfg.f64("x", 300.0)?,
            y: cfg.f64("y", 60.0)?,
            layer: cfg.i64("layer", 0)? as i32,
        }]
    } else {
        list.iter().map(|s| parse_spawn(s)).collect::<Result<_>>()?
    };

    let pushes: Vec<Spawn> = cfg.list("pushes")?.iter().map(|s| parse_spawn(s)).collect::<Result<_>>()?;

    let create = code.method("fish.system.Layout", "createObject")?;
    let cb_t = code.func_type(create)?.args[7];
    let bool_t = code.ty_bool();

    // tick(sheet, layout): for each spawn
    //   if (layout.currentTick == tick) layout.createObject(object, layer, x, y, false, "", null);
    let mut f = hooks::handler(code, "tick", "debug-spawn/spawn")?;
    let layout = f.arg(1);
    let now = f.get_new(layout, "currentTick")?;
    for s in &spawns {
        let skip = f.label();
        let at = f.const_i32(s.tick);
        f.jne(now, at, skip);
        let name = f.string_obj(&s.object)?;
        let layer = f.const_i32(s.layer);
        let (x, y) = (f.const_f64(s.x), f.const_f64(s.y));
        let no = f.reg(bool_t);
        f.bool(no, false);
        let empty = f.string_obj("")?;
        let cb = f.reg(cb_t);
        f.op(Opcode::Null { dst: cb });
        f.call_new(create, &[layout, name, layer, x, y, no, empty, cb])?;
        f.place(skip);
    }
    // pushes: x, y are the velocity
    let set_v = f.code().method("fish.system.beh.Physics", "setVelocity")?;
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    let sheet = f.arg(0);
    for p in &pushes {
        let skip = f.label();
        let at = f.const_i32(p.tick);
        f.jne(now, at, skip);
        let objs = f.get_new(sheet, "physics_obj")?;
        f.jnull(objs, skip);
        let insts = f.get_new(objs, "insts")?;
        let n = f.array_len(insts)?;
        let (vx, vy) = (f.const_f64(p.x), f.const_f64(p.y));
        f.for_range(n, |f, i| {
            let next = f.label();
            let o = f.array_get(insts, i, obj_t)?;
            f.jnull(o, next);
            let t = f.get_new(o, "type")?;
            f.jstr_ne(t, &p.object, next)?;
            let ph = f.get_new(o, "physics")?;
            f.jnull(ph, next);
            f.call_new(set_v, &[ph, vx, vy])?;
            f.place(next);
            Ok(())
        })?;
        f.place(skip);
    }
    f.ret_void();
    let spawn = f.finish()?;
    hooks::subscribe(code, "tick", spawn)
}
