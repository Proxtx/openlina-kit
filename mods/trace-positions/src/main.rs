//! `trace-positions`: a test fixture that prints where objects are, so scenarios can check
//! positions without the mod under test printing them:
//!
//! ```text
//! [pos] tick 130 player 412.5 150.25
//! ```
//!
//! at every tick in `ticks`, for every object in `physics_obj` whose `type` is in `types`. Check
//! them with `[[expect]] position = { tick = 130, type = "player", x = 412, y = 150, within = 6 }`
//! (see tools/lina/src/scenario.rs).

use anyhow::{bail, Context, Result};
use openlina_sdk::asm::{Label, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

/// `from-to/step`, `t`, or a comma list of those → (from, to, step).
fn parse_ticks(s: &str) -> Result<Vec<(i32, i32, i32)>> {
    s.split(',')
        .map(|part| {
            let part = part.trim();
            let (range, step) = part.split_once('/').unwrap_or((part, "1"));
            let (a, b) = range.split_once('-').unwrap_or((range, range));
            let p = |x: &str| x.trim().parse::<i32>().with_context(|| format!("ticks `{s}`: bad number `{x}`"));
            let (a, b, step) = (p(a)?, p(b)?, p(step)?);
            if a > b || step <= 0 {
                bail!("ticks `{s}`: bad range `{part}`");
            }
            Ok((a, b, step))
        })
        .collect()
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let types = cfg.list("types")?;
    let ranges = parse_ticks(cfg.str("ticks", "60-600/60")?)?;
    let obj_t = code.class("fish.system.ObjectClass")?;
    let i32_t = code.ty_i32();

    let mut f = hooks::handler(code, "tick", "trace-positions/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let (report, end) = (f.label(), f.label());
    let tick = f.get_new(layout, "currentTick")?;
    let rem = f.reg(i32_t);
    for (a, b, step) in ranges {
        let next: Label = f.label();
        let (lo, hi) = (f.const_i32(a), f.const_i32(b));
        f.jlt(tick, lo, next);
        f.jgt(tick, hi, next);
        let (off, st) = (f.reg(i32_t), f.const_i32(step));
        f.sub(off, tick, lo);
        f.op(Opcode::SMod { dst: rem, a: off, b: st });
        let zero = f.const_i32(0);
        f.jeq(rem, zero, report);
        f.place(next);
    }
    f.jmp(end);

    f.place(report);
    let picker = f.get_new(sheet, "physics_obj")?;
    let insts = f.get_new(picker, "insts")?;
    let n = f.array_len(insts)?;
    f.for_range(n, |f, i| {
        let next = f.label();
        let obj = f.array_get(insts, i, obj_t)?;
        f.jnull(obj, next);
        let ty = f.get_new(obj, "type")?;
        if !types.iter().any(|t| t == "*") {
            let wanted = f.label();
            for t in &types {
                let other = f.label();
                f.jstr_ne(ty, t, other)?;
                f.jmp(wanted);
                f.place(other);
            }
            f.jmp(next);
            f.place(wanted);
        }
        let sprite = f.get_new(obj, "sprite")?;
        f.jnull(sprite, next);
        let pos = f.get_new(sprite, "position")?;
        let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
        f.print(&[
            Print::Str("[pos] tick "),
            Print::Val(tick),
            Print::Str(" "),
            Print::Val(ty),
            Print::Str(" "),
            Print::Val(x),
            Print::Str(" "),
            Print::Val(y),
        ])?;
        f.place(next);
        Ok(())
    })?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}
