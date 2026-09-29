//! `trace-calls`: print a line to stdout when chosen functions are called.
//!
//! The quickest way to answer "does this code run, and how often?" without a debugger.
//! Each function gets its own call counter (a new global); the first `first` calls are
//! printed, then every `every`-th call.

use anyhow::{bail, Result};
use mosa_bc::asm::{FnBuilder, Print};
use mosa_bc::edit::prepend_call;
use mosa_bc::hlbc::opcodes::Opcode;
use mosa_bc::{Code, Mod, ModConfig};

pub struct TraceCalls;

impl Mod for TraceCalls {
    fn id(&self) -> &'static str {
        "trace-calls"
    }

    fn description(&self) -> &'static str {
        "Debug tool: print a line when chosen functions are called"
    }

    fn options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("functions", "[]", "functions to trace, as `pkg.Class.method` or findex strings"),
            ("first", "3", "print the first N calls of each function"),
            ("every", "600", "then print every N-th call (0 = never)"),
        ]
    }

    fn apply(&self, code: &mut Code, cfg: &ModConfig) -> Result<()> {
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
            let target = match name.parse::<usize>() {
                Ok(i) => mosa_bc::hlbc::types::RefFun(i),
                Err(_) => {
                    let (class, method) = name.rsplit_once('.').ok_or_else(|| anyhow::anyhow!("bad function `{name}`"))?;
                    code.method(class, method)?
                }
            };
            let label = code.func_name(target);
            let i32_t = code.ty_i32();
            let void = code.ty_void();
            let counter = code.add_global(i32_t);

            let mut f = FnBuilder::new(code, "mosa_trace_calls", &[], void);
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
            f.print(&[Print::Str("[trace-calls] "), Print::Str(&label), Print::Str(" call #"), Print::Val(n)])?;
            f.place(done);
            f.ret_void();
            let hook = f.finish()?;
            prepend_call(code, target, hook, &[])?;
        }
        Ok(())
    }
}
