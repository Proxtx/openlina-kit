//! `debug-spawn`: spawn an object at a fixed tick of every gameplay layout.
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

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let object = cfg.str("object", "box")?.to_string();
    let tick = cfg.i64("tick", 60)? as i32;
    let (x, y) = (cfg.f64("x", 300.0)?, cfg.f64("y", 60.0)?);
    let layer = cfg.i64("layer", 0)? as i32;

    let create = code.method("fish.system.Layout", "createObject")?;
    let cb_t = code.func_type(create)?.args[7];
    let bool_t = code.ty_bool();

    // tick(sheet, layout):
    //   if (layout.currentTick == tick) layout.createObject(object, layer, x, y, false, "", null);
    let mut f = hooks::handler(code, "tick", "debug-spawn/spawn")?;
    let layout = f.arg(1);
    let skip = f.label();
    let now = f.get_new(layout, "currentTick")?;
    let at = f.const_i32(tick);
    f.jne(now, at, skip);
    let name = f.string_obj(&object)?;
    let layer = f.const_i32(layer);
    let (x, y) = (f.const_f64(x), f.const_f64(y));
    let no = f.reg(bool_t);
    f.bool(no, false);
    let empty = f.string_obj("")?;
    let cb = f.reg(cb_t);
    f.op(Opcode::Null { dst: cb });
    f.call_new(create, &[layout, name, layer, x, y, no, empty, cb])?;
    f.place(skip);
    f.ret_void();
    let spawn = f.finish()?;
    hooks::subscribe(code, "tick", spawn)
}
