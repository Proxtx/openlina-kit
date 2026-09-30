//! `trace-calls`: print a line to stdout when chosen functions are called.
//!
//! The quickest way to answer "does this code run, and how often?" without a debugger.
//! Each function gets its own call counter (a new global); the first `first` calls are
//! printed, then every `every`-th call. With `args` (default), each line also shows the call's
//! arguments: numbers and booleans as values, strings as text, objects as their static type, anything
//! else (dynamic values, closures, refs, bytes) as `_`.

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
    let show_args = cfg.bool("args", true)?;
    let names: Vec<String> = match cfg.table.get("functions") {
        None => vec![],
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("`functions` must be strings")))
            .collect::<Result<_>>()?,
        Some(_) => bail!("`functions` must be an array of strings"),
    };
    for name in names {
        let target = code.find_fn(&name)?;
        let label = code.func_name(target);
        // Anonymous closures all print as `<anonymous>`: the findex and source location tell them apart.
        let place = match code.func(target).ok().and_then(|f| code.func_location(f)) {
            Some(loc) => format!("  (fn@{} {loc})", target.0),
            None => format!("  (fn@{})", target.0),
        };
        let i32_t = code.ty_i32();
        let void = code.ty_void();
        let counter = code.add_global(i32_t);

        let arg_types = if show_args { code.func_type(target)?.args.clone() } else { vec![] };
        // Per argument: print its value, or this fixed text (objects: their static type).
        let shown: Vec<Option<String>> = arg_types.iter().map(|&t| shown(code, t)).collect();
        let mut f = FnBuilder::new(code, "trace-calls/trace", &arg_types, void);
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
        let mut line = vec![Print::Str("[trace-calls] "), Print::Str(&label), Print::Str(" call #"), Print::Val(n)];
        if show_args {
            line.push(Print::Str(" ("));
            for (i, fixed) in shown.iter().enumerate() {
                if i > 0 {
                    line.push(Print::Str(", "));
                }
                line.push(match fixed {
                    None => Print::Val(f.arg(i)),
                    Some(text) => Print::Str(text),
                });
            }
            line.push(Print::Str(")"));
        }
        line.push(Print::Str(&place));
        f.print(&line)?;
        f.place(done);
        f.ret_void();
        let hook = f.finish()?;
        let regs: Vec<_> = (0..arg_types.len() as u32).map(openlina_sdk::hlbc::types::Reg).collect();
        prepend_call(code, target, hook, &regs)?;
    }
    Ok(())
}

/// How an argument of this type is shown: `None` = its value (numbers, booleans, strings), else a
/// fixed text. Objects show their static type: turning arbitrary objects into text (`Std.string`)
/// can crash the game.
fn shown(code: &Code, t: openlina_sdk::hlbc::types::RefType) -> Option<String> {
    use openlina_sdk::hlbc::types::Type;
    match &code.bc.types[t.0] {
        Type::UI8 | Type::UI16 | Type::I32 | Type::I64 | Type::F32 | Type::F64 | Type::Bool => None,
        Type::Obj(_) if code.type_name(t) == "String" => None,
        Type::Obj(_) | Type::Struct(_) => {
            let n = code.type_name(t);
            Some(n.rsplit('.').next().unwrap_or(&n).to_string())
        }
        _ => Some("_".into()),
    }
}
