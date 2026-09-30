//! `inspect`: a test fixture that prints parts of the game's state at chosen moments, so a probe
//! needs no mod of its own:
//!
//! ```toml
//! fixtures = ["inspect"]
//! [options.inspect]
//! at = ["tick:30", "layout:manager@200"]
//! print = ["game.itemManager.pickedItems[].type.name", "@item_icon[].sprite.anim", "$fish.system.Main.frameTime"]
//! ```
//!
//! prints `[inspect] layout:manager@200 game.itemManager.pickedItems[0].type.name = swap`.
//!
//! Paths start at `Main.i`, at `@<type>` (the main layout's objects of that type, an array) or at
//! `$<class>.<static field>`. Every step after the root is a dynamic field read (HashLink `DynGet`),
//! so any field of any object works without knowing its type; `[]` walks an object array
//! (`hl.types.ArrayObj`, which is what `Picker.insts`, `itemPool`, … hold), `[n]` picks one element.
//! A null along the way prints `= null`.
//!
//! Moments: `tick:N` runs in the core `tick` hook (gameplay layouts) at layout tick N;
//! `layout:<name>@N` runs from `Main.mainLoop` on the first frame whose main layout is `<name>`
//! with `currentTick >= N` (menus, the MANAGER screen: layouts without gameplay ticks), once.

use anyhow::{bail, Context, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

enum Root {
    Main,
    Objects(String),
    Static(String, String),
}

enum Seg {
    Field(String),
    Each,
    Index(i32),
}

struct Query {
    root: Root,
    segs: Vec<Seg>,
}

/// `a.b[].c[2]` → segments (after the root part).
fn segments(parts: &[&str]) -> Result<Vec<Seg>> {
    let mut out = Vec::new();
    for p in parts {
        let (name, rest) = match p.find('[') {
            Some(i) => (&p[..i], &p[i..]),
            None => (*p, ""),
        };
        if !name.is_empty() {
            out.push(Seg::Field(name.to_string()));
        }
        let mut rest = rest;
        while let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').context("unclosed `[`")?;
            let idx = &r[..end];
            out.push(if idx.is_empty() { Seg::Each } else { Seg::Index(idx.parse().context("bad index")?) });
            rest = &r[end + 1..];
        }
        if !rest.is_empty() {
            bail!("unexpected `{rest}`");
        }
    }
    Ok(out)
}

fn parse(code: &Code, text: &str) -> Result<Query> {
    let root_and_segs = |root: Root, parts: &[&str]| -> Result<Query> { Ok(Query { root, segs: segments(parts)? }) };
    let q = if let Some(rest) = text.strip_prefix('@') {
        let parts: Vec<&str> = rest.split('.').collect();
        let (ty, first_rest) = match parts[0].find('[') {
            Some(i) => (&parts[0][..i], &parts[0][i..]),
            None => (parts[0], ""),
        };
        let mut all = vec![first_rest];
        all.extend_from_slice(&parts[1..]);
        root_and_segs(Root::Objects(ty.to_string()), &all)?
    } else if let Some(rest) = text.strip_prefix('$') {
        // The longest prefix that names a class; the next part is its static field.
        let parts: Vec<&str> = rest.split('.').collect();
        let k = (1..parts.len())
            .rev()
            .find(|&k| code.class(&parts[..k].join(".")).is_ok())
            .with_context(|| format!("`{text}`: no class in it"))?;
        let (field, idx) = match parts[k].find('[') {
            Some(i) => (&parts[k][..i], &parts[k][i..]),
            None => (parts[k], ""),
        };
        let mut all = vec![idx];
        all.extend_from_slice(&parts[k + 1..]);
        root_and_segs(Root::Static(parts[..k].join("."), field.to_string()), &all)?
    } else {
        let parts: Vec<&str> = text.split('.').collect();
        root_and_segs(Root::Main, &parts)?
    };
    Ok(q)
}

/// Pieces of the printed path: text, or an index register.
#[derive(Clone)]
enum Part {
    Text(String),
    Index(Reg),
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let queries: Vec<Query> = cfg.list("print")?.iter().map(|p| parse(code, p)).collect::<Result<_>>()?;
    if queries.is_empty() {
        bail!("inspect: set `print` (paths to print)");
    }
    for at in cfg.list("at")? {
        if let Some(n) = at.strip_prefix("tick:") {
            let n: i32 = n.parse().with_context(|| format!("bad `{at}`"))?;
            let mut f = hooks::handler(code, "tick", &format!("inspect/{at}"))?;
            let layout = f.arg(1);
            let end = f.label();
            let t = f.get_new(layout, "currentTick")?;
            let want = f.const_i32(n);
            f.jne(t, want, end);
            emit_all(&mut f, &at, &queries)?;
            f.place(end);
            f.ret_void();
            let h = f.finish()?;
            hooks::subscribe(code, "tick", h)?;
        } else if let Some(rest) = at.strip_prefix("layout:") {
            let (name, n) = match rest.split_once('@') {
                Some((name, n)) => (name, n.parse::<i32>().with_context(|| format!("bad `{at}`"))?),
                None => (rest, 0),
            };
            let main_loop = code.method("fish.system.Main", "mainLoop")?;
            let main_t = code.class("fish.system.Main")?;
            let (void, i32_t) = (code.ty_void(), code.ty_i32());
            let done = code.add_global(i32_t);
            let mut f = FnBuilder::new(code, &format!("inspect/{at}"), &[main_t], void);
            let main = f.arg(0);
            let end = f.label();
            let d = f.get_global(done);
            let zero = f.const_i32(0);
            f.jne(d, zero, end);
            let game = f.get_new(main, "game")?;
            f.jnull(game, end);
            let layouts = f.get_new(game, "layouts")?;
            f.jnull(layouts, end);
            let layout = f.get_new(layouts, "mainLayout")?;
            f.jnull(layout, end);
            let lname = f.get_new(layout, "name")?;
            f.jnull(lname, end);
            f.jstr_ne(lname, name, end)?;
            let t = f.get_new(layout, "currentTick")?;
            let want = f.const_i32(n);
            f.jlt(t, want, end);
            let one = f.const_i32(1);
            f.set_global(done, one);
            emit_all(&mut f, &at, &queries)?;
            f.place(end);
            f.ret_void();
            let h = f.finish()?;
            openlina_sdk::edit::prepend_call(code, main_loop, h, &[Reg(0)])?;
        } else {
            bail!("inspect: unknown moment `{at}` (tick:N or layout:<name>@N)");
        }
    }
    Ok(())
}

fn emit_all(f: &mut FnBuilder, at: &str, queries: &[Query]) -> Result<()> {
    for q in queries {
        let (root, first, elem): (Reg, String, Option<String>) = match &q.root {
            Root::Main => {
                let st = f.static_obj("fish.system.Main")?;
                (f.get_new(st, "i")?, String::new(), None)
            }
            Root::Objects(ty) => {
                let st = f.static_obj("fish.system.Main")?;
                let main = f.get_new(st, "i")?;
                let game = f.get_new(main, "game")?;
                let layouts = f.get_new(game, "layouts")?;
                let layout = f.get_new(layouts, "mainLayout")?;
                let insts = f.get_new(layout, "insts")?;
                let get = f.code().method("haxe.ds.StringMap", "get")?;
                let key = f.string_obj(ty)?;
                let v = f.call_new(get, &[insts, key])?;
                let class = format!("fish.game.oclass.OClass_{ty}");
                let elem = f.code().class(&class).is_ok().then_some(class);
                (v, format!("@{ty}"), elem)
            }
            Root::Static(class, name) => {
                let st = f.static_obj(class)?;
                (field(f, st, name)?, format!("${class}.{name}"), None)
            }
        };
        emit(f, at, root, &q.segs, vec![Part::Text(first)], elem.as_deref())?;
    }
    Ok(())
}

/// `cur.name`: a typed field read when `cur`'s type has the field (objects, virtuals); on a
/// superclass, a cast to the one subclass that has it; otherwise a dynamic read (`DynGet`).
fn field(f: &mut FnBuilder, cur: Reg, name: &str) -> Result<Reg> {
    let t = f.reg_type(cur);
    if f.code().field(t, name).is_ok() {
        return f.get_new(cur, name);
    }
    // A subclass of `t` that declares `name` (e.g. `ObjectClass` → `OClass_item` for `NAME`).
    let subs: Vec<_> = subclasses_with(f.code(), t, name);
    if let [one] = subs.as_slice() {
        let c = f.cast(cur, *one);
        return f.get_new(c, name);
    }
    if matches!(f.code().bc.types[t.0], openlina_sdk::hlbc::types::Type::Obj(_)) {
        let tn = f.code().type_name(t);
        bail!("`{name}`: no such field on {tn} or a subclass (search work/dump/hx for the class)");
    }
    let dyn_t = f.code().ty_dyn();
    let d = if f.reg_type(cur) == dyn_t {
        cur
    } else {
        let d = f.reg(dyn_t);
        f.op(Opcode::ToDyn { dst: d, src: cur });
        d
    };
    let v = f.reg(dyn_t);
    let s = f.code().string(name);
    f.op(Opcode::DynGet { dst: v, obj: d, field: s });
    Ok(v)
}

/// Object types below `t` (by `super` chain) that have a field `name`.
fn subclasses_with(
    code: &Code,
    t: openlina_sdk::hlbc::types::RefType,
    name: &str,
) -> Vec<openlina_sdk::hlbc::types::RefType> {
    use openlina_sdk::hlbc::types::{RefType, Type};
    let is_below = |mut x: RefType| loop {
        if x == t {
            return true;
        }
        match &code.bc.types[x.0] {
            Type::Obj(o) => match o.super_ {
                Some(s) => x = s,
                None => return false,
            },
            _ => return false,
        }
    };
    (0..code.bc.types.len())
        .map(RefType)
        .filter(|&x| x != t && matches!(code.bc.types[x.0], Type::Obj(_)) && is_below(x) && code.field(x, name).is_ok())
        .collect()
}

/// The register type for array elements: the known class, or `Dyn`.
fn elem_type(f: &mut FnBuilder, elem: Option<&str>) -> Result<openlina_sdk::hlbc::types::RefType> {
    match elem {
        Some(c) => f.code().class(c),
        None => Ok(f.code().ty_dyn()),
    }
}

fn emit(f: &mut FnBuilder, at: &str, cur: Reg, segs: &[Seg], path: Vec<Part>, elem: Option<&str>) -> Result<()> {
    let done = f.label();
    let not_null = f.label();
    f.jnotnull(cur, not_null);
    print_line(f, at, &path, None)?;
    f.jmp(done);
    f.place(not_null);
    match segs.first() {
        None => print_line(f, at, &path, Some(cur))?,
        Some(Seg::Field(name)) => {
            let v = field(f, cur, name)?;
            let mut p = path.clone();
            let sep = if matches!(path.as_slice(), [Part::Text(t)] if t.is_empty()) { "" } else { "." };
            p.push(Part::Text(format!("{sep}{name}")));
            emit(f, at, v, &segs[1..], p, None)?;
        }
        Some(Seg::Each) => {
            let arr = f.as_array_obj(cur)?;
            let n = f.array_len(arr)?;
            let elem = elem_type(f, elem)?;
            f.for_range(n, |f, i| {
                let el = f.array_get(arr, i, elem)?;
                let mut p = path.clone();
                p.push(Part::Text("[".into()));
                p.push(Part::Index(i));
                p.push(Part::Text("]".into()));
                emit(f, at, el, &segs[1..], p, None)
            })?;
        }
        Some(Seg::Index(k)) => {
            let arr = f.as_array_obj(cur)?;
            let n = f.array_len(arr)?;
            let kr = f.const_i32(*k);
            let out = f.label();
            f.jge(kr, n, out);
            let elem = elem_type(f, elem)?;
            let el = f.array_get(arr, kr, elem)?;
            let mut p = path.clone();
            p.push(Part::Text(format!("[{k}]")));
            emit(f, at, el, &segs[1..], p, None)?;
            f.place(out);
        }
    }
    f.place(done);
    Ok(())
}

fn print_line(f: &mut FnBuilder, at: &str, path: &[Part], value: Option<Reg>) -> Result<()> {
    let head = format!("[inspect] {at} ");
    let mut pieces: Vec<Print> = vec![Print::Str(&head)];
    for p in path {
        match p {
            Part::Text(t) => pieces.push(Print::Str(t)),
            Part::Index(r) => pieces.push(Print::Val(*r)),
        }
    }
    pieces.push(Print::Str(" = "));
    match value {
        Some(v) => pieces.push(Print::Val(v)),
        None => pieces.push(Print::Str("null")),
    }
    f.print(&pieces)
}
