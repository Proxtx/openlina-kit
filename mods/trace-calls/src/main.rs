//! `trace-calls`: print a line to stdout when chosen functions are called.
//!
//! The quickest way to answer "does this code run, and how often?" without a debugger.
//! Each function gets its own call counter (a new global); the first `first` calls are
//! printed, then every `every`-th call.

use anyhow::{bail, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::prepend_call;
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::{Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let first = cfg.i64("first", 3)? as i32;
    let every = cfg.i64("every", 600)? as i32;
    let names: Vec<String> = match cfg.table.get("functions") {
        None => vec![],
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("`functions` must be strings")))
            .collect::<Result<_>>()?,
        Some(_) => bail!("`functions` must be an array of strings"),
    };
    for name in names {
        let target = if let Some(hook) = name.strip_prefix("hook:") {
            openlina_sdk::hooks::find(code, hook)?
        } else {
            match name.parse::<usize>() {
                Ok(i) => openlina_sdk::hlbc::types::RefFun(i),
                Err(_) => {
                    let (class, method) =
                        name.rsplit_once('.').ok_or_else(|| anyhow::anyhow!("bad function `{name}`"))?;
                    code.method(class, method)?
                }
            }
        };
        let label = code.func_name(target);
        // Anonymous closures all print as `<anonymous>`: the findex and source location tell them apart.
        let place = match code.func(target).ok().and_then(|f| code.func_location(f)) {
            Some(loc) => format!("  (fn@{} {loc})", target.0),
            None => format!("  (fn@{})", target.0),
        };
        let i32_t = code.ty_i32();
        let void = code.ty_void();
        let counter = code.add_global(i32_t);

        let mut f = FnBuilder::new(code, "trace-calls/trace", &[], void);
        let (n, lim, rem, zero) = (f.reg(i32_t), f.const_i32(first), f.reg(i32_t), f.const_i32(0));
        let (print, done) = (f.label(), f.label());
        f.op(Opcode::GetGlobal { dst: n, global: counter });
        f.op(Opcode::Incr { dst: n });
        f.op(Opcode::SetGlobal { global: counter, src: n });
        f.jle(n, lim, print);
        if every > 0 {
            let ev = f.const_i32(every);
            f.op(Opcode::SMod { dst: rem, a: n, b: ev });
            f.jeq(rem, zero, print);
        }
        f.jmp(done);
        f.place(print);
        f.print(&[
            Print::Str("[trace-calls] "),
            Print::Str(&label),
            Print::Str(" call #"),
            Print::Val(n),
            Print::Str(&place),
        ])?;
        f.place(done);
        f.ret_void();
        let hook = f.finish()?;
        prepend_call(code, target, hook, &[])?;
    }
    Ok(())
}
