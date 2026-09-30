//! `record`: prints what `lina run --record` needs to turn play into scenarios.
//!
//! In every gameplay layout (core `tick` hook):
//!
//! ```text
//! [record] level greendemo 1 modifier 0            (tick 1)
//! [record] slot 0 box 6                            (tick 1, every tool slot)
//! [record] tick 61 bits 8                          (whenever the input bits change)
//! ```
//!
//! Bits are `PlayerInputs.toBin()` of player 1 (0 up, 1 down, 2 left, 3 right, 4 jump, 5 shoot,
//! 6 switch, 7 restart), the same bits the harness feeds back with `readBin`. `lina` turns each
//! level attempt into `level`, `modifier`, `slots` and `inputs` of a scenario.

use anyhow::Result;
use openlina_sdk::asm::Print;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, _cfg: &ModConfig) -> Result<()> {
    let to_bin = code.method("fish.system.PlayerInputs", "toBin")?;
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let i32_t = code.ty_i32();
    let last = code.add_global(i32_t);
    let mut f = hooks::handler(code, "tick", "record/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    let t = f.get_new(layout, "currentTick")?;
    let inputs = f.get_new(layout, "p1Inputs")?;
    f.jnull(inputs, end);
    let bits = f.call_new(to_bin, &[inputs])?;
    let (report, not_first) = (f.label(), f.label());
    let one = f.const_i32(1);
    f.jne(t, one, not_first);
    // Level start: which level, modifier and tools.
    let st = f.static_obj("fish.system.Main")?;
    let main = f.get_new(st, "i")?;
    let game = f.get_new(main, "game")?;
    let lm = f.get_new(game, "levelManager")?;
    let cur = f.get_new(lm, "currentLevel")?;
    f.jnull(cur, end);
    let ty = f.get_new(cur, "type")?;
    let name = f.get_new(ty, "name")?;
    let m = f.get_new(cur, "modifier")?;
    f.print(&[Print::Str("[record] level "), Print::Val(name), Print::Str(" modifier "), Print::Val(m)])?;
    let _ = sheet;
    let mgr = f.get_new(game, "ev_manager_ev")?;
    let picker = f.get_new(mgr, "b_item")?;
    let insts = f.get_new(picker, "insts")?;
    let n = f.array_len(insts)?;
    f.for_range(n, |f, i| {
        let next = f.label();
        let slot = f.array_get(insts, i, b_item_t)?;
        f.jnull(slot, next);
        let item = f.get_new(slot, "item")?;
        f.jnull(item, next);
        let ty = f.get_new(item, "type")?;
        f.jnull(ty, next);
        let nm = f.get_new(ty, "name")?;
        let ammo = f.get_new(slot, "ammo")?;
        f.print(&[
            Print::Str("[record] slot "),
            Print::Val(i),
            Print::Str(" "),
            Print::Val(nm),
            Print::Str(" "),
            Print::Val(ammo),
        ])?;
        f.place(next);
        Ok(())
    })?;
    f.jmp(report);
    f.place(not_first);
    let prev = f.get_global(last);
    f.jeq(prev, bits, end);
    f.place(report);
    f.set_global(last, bits);
    f.print(&[Print::Str("[record] tick "), Print::Val(t), Print::Str(" bits "), Print::Val(bits)])?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}
