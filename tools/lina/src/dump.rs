//! `mosa dump`: decompile the whole bytecode into a greppable tree.
//!
//! Layout of the output directory:
//! - `hx/<package>/<Class>.hx`   decompiled pseudo-Haxe (lossy, but readable)
//! - `asm/<package>/<Class>.asm` exact disassembly of every function of the class
//! - `classes.tsv`               type index, name, super class, own fields
//! - `functions.tsv`             findex, qualified name, signature, source location, op count

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use anyhow::Result;
use hlbc::types::{Function, RefField, RefFun, Type, TypeObj};
use hlbc_decompiler::ast::Method;
use hlbc_decompiler::fmt::FormatOptions;
use openlina_sdk::Code;

pub fn run(input: &Path, out: &Path) -> Result<()> {
    let code = Code::load(input)?;
    let bc = &code.bc;
    fs::create_dir_all(out)?;

    // Group functions by owning class (static functions belong to `$Class`, fold them in).
    let mut by_class: HashMap<String, Vec<&Function>> = HashMap::new();
    let mut functions_tsv = String::from("findex\tname\tsignature\tlocation\tops\n");
    for f in &bc.functions {
        let owner = f
            .parent
            .map(|p| code.type_name(p).replace('$', ""))
            .unwrap_or_else(|| "_global".into());
        let _ = writeln!(
            functions_tsv,
            "{}\t{}\t{}\t{}\t{}",
            f.findex.0,
            code.func_name(f.findex),
            code.type_name(f.t),
            code.func_location(f).unwrap_or_default(),
            f.ops.len()
        );
        by_class.entry(owner).or_default().push(f);
    }
    fs::write(out.join("functions.tsv"), functions_tsv)?;

    // The decompiler panics on some control flow; keep going and note it.
    std::panic::set_hook(Box::new(|_| {}));
    let mut classes_tsv = String::from("type\tname\tsuper\tfields\n");
    let mut failed = 0usize;
    let mut count = 0usize;
    for (i, t) in bc.types.iter().enumerate() {
        let Type::Obj(obj) = t else { continue };
        let name = obj.name(bc).to_string();
        let fields: Vec<String> = obj
            .own_fields
            .iter()
            .map(|f| format!("{}:{}", f.name(bc), code.type_name(f.t)))
            .collect();
        let sup = obj.super_.map(|s| code.type_name(s)).unwrap_or_default();
        let _ = writeln!(classes_tsv, "{i}\t{name}\t{sup}\t{}", fields.join(", "));
        if name.contains('$') {
            continue;
        }
        count += 1;

        let hx = decompile_class(&code, obj, &mut failed);
        let header = format!("// type@{i} {name}\n");
        write_file(&class_path(out, "hx", &name, "hx"), &(header + &hx))?;

        let mut asm = String::new();
        if let Some(fs) = by_class.get(&name) {
            for f in fs {
                disassemble(&code, f, &mut asm);
            }
        }
        write_file(&class_path(out, "asm", &name, "asm"), &asm)?;
    }
    fs::write(out.join("classes.tsv"), classes_tsv)?;
    let _ = std::panic::take_hook();

    // Functions without an owning class.
    let mut asm = String::new();
    for f in by_class.get("_global").into_iter().flatten() {
        disassemble(&code, f, &mut asm);
    }
    write_file(&out.join("asm/_global.asm"), &asm)?;

    println!(
        "dumped {count} classes ({failed} functions failed to decompile) and {} functions to {}",
        bc.functions.len(),
        out.display()
    );
    Ok(())
}

/// Decompile a class method by method, so that one function the decompiler chokes on
/// doesn't hide the rest of the class. Every method is annotated with its findex and
/// source location so it can be cross-referenced with the `.asm` dump.
fn decompile_class(code: &Code, obj: &TypeObj, failed: &mut usize) -> String {
    let bc = &code.bc;
    let mut out = format!("class {}", obj.name(bc));
    if let Some(s) = obj.super_ {
        let _ = write!(out, " extends {}", code.type_name(s));
    }
    out.push_str(" {\n");

    let static_type = obj.get_static_type(bc);
    let is_bound = |o: &TypeObj, i: usize| {
        o.bindings.contains_key(&RefField(i + o.fields.len() - o.own_fields.len()))
    };
    for (i, f) in obj.own_fields.iter().enumerate() {
        if !is_bound(obj, i) {
            let _ = writeln!(out, "  var {}: {};", f.name(bc), code.type_name(f.t));
        }
    }
    if let Some(st) = static_type {
        for (i, f) in st.own_fields.iter().enumerate() {
            if !is_bound(st, i) {
                let _ = writeln!(out, "  static var {}: {};", f.name(bc), code.type_name(f.t));
            }
        }
    }

    let mut methods: Vec<(RefFun, bool, bool)> = Vec::new();
    methods.extend(obj.bindings.values().map(|f| (*f, false, true)));
    if let Some(st) = static_type {
        methods.extend(st.bindings.values().map(|f| (*f, true, false)));
    }
    methods.extend(obj.protos.iter().map(|p| (p.findex, false, false)));
    methods.sort_by_key(|m| m.0);

    let opts = FormatOptions::new(2);
    for (fun, static_, dynamic) in methods {
        let Ok(f) = code.func(fun) else { continue };
        let _ = writeln!(
            out,
            "\n  // fn@{} {}",
            fun.0,
            code.func_location(f).unwrap_or_default()
        );
        let text = catch_unwind(AssertUnwindSafe(|| {
            let m = Method {
                fun,
                static_,
                dynamic,
                statements: hlbc_decompiler::decompile_code(bc, f),
            };
            let t = m.display(bc, &opts).to_string();
            t
        }));
        match text {
            Ok(t) => out.push_str(&t),
            Err(_) => {
                *failed += 1;
                let _ = writeln!(
                    out,
                    "  // function {}: decompilation failed, see fn@{} in the .asm dump",
                    f.name(bc),
                    fun.0
                );
            }
        }
    }
    out.push_str("}\n");
    out
}

/// Write the exact disassembly of a function, one opcode per line, with source lines.
pub fn disassemble(code: &Code, f: &Function, out: &mut String) {
    disassemble_range(code, f, None, out)
}

/// Like [`disassemble`], optionally restricted to a range of ops.
pub fn disassemble_range(code: &Code, f: &Function, range: Option<std::ops::Range<usize>>, out: &mut String) {
    let bc = &code.bc;
    let _ = writeln!(
        out,
        "fn@{} {} {}  // {}",
        f.findex.0,
        code.func_name(f.findex),
        code.type_name(f.t),
        code.func_location(f).unwrap_or_default()
    );
    for (i, r) in f.regs.iter().enumerate() {
        let name = f.var_name(bc, i).map(|s| format!(" ({s})")).unwrap_or_default();
        let _ = writeln!(out, "    reg{i}: {}{name}", code.type_name(*r));
    }
    for (i, op) in f.ops.iter().enumerate() {
        if range.as_ref().is_some_and(|r| !r.contains(&i)) {
            continue;
        }
        let line = f
            .debug_info
            .as_ref()
            .and_then(|d| d.get(i))
            .map(|(_, l)| format!("L{l}"))
            .unwrap_or_default();
        let _ = writeln!(out, "  {i:>5} {line:>6}  {}", op.display(bc, f, i as i32, 12));
    }
    out.push('\n');
}

fn class_path(out: &Path, kind: &str, name: &str, ext: &str) -> PathBuf {
    let mut p = out.join(kind);
    let parts: Vec<&str> = name.split('.').collect();
    for pkg in &parts[..parts.len() - 1] {
        p.push(pkg);
    }
    // Class names may contain characters that are awkward in paths.
    let file = parts[parts.len() - 1].replace(['/', '<', '>', ':'], "_");
    p.push(format!("{file}.{ext}"));
    p
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}
