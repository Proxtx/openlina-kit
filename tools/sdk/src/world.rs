//! The game world from inside a handler: which layout is a level, Lina's position, creating and
//! counting objects. All take registers of the function being built (`asm::FnBuilder`).
//!
//! ```ignore
//! let mut f = hooks::handler(code, "tick", "cannons/spawn")?;
//! let (sheet, layout) = (f.arg(0), f.arg(1));
//! let done = f.label();
//! world::jump_unless_level(&mut f, layout, done)?;
//! let (has, px, _py) = world::player_pos(&mut f, sheet)?;
//! let x = f.random()?;                              // 0..1
//! let cannon = world::spawn(&mut f, layout, "cannon_base", x, y)?;
//! ```

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use crate::asm::{FnBuilder, Label};

/// Layouts that run the gameplay sheet (and the `tick` hook) but are not levels: the hub, the
/// title screens, the tool selection.
pub const NOT_LEVELS: &[&str] = &["help", "main", "first_screen", "manager"];

/// Jump to `not_level` unless `layout` (`fish.system.Layout`) is a level.
pub fn jump_unless_level(f: &mut FnBuilder, layout: Reg, not_level: Label) -> Result<()> {
    let name = f.get_new(layout, "name")?;
    for s in NOT_LEVELS {
        let next = f.label();
        f.jstr_ne(name, s, next)?;
        f.jmp(not_level);
        f.place(next);
    }
    Ok(())
}

/// New Bool register: is `layout` a level?
pub fn is_level(f: &mut FnBuilder, layout: Reg) -> Result<Reg> {
    let r = f.reg_bool();
    let (no, done) = (f.label(), f.label());
    jump_unless_level(f, layout, no)?;
    f.bool(r, true);
    f.jmp(done);
    f.place(no);
    f.bool(r, false);
    f.place(done);
    Ok(r)
}

/// Lina's position: new registers `(has, x, y)` (Bool, F64, F64), from the first instance of the
/// sheet's `player` picker; `has` is false (x, y 0) on screens without her.
pub fn player_pos(f: &mut FnBuilder, sheet: Reg) -> Result<(Reg, Reg, Reg)> {
    let first = f.code().method("fish.system.Picker", "first")?;
    let player_t = f.code().class("fish.game.oclass.OClass_player")?;
    let (has, x, y) = (f.reg_bool(), f.reg_f64(), f.reg_f64());
    f.bool(has, false);
    f.float(x, 0.0);
    f.float(y, 0.0);
    let none = f.label();
    let picker = f.get_new(sheet, "player")?;
    f.jnull(picker, none);
    let d = f.call_new(first, &[picker])?;
    f.jnull(d, none);
    let p = f.cast(d, player_t);
    let sprite = f.get_new(p, "sprite")?;
    let pos = f.get_new(sprite, "position")?;
    f.get(x, pos, "x")?;
    f.get(y, pos, "y")?;
    f.bool(has, true);
    f.place(none);
    Ok((has, x, y))
}

/// `layout.createObject(kind, 0, x, y, false, "", null)`: an object of type `kind` (e.g.
/// `"box"`, `"cannon_base"`) created like level objects are, with its container parts. Only types
/// registered in `ObjectClasses.createInstance` work. Returns the new object's register.
pub fn spawn(f: &mut FnBuilder, layout: Reg, kind: &str, x: Reg, y: Reg) -> Result<Reg> {
    let create = f.code().method("fish.system.Layout", "createObject")?;
    let cb_t = f.code().func_type(create)?.args[7];
    let name = f.string_obj(kind)?;
    let layer = f.const_i32(0);
    let no = f.reg_bool();
    f.bool(no, false);
    let empty = f.string_obj("")?;
    let cb = f.reg(cb_t);
    f.op(Opcode::Null { dst: cb });
    f.call_new(create, &[layout, name, layer, x, y, no, empty, cb])
}

/// New I32 register: how many instances of `kind` the layout has now (the sheet's picker of that
/// name, e.g. `"cannon_base"`; 0 if it has none).
pub fn count(f: &mut FnBuilder, sheet: Reg, kind: &str) -> Result<Reg> {
    let n = f.reg_i32();
    f.int(n, 0);
    let none = f.label();
    let picker = f.get_new(sheet, kind)?;
    f.jnull(picker, none);
    let insts = f.get_new(picker, "insts")?;
    f.jnull(insts, none);
    let len = f.array_len(insts)?;
    f.mov(n, len);
    f.place(none);
    Ok(n)
}
