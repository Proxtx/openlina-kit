//! `mosa fn`, `mosa callers`, `mosa strings`: quick queries without a full dump.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use anyhow::{bail, Context, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefFun};
use hlbc_decompiler::ast::Method;
use hlbc_decompiler::fmt::FormatOptions;
use openlina_sdk::edit::call_target;
use openlina_sdk::Code;

/// Resolve `1234` or `pkg.Class.method`.
pub fn resolve(code: &Code, spec: &str) -> Result<RefFun> {
    if let Ok(i) = spec.parse::<usize>() {
        return Ok(RefFun(i));
    }
    let (class, name) = spec.rsplit_once('.').context("expected `Class.method` or a findex")?;
    code.method(class, name)
}

pub fn show_fn(input: &Path, spec: &str, hx: bool, ops: Option<&str>) -> Result<()> {
    let code = Code::load(input)?;
    let f = resolve(&code, spec)?;
    let fun = code.func(f)?;
    let range = match ops {
        Some(r) => {
            let (a, b) = r.split_once("..").context("--ops expects `start..end`")?;
            Some(a.parse::<usize>()?..b.parse::<usize>()?)
        }
        None => None,
    };
    let mut out = String::new();
    crate::dump::disassemble_range(&code, fun, range, &mut out);
    print!("{out}");
    if hx {
        std::panic::set_hook(Box::new(|_| {}));
        let text = catch_unwind(AssertUnwindSafe(|| {
            let m = Method {
                fun: f,
                static_: !fun.is_method(),
                dynamic: false,
                statements: hlbc_decompiler::decompile_code(&code.bc, fun),
            };
            let t = m.display(&code.bc, &FormatOptions::new(2)).to_string();
            t
        }));
        let _ = std::panic::take_hook();
        match text {
            Ok(t) => println!("{t}"),
            Err(_) => println!("// decompilation failed; read the disassembly above"),
        }
    }
    Ok(())
}

fn references(op: &Opcode, target: RefFun) -> bool {
    if call_target(op).is_some_and(|(f, _)| f == target) {
        return true;
    }
    matches!(op, Opcode::StaticClosure { fun, .. } | Opcode::InstanceClosure { fun, .. } if *fun == target)
}

pub fn callers(input: &Path, spec: &str) -> Result<()> {
    let code = Code::load(input)?;
    let target = resolve(&code, spec)?;
    println!("references to fn@{} {}:", target.0, code.func_name(target));
    let mut n = 0;
    for fun in &code.bc.functions {
        for (i, op) in fun.ops.iter().enumerate() {
            if references(op, target) {
                n += 1;
                println!(
                    "  fn@{} {} op {i}  // {}",
                    fun.findex.0,
                    code.func_name(fun.findex),
                    code.op_location(fun, i).unwrap_or_default()
                );
            }
        }
    }
    if n == 0 {
        println!("  none (it may only be called dynamically, e.g. through a field or `CallMethod`)");
    }
    Ok(())
}

pub fn strings(input: &Path, pattern: &str) -> Result<()> {
    let code = Code::load(input)?;
    let pat = pattern.to_lowercase();
    let hits: Vec<usize> = (0..code.bc.strings.len())
        .filter(|&i| code.bc.strings[i].to_lowercase().contains(&pat))
        .collect();
    if hits.is_empty() {
        bail!("no string contains `{pattern}`");
    }
    for i in hits.iter().take(200) {
        let users: Vec<&Function> = code
            .bc
            .functions
            .iter()
            .filter(|f| f.ops.iter().any(|op| matches!(op, Opcode::String { ptr, .. } if ptr.0 == *i)))
            .collect();
        println!("string@{i} {:?}", code.bc.strings[*i].as_str());
        for f in users.iter().take(10) {
            println!("    used in fn@{} {}", f.findex.0, code.func_name(f.findex));
        }
        if users.len() > 10 {
            println!("    ... and {} more", users.len() - 10);
        }
    }
    if hits.len() > 200 {
        println!("... {} more matches", hits.len() - 200);
    }
    Ok(())
}
