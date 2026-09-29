//! `debug-spawn`: spawn an object at a fixed tick of every gameplay layout.
//!
//! A test fixture for other mods (and a minimal example of hooking a function and calling game
//! code): with `screen-wrap` enabled, a box spawned mid-air falls through the floor and keeps
//! wrapping from bottom to top forever.

use anyhow::Result;
use mosa_bc::asm::FnBuilder;
use mosa_bc::edit::prepend_call;
use mosa_bc::hlbc::types::Reg;
use mosa_bc::{Code, Mod, ModConfig};

pub struct DebugSpawn;

impl Mod for DebugSpawn {
    fn id(&self) -> &'static str {
        "debug-spawn"
    }

    fn description(&self) -> &'static str {
        "Test fixture: spawn an object at a fixed tick of every gameplay layout"
    }

    fn options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("object", "\"box\"", "object type to create (see fish.game.oclass.OClass_*)"),
            ("tick", "60", "layout tick at which to spawn"),
            ("x", "300", "x position (the play field is 600 x 338)"),
            ("y", "60", "y position"),
            ("layer", "0", "layer index"),
        ]
    }

    fn apply(&self, code: &mut Code, cfg: &ModConfig) -> Result<()> {
        let object = cfg.str("object", "box")?.to_string();
        let tick = cfg.i64("tick", 60)? as i32;
        let (x, y) = (cfg.f64("x", 300.0)?, cfg.f64("y", 60.0)?);
        let layer = cfg.i64("layer", 0)? as i32;

        let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
        let create = code.method("fish.system.Layout", "createObject")?;
        let layout_t = code.class("fish.system.Layout")?;
        let void = code.ty_void();
        let bool_t = code.ty_bool();

        // mosa_debug_spawn(layout):
        //   if (layout.currentTick == tick) layout.createObject(object, layer, x, y, false, "", null);
        let mut f = FnBuilder::new(code, "mosa_debug_spawn", &[layout_t], void);
        let layout = f.arg(0);
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
        let cb_t = f.code().func_type(create)?.args[7];
        let cb = f.reg(cb_t);
        f.op(mosa_bc::hlbc::opcodes::Opcode::Null { dst: cb });
        f.call_new(create, &[layout, name, layer, x, y, no, empty, cb])?;
        f.place(skip);
        f.ret_void();
        let spawn = f.finish()?;

        // update(this, layout): reg1 is the layout argument.
        prepend_call(code, update, spawn, &[Reg(1)])
    }
}
