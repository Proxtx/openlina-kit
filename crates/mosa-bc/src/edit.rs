//! Finding and editing opcodes inside existing functions.
//!
//! HashLink jumps are *relative*: an op at index `p` with offset `o` jumps to `p + 1 + o`.
//! [`insert_ops`] and [`remove_ops`] keep every jump (including `Switch` and `Trap`) pointing
//! at the same instruction, and keep debug info aligned.

use anyhow::{bail, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefFun, Reg};

use crate::Code;

/// Mutable references to every jump offset carried by an opcode. For `Switch` the last one is
/// `end`, which marks the end of the switch block rather than a jump (the default case falls
/// through) but must be relocated like one.
pub fn jump_offsets_mut(op: &mut Opcode) -> Vec<&mut i32> {
    use Opcode::*;
    match op {
        JTrue { offset, .. }
        | JFalse { offset, .. }
        | JNull { offset, .. }
        | JNotNull { offset, .. }
        | JSLt { offset, .. }
        | JSGte { offset, .. }
        | JSGt { offset, .. }
        | JSLte { offset, .. }
        | JULt { offset, .. }
        | JUGte { offset, .. }
        | JNotLt { offset, .. }
        | JNotGte { offset, .. }
        | JEq { offset, .. }
        | JNotEq { offset, .. }
        | JAlways { offset }
        | Trap { offset, .. } => vec![offset],
        Switch { offsets, end, .. } => {
            let mut v: Vec<&mut i32> = offsets.iter_mut().collect();
            v.push(end);
            v
        }
        _ => vec![],
    }
}

/// Absolute jump targets of the op at index `pos`.
pub fn jump_targets(op: &Opcode, pos: usize) -> Vec<usize> {
    let mut op = op.clone();
    jump_offsets_mut(&mut op)
        .into_iter()
        .map(|o| (pos as i64 + 1 + *o as i64) as usize)
        .collect()
}

/// The function called by a direct call opcode (`Call0`..`CallN`), with its arguments.
pub fn call_target(op: &Opcode) -> Option<(RefFun, Vec<Reg>)> {
    use Opcode::*;
    Some(match op {
        Call0 { fun, .. } => (*fun, vec![]),
        Call1 { fun, arg0, .. } => (*fun, vec![*arg0]),
        Call2 { fun, arg0, arg1, .. } => (*fun, vec![*arg0, *arg1]),
        Call3 { fun, arg0, arg1, arg2, .. } => (*fun, vec![*arg0, *arg1, *arg2]),
        Call4 { fun, arg0, arg1, arg2, arg3, .. } => (*fun, vec![*arg0, *arg1, *arg2, *arg3]),
        CallN { fun, args, .. } => (*fun, args.clone()),
        _ => return None,
    })
}

/// Build the most compact direct call opcode for `dst = fun(args...)`.
pub fn call(dst: Reg, fun: RefFun, args: &[Reg]) -> Opcode {
    match *args {
        [] => Opcode::Call0 { dst, fun },
        [arg0] => Opcode::Call1 { dst, fun, arg0 },
        [arg0, arg1] => Opcode::Call2 { dst, fun, arg0, arg1 },
        [arg0, arg1, arg2] => Opcode::Call3 { dst, fun, arg0, arg1, arg2 },
        [arg0, arg1, arg2, arg3] => Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 },
        _ => Opcode::CallN { dst, fun, args: args.to_vec() },
    }
}

/// Indices of all ops matching a predicate.
pub fn find(fun: &Function, mut pred: impl FnMut(usize, &Opcode) -> bool) -> Vec<usize> {
    fun.ops.iter().enumerate().filter(|(i, op)| pred(*i, op)).map(|(i, _)| i).collect()
}

/// Indices of all direct calls to `target`.
pub fn find_calls(fun: &Function, target: RefFun) -> Vec<usize> {
    find(fun, |_, op| call_target(op).is_some_and(|(f, _)| f == target))
}

/// Indices of `Field`/`SetField`/`GetThis`/`SetThis` ops touching a field named `name`,
/// whatever the object type. Handy as a stable anchor when the field name is distinctive.
pub fn find_field_access(code: &Code, fun: &Function, name: &str, write: bool) -> Vec<usize> {
    find(fun, |_, op| {
        let (obj_ty, field) = match (op, write) {
            (Opcode::Field { obj, field, .. }, false) => (fun.regs[obj.0 as usize], *field),
            (Opcode::GetThis { field, .. }, false) => (fun.regs[0], *field),
            (Opcode::SetField { obj, field, .. }, true) => (fun.regs[obj.0 as usize], *field),
            (Opcode::SetThis { field, .. }, true) => (fun.regs[0], *field),
            _ => return false,
        };
        let fields = match &code.bc.types[obj_ty.0] {
            hlbc::types::Type::Obj(o) | hlbc::types::Type::Struct(o) => &o.fields,
            hlbc::types::Type::Virtual { fields } => fields,
            _ => return false,
        };
        fields.get(field.0).is_some_and(|f| code.str(f.name) == name)
    })
}

/// Assert that exactly one match was found. Use this to pin down every anchor a mod relies on,
/// so a game update fails loudly instead of patching the wrong instruction.
pub fn expect_one(matches: Vec<usize>, what: &str) -> Result<usize> {
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => bail!("anchor not found: {what}"),
        many => bail!("anchor `{what}` is ambiguous: {} matches at ops {:?}", many.len(), many),
    }
}

/// First match at or after `from`.
pub fn next_match(fun: &Function, from: usize, mut pred: impl FnMut(&Opcode) -> bool) -> Option<usize> {
    (from..fun.ops.len()).find(|&i| pred(&fun.ops[i]))
}

/// Last match strictly before `before`.
pub fn prev_match(fun: &Function, before: usize, mut pred: impl FnMut(&Opcode) -> bool) -> Option<usize> {
    (0..before).rev().find(|&i| pred(&fun.ops[i]))
}

/// Replace a single op. Jumps are unaffected since the op count doesn't change; the new op
/// must not carry jumps (use [`insert_ops`] for control flow).
pub fn replace_op(fun: &mut Function, at: usize, op: Opcode) {
    fun.ops[at] = op;
}

/// Where jumps that targeted the insertion point should land after [`insert_ops`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Incoming {
    /// Jumps to `at` now land on the first inserted op: the inserted code runs on every path
    /// that reached the original instruction (use this to prepend code to a basic block).
    /// Backward jumps are never redirected, since HashLink loop heads must stay on `Label`.
    ToInserted,
    /// Jumps to `at` still land on the original op: the inserted code only runs when falling
    /// through from the previous instruction.
    ToOriginal,
}

/// Insert `new_ops` before the op currently at index `at` (`at == ops.len()` appends).
///
/// Jumps *inside* `new_ops` are relative to their own position and are kept as-is, so they
/// may target any op inside the inserted block, or `at + new_ops.len()` (the original op).
/// Debug info for the new ops copies the location of the original op at `at`.
pub fn insert_ops(fun: &mut Function, at: usize, new_ops: Vec<Opcode>, incoming: Incoming) {
    let k = new_ops.len();
    if k == 0 {
        return;
    }
    let remap = |target: usize, from: usize| -> usize {
        if target < at {
            target
        } else if target == at && incoming == Incoming::ToInserted && from < at {
            at
        } else {
            target + k
        }
    };
    for (p, op) in fun.ops.iter_mut().enumerate() {
        let new_p = if p < at { p } else { p + k };
        for off in jump_offsets_mut(op) {
            let target = (p as i64 + 1 + *off as i64) as usize;
            let new_target = remap(target, p);
            *off = (new_target as i64 - new_p as i64 - 1) as i32;
        }
    }
    fun.ops.splice(at..at, new_ops);

    if let Some(dbg) = fun.debug_info.as_mut() {
        let loc = dbg.get(at).or(dbg.last()).copied().unwrap_or((0, 0));
        dbg.splice(at..at, std::iter::repeat_n(loc, k));
    }
    shift_assigns(fun, at, k as i64);
}

/// A jump in inserted code whose target is an op of the *original* function.
#[derive(Clone, Copy, Debug)]
pub struct Exit {
    /// Index of the jump op within the inserted ops.
    pub op: usize,
    /// Index of the target op in the function *before* insertion.
    pub target: usize,
}

/// [`insert_ops`], then point the given inserted jumps at original ops (resolved after
/// relocation). The offsets of those jump ops in `new_ops` are ignored.
pub fn insert_ops_with_exits(fun: &mut Function, at: usize, new_ops: Vec<Opcode>, exits: &[Exit], incoming: Incoming) {
    let k = new_ops.len();
    insert_ops(fun, at, new_ops, incoming);
    for e in exits {
        let pos = at + e.op;
        let target = if e.target < at { e.target } else { e.target + k };
        let offs = jump_offsets_mut(&mut fun.ops[pos]);
        assert_eq!(offs.len(), 1, "exit op must be a simple jump");
        for o in offs {
            *o = target as i32 - pos as i32 - 1;
        }
    }
}

/// Guard an existing op: insert `if (guard(args...)) goto <op after at>` right before op `at`,
/// so the original op only runs when the guard function returns false. All jumps that reached
/// `at` now evaluate the guard first. `bool_reg` receives the guard result.
///
/// This is the least invasive way to override vanilla behavior conditionally: the original
/// code stays in place as the fallback.
pub fn guard_op(fun: &mut Function, at: usize, bool_reg: Reg, guard: RefFun, args: &[Reg]) {
    insert_ops_with_exits(
        fun,
        at,
        vec![call(bool_reg, guard, args), Opcode::JTrue { cond: bool_reg, offset: 0 }],
        &[Exit { op: 1, target: at + 1 }],
        Incoming::ToInserted,
    );
}

/// Hook a function: call `hook(args...)` at its very start (result discarded). `args` are
/// registers of the hooked function, typically its arguments (`Reg(0)` is `this` for methods).
pub fn prepend_call(code: &mut Code, target: RefFun, hook: RefFun, args: &[Reg]) -> Result<()> {
    let ret = code.func_type(hook)?.ret;
    let fun = code.func_mut(target)?;
    let dst = add_reg(fun, ret);
    insert_ops(fun, 0, vec![call(dst, hook, args)], Incoming::ToOriginal);
    Ok(())
}

/// Remove ops `range`. Fails if any remaining jump targets a removed op.
pub fn remove_ops(fun: &mut Function, range: std::ops::Range<usize>) -> Result<()> {
    let k = range.len();
    for (p, op) in fun.ops.iter().enumerate() {
        if range.contains(&p) {
            continue;
        }
        for t in jump_targets(op, p) {
            if range.contains(&t) {
                bail!("op {p} jumps into removed range {range:?}");
            }
        }
    }
    for (p, op) in fun.ops.iter_mut().enumerate() {
        if range.contains(&p) {
            continue;
        }
        let new_p = if p < range.start { p } else { p - k };
        for off in jump_offsets_mut(op) {
            let target = (p as i64 + 1 + *off as i64) as usize;
            let new_target = if target < range.start { target } else { target - k };
            *off = (new_target as i64 - new_p as i64 - 1) as i32;
        }
    }
    fun.ops.drain(range.clone());
    if let Some(dbg) = fun.debug_info.as_mut() {
        dbg.drain(range.clone());
    }
    shift_assigns(fun, range.start, -(k as i64));
    Ok(())
}

fn shift_assigns(fun: &mut Function, at: usize, delta: i64) {
    let n = fun.ops.len();
    if let Some(assigns) = fun.assigns.as_mut() {
        for (_, pos) in assigns.iter_mut() {
            // Argument entries are encoded as negative values (read as huge usizes); skip them.
            if *pos > at && *pos <= n + delta.unsigned_abs() as usize {
                *pos = (*pos as i64 + delta) as usize;
            }
        }
    }
}

/// Add a register of type `t` to a function, returning it.
pub fn add_reg(fun: &mut Function, t: hlbc::types::RefType) -> Reg {
    fun.regs.push(t);
    Reg(fun.regs.len() as u32 - 1)
}
