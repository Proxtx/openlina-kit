//! `lina fn`, `lina callers`, `lina strings`, `lina refs`, `lina class`: quick queries without a full dump.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use anyhow::{bail, Context, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefFun};
use hlbc_decompiler::ast::Method;
use hlbc_decompiler::fmt::FormatOptions;
use openlina_sdk::edit::call_target;
use openlina_sdk::Code;

/// Resolve `1234`, `pkg.Class.method`, or the name of a function a mod injected (`swap/use`,
/// `hook/tick`: the name given to `FnBuilder::new`, shown in dumps and stack traces).
pub fn resolve(code: &Code, spec: &str) -> Result<RefFun> {
    code.find_fn(spec)
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
    for fun in code.bc.functions.iter().filter(|f| f.findex != target) {
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
    let hits: Vec<usize> =
        (0..code.bc.strings.len()).filter(|&i| code.bc.strings[i].to_lowercase().contains(&pat)).collect();
    if hits.is_empty() {
        bail!("no string contains `{pattern}`");
    }
    for i in hits.iter().take(200) {
        let users: Vec<&Function> = code
            .bc
            .functions
            .iter()
            .filter(|f| {
                f.ops.iter().any(|op| match op {
                    Opcode::String { ptr, .. } => ptr.0 == *i,
                    // The game keeps most literals in globals initialized with the constant.
                    Opcode::GetGlobal { global, .. } => {
                        code.global_string(*global) == Some(code.bc.strings[*i].as_str())
                    }
                    _ => false,
                })
            })
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

/// `lina refs <name>`: every function that reads or writes a field called `name`, calls a native of
/// that name, or uses the string `name` (as a `String` op or through a global holding the
/// constant, which is how the game keeps most of its strings: object types, animation names).
pub fn refs(input: &Path, name: &str) -> Result<()> {
    let code = Code::load(input)?;
    // Natives of that name: their call sites count too.
    let natives: Vec<RefFun> = code
        .bc
        .natives
        .iter()
        .filter(|n| code.str(n.name) == name || format!("{}.{}", code.str(n.lib), code.str(n.name)) == name)
        .map(|n| n.findex)
        .collect();
    let mut rows = Vec::new();
    for fun in &code.bc.functions {
        let calls: Vec<usize> = fun
            .ops
            .iter()
            .enumerate()
            .filter(|(_, op)| natives.iter().any(|&n| references(op, n)))
            .map(|(i, _)| i)
            .collect();
        let reads = openlina_sdk::edit::find_field_access(&code, fun, name, false);
        let writes = openlina_sdk::edit::find_field_access(&code, fun, name, true);
        let mut dyn_ops = Vec::new();
        let mut strings = Vec::new();
        for (i, op) in fun.ops.iter().enumerate() {
            match op {
                Opcode::DynGet { field, .. } | Opcode::DynSet { field, .. } if code.str(*field) == name => {
                    dyn_ops.push(i)
                }
                Opcode::String { ptr, .. } if code.str(*ptr) == name => strings.push(i),
                Opcode::GetGlobal { global, .. } if code.global_string(*global) == Some(name) => strings.push(i),
                _ => {}
            }
        }
        if reads.is_empty() && writes.is_empty() && dyn_ops.is_empty() && strings.is_empty() && calls.is_empty() {
            continue;
        }
        let list = |kind: &str, v: &[usize]| -> Option<String> {
            (!v.is_empty()).then(|| {
                let ops: Vec<String> = v.iter().take(6).map(|i| i.to_string()).collect();
                format!("{kind} ×{} (op {}{})", v.len(), ops.join(","), if v.len() > 6 { ",…" } else { "" })
            })
        };
        let what: Vec<String> = [
            list("reads", &reads),
            list("writes", &writes),
            list("dynamic", &dyn_ops),
            list("string", &strings),
            list("calls native", &calls),
        ]
        .into_iter()
        .flatten()
        .collect();
        let loc = code.func_location(fun).unwrap_or_default();
        rows.push(format!("fn@{} {}  // {loc}\n    {}", fun.findex.0, code.func_name(fun.findex), what.join(", ")));
    }
    if rows.is_empty() {
        let like: Vec<&str> = code
            .bc
            .strings
            .iter()
            .map(|s| s.as_str())
            .filter(|s| s.to_lowercase().contains(&name.to_lowercase()) && s.len() < 80)
            .take(15)
            .collect();
        bail!(
            "nothing reads, writes, calls or uses `{name}` (exact match; fields, strings, natives). \
             Strings that contain it: {like:?}"
        );
    }
    rows.sort();
    for r in rows.iter().take(150) {
        println!("{r}");
    }
    if rows.len() > 150 {
        println!("... {} more functions", rows.len() - 150);
    }
    Ok(())
}

/// `lina class <name>`: a class's fields (inherited ones marked), methods and static fields. The
/// name may leave out the package (`OClass_item`).
pub fn class(input: &Path, name: &str) -> Result<()> {
    use hlbc::types::{RefType, Type};
    let code = Code::load(input)?;
    let t = match code.class(name) {
        Ok(t) => t,
        Err(_) => {
            let suffix = format!(".{name}");
            let found: Vec<RefType> = (0..code.bc.types.len())
                .map(RefType)
                .filter(|&t| matches!(code.bc.types[t.0], Type::Obj(_)))
                .filter(|&t| {
                    let n = code.type_name(t);
                    n == name || n.ends_with(&suffix)
                })
                .filter(|&t| !code.type_name(t).contains('$'))
                .collect();
            match found.as_slice() {
                [one] => *one,
                [] => bail!("no class `{name}` (see work/dump/classes.tsv)"),
                many => bail!(
                    "`{name}` is ambiguous: {}",
                    many.iter().map(|t| code.type_name(*t)).collect::<Vec<_>>().join(", ")
                ),
            }
        }
    };
    let Type::Obj(o) = &code.bc.types[t.0] else { bail!("{name} is not a class") };
    let mut chain = vec![];
    let mut sup = o.super_;
    while let Some(s) = sup {
        chain.push(code.type_name(s));
        sup = match &code.bc.types[s.0] {
            Type::Obj(p) => p.super_,
            _ => None,
        };
    }
    println!(
        "class {}{}",
        code.type_name(t),
        if chain.is_empty() { String::new() } else { format!(" extends {}", chain.join(" < ")) }
    );
    let own: std::collections::HashSet<String> = o.own_fields.iter().map(|f| code.str(f.name).to_string()).collect();
    println!("fields:");
    for f in &o.fields {
        let n = code.str(f.name);
        let mark = if own.contains(n) { "" } else { "  (inherited)" };
        println!("  {n}: {}{mark}", code.type_name(f.t));
    }
    println!("methods:");
    for p in &o.protos {
        println!("  {}  fn@{}", code.str(p.name), p.findex.0);
    }
    if o.global.0 > 0 {
        if let Some(Type::Obj(st)) = code.bc.globals.get(o.global.0 - 1).map(|g| &code.bc.types[g.0]) {
            println!("statics (`$` object):");
            let mut seen = std::collections::HashSet::new();
            for f in st.fields.iter().chain(&st.own_fields) {
                let n = code.str(f.name);
                if seen.insert(n.to_string()) && !n.starts_with("__") {
                    println!("  {n}: {}", code.type_name(f.t));
                }
            }
        }
    }
    Ok(())
}
