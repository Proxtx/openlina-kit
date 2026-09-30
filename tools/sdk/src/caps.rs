//! Capability check: what a mod's patch makes the game able to do outside the game.
//!
//! The patch itself runs sandboxed (wasm), but its output is game code that runs with the
//! player's rights, and the game already contains natives for files, processes, sockets, TLS and
//! Steam. So after every mod the bytecode is compared with the bytecode before it:
//!
//! - new or changed functions that call (or make closures of) sensitive functions: file system,
//!   programs/environment, network, Steam, reflection (which could reach any of those)
//! - changed functions that already did such calls (their arguments may have been changed)
//! - new natives, changed string constants, changed existing types (method tables), a changed
//!   entry point
//!
//! Gameplay mods need none of this; the showcase mods trigger nothing. It is a heuristic: it finds
//! direct uses, not every indirect way (e.g. calling a game function that saves files with
//! arguments of the mod's choice). Reviews still matter.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};

use hlbc::opcodes::Opcode;
use hlbc::types::{RefFun, Type as HlType};

use crate::Code;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// files | programs | network | steam | reflection | natives | constants | types
    pub category: &'static str,
    pub text: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.category, self.text)
    }
}

/// Fingerprints of a bytecode, to compare the next mod's output against.
pub struct Snapshot {
    funcs: HashMap<usize, u64>,
    types: Vec<u64>,
    strings: Vec<String>,
    natives: HashSet<usize>,
    entrypoint: usize,
}

struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher);

impl std::fmt::Write for HashWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        s.hash(self.0);
        Ok(())
    }
}

fn fingerprint(v: &impl std::fmt::Debug) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let _ = write!(HashWriter(&mut h), "{v:?}");
    h.finish()
}

/// Types hashed with their method bindings sorted (hlbc keeps them in a HashMap).
fn type_fingerprint(t: &HlType) -> u64 {
    match t {
        HlType::Obj(o) | HlType::Struct(o) => {
            let mut bindings: Vec<_> = o.bindings.iter().map(|(k, v)| (k.0, v.0)).collect();
            bindings.sort();
            fingerprint(&(o.name, o.super_, o.global, &o.fields, &o.protos, bindings))
        }
        other => fingerprint(other),
    }
}

impl Snapshot {
    pub fn of(code: &Code) -> Self {
        Self {
            funcs: code.bc.functions.iter().map(|f| (f.findex.0, fingerprint(&(&f.ops, &f.regs)))).collect(),
            types: code.bc.types.iter().map(type_fingerprint).collect(),
            strings: code.bc.strings.iter().map(|s| s.to_string()).collect(),
            natives: code.bc.natives.iter().map(|n| n.findex.0).collect(),
            entrypoint: code.bc.entrypoint.0,
        }
    }
}

/// Category of a function, if calling it reaches outside the game.
fn sensitive(code: &Code, f: RefFun) -> Option<&'static str> {
    if let Some(n) = code.bc.natives.iter().find(|n| n.findex == f) {
        let (lib, name) = (code.str(n.lib), code.str(n.name));
        if lib == "steam" {
            return Some("steam");
        }
        return match name {
            _ if name.starts_with("file_") => Some("files"),
            "sys_delete" | "sys_rename" | "sys_create_dir" | "sys_remove_dir" | "sys_read_dir" => Some("files"),
            _ if name.starts_with("process_") => Some("programs"),
            "sys_command" | "sys_put_env" | "sys_set_cwd" => Some("programs"),
            _ if name.starts_with("socket_") || name.starts_with("host_") || name.starts_with("ssl_") => {
                Some("network")
            }
            _ => None,
        };
    }
    let fun = code.bc.functions.iter().find(|x| x.findex == f)?;
    let class = code.type_name(fun.parent?).replace('$', "");
    let method = code.str(fun.name);
    let prefix = |p: &str| class == p || class.starts_with(&format!("{p}."));
    if prefix("sys.io.File") || class.starts_with("sys.io.File") || prefix("sys.FileSystem") || prefix("sys.db") {
        return Some("files");
    }
    if prefix("sys.io.Process")
        || (class == "Sys" && ["command", "putEnv", "setCwd", "getEnv", "environment", "programPath"].contains(&method))
    {
        return Some("programs");
    }
    if prefix("sys.net") || prefix("sys.ssl") || prefix("sys.Http") || prefix("haxe.Http") {
        return Some("network");
    }
    if prefix("steam") {
        return Some("steam");
    }
    if (class == "Reflect" && ["callMethod", "field", "setField", "getProperty", "setProperty"].contains(&method))
        || (class == "Type"
            && ["resolveClass", "resolveEnum", "createInstance", "createEmptyInstance", "createEnum"].contains(&method))
    {
        return Some("reflection");
    }
    None
}

/// Functions an op calls or makes a closure of (virtual calls resolved through the class).
fn targets(code: &Code, fun: &hlbc::types::Function, op: &Opcode) -> Vec<RefFun> {
    use Opcode::*;
    if let Some((f, _)) = crate::edit::call_target(op) {
        return vec![f];
    }
    let method = |obj: hlbc::types::Reg, field: usize| -> Option<RefFun> {
        let mut t = *fun.regs.get(obj.0 as usize)?;
        loop {
            let HlType::Obj(o) = &code.bc.types[t.0] else { return None };
            if let Some(p) = o.protos.iter().find(|p| p.pindex == field as i32) {
                return Some(p.findex);
            }
            t = o.super_?;
        }
    };
    match op {
        StaticClosure { fun: f, .. } | InstanceClosure { fun: f, .. } => vec![*f],
        CallThis { field, .. } => method(hlbc::types::Reg(0), field.0).into_iter().collect(),
        CallMethod { field, args, .. } => args.first().and_then(|a| method(*a, field.0)).into_iter().collect(),
        VirtualClosure { obj, field, .. } => method(*obj, field.0 as usize).into_iter().collect(),
        _ => vec![],
    }
}

fn uses(code: &Code, fun: &hlbc::types::Function) -> Vec<(&'static str, RefFun)> {
    let mut out = Vec::new();
    for op in &fun.ops {
        for t in targets(code, fun, op) {
            if let Some(c) = sensitive(code, t) {
                if !out.contains(&(c, t)) {
                    out.push((c, t));
                }
            }
        }
    }
    out
}

/// What changed from `before` (snapshot + code) to `after` that reaches outside the game.
pub fn diff(before: &Snapshot, before_code: &Code, after: &Code) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |category, text: String| {
        let f = Finding { category, text };
        if !out.contains(&f) {
            out.push(f);
        }
    };
    for n in &after.bc.natives {
        if !before.natives.contains(&n.findex.0) {
            push("natives", format!("adds native `{}` from `{}`", after.str(n.name), after.str(n.lib)));
        }
    }
    for (i, s) in after.bc.strings.iter().enumerate().take(before.strings.len()) {
        if s.as_str() != before.strings[i] {
            let short = |x: &str| x.chars().take(40).collect::<String>();
            push(
                "constants",
                format!("changes the game's string constant {:?} to {:?}", short(&before.strings[i]), short(s)),
            );
        }
    }
    for (i, t) in after.bc.types.iter().enumerate().take(before.types.len()) {
        if type_fingerprint(t) != before.types[i] {
            push("types", format!("changes the existing type `{}`", after.type_name(hlbc::types::RefType(i))));
        }
    }
    if after.bc.entrypoint.0 != before.entrypoint {
        push("types", "changes the game's entry point".into());
    }
    for f in &after.bc.functions {
        let old = before.funcs.get(&f.findex.0);
        if old == Some(&fingerprint(&(&f.ops, &f.regs))) {
            continue;
        }
        let name = after.func_name(f.findex);
        let had: Vec<(&'static str, RefFun)> = match old {
            Some(_) => before_code
                .bc
                .functions
                .iter()
                .find(|x| x.findex == f.findex)
                .map(|x| uses(before_code, x))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        for (c, t) in uses(after, f) {
            if had.contains(&(c, t)) {
                push(c, format!("changes `{name}`, which uses `{}`", after.func_name(t)));
            } else {
                push(c, format!("`{name}` uses `{}`", after.func_name(t)));
            }
        }
    }
    out
}
