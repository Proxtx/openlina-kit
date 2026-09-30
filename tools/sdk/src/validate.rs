//! Static sanity checks for patched or injected functions.
//!
//! This is not a full HashLink verifier; it catches the mistakes that are easy to make when
//! patching by hand and that otherwise show up as opaque JIT crashes:
//! - register indices out of range
//! - jumps outside the function, backward jumps not landing on a `Label`
//! - calls with the wrong number of arguments
//! - value kind mismatches (int vs float vs pointer) for call args, return values, fields
//! - functions that can fall off their end

use anyhow::{bail, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefType, Type};

use crate::edit::{call_target, jump_targets};
use crate::Code;

/// Machine-level kind of a value, which is what the JIT cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Void,
    U8,
    U16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Ptr,
}

pub fn kind(code: &Code, t: RefType) -> Kind {
    match &code.bc.types[t.0] {
        Type::Void => Kind::Void,
        Type::UI8 => Kind::U8,
        Type::UI16 => Kind::U16,
        Type::I32 => Kind::I32,
        Type::I64 => Kind::I64,
        Type::F32 => Kind::F32,
        Type::F64 => Kind::F64,
        Type::Bool => Kind::Bool,
        _ => Kind::Ptr,
    }
}

/// Validate every function touched by patches ([`Code::touched`]).
pub fn check_touched(code: &Code) -> Result<()> {
    let mut errors = Vec::new();
    for &f in &code.touched {
        match code.func(f) {
            Ok(fun) => errors.extend(check_function(code, fun)),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if !errors.is_empty() {
        bail!("validation failed:\n  {}", errors.join("\n  "));
    }
    Ok(())
}

/// Validate a single function, returning human-readable problems.
pub fn check_function(code: &Code, fun: &Function) -> Vec<String> {
    let name = code.func_name(fun.findex);
    let mut errs = Vec::new();
    let nregs = fun.regs.len();
    let n = fun.ops.len();
    let mut err = |i: usize, msg: String| errs.push(format!("{name} op {i}: {msg}"));

    if n == 0 {
        err(0, "function has no ops".into());
        return errs;
    }
    if let Some(d) = &fun.debug_info {
        if d.len() != n {
            err(0, format!("debug_info has {} entries for {n} ops", d.len()));
        }
    }
    if let Some(sig) = fun.t.as_fun(&code.bc) {
        if sig.args.len() > nregs {
            err(0, "fewer registers than arguments".into());
        } else {
            for (i, a) in sig.args.iter().enumerate() {
                if kind(code, *a) != kind(code, fun.regs[i]) {
                    err(0, format!("arg {i} type differs from reg{i}"));
                }
            }
        }
    }

    for (i, op) in fun.ops.iter().enumerate() {
        for r in regs_of(op) {
            if r >= nregs {
                err(i, format!("register reg{r} out of range ({nregs} registers)"));
            }
        }
        let mut targets = jump_targets(op, i);
        if let Opcode::Switch { .. } = op {
            // `end` marks the end of the switch block (the default case falls through); it is
            // not a jump and may point one past the last op.
            let end = targets.pop().unwrap_or(0);
            if end > n {
                err(i, format!("switch end {end} out of range"));
            }
        }
        for t in targets {
            if t >= n {
                err(i, format!("jump target {t} out of range"));
            } else if t <= i && !matches!(fun.ops[t], Opcode::Label) {
                err(i, format!("backward jump to {t}, which is not a Label"));
            }
        }
        if regs_of(op).iter().any(|&r| r >= nregs) {
            continue;
        }
        let rk = |r: &hlbc::types::Reg| kind(code, fun.regs[r.0 as usize]);

        if let Some((callee, args)) = call_target(op) {
            match code.func_type(callee) {
                Ok(sig) => {
                    if sig.args.len() != args.len() {
                        err(
                            i,
                            format!(
                                "call to {} with {} args, expects {}",
                                code.func_name(callee),
                                args.len(),
                                sig.args.len()
                            ),
                        );
                    } else {
                        for (j, (a, p)) in args.iter().zip(&sig.args).enumerate() {
                            if rk(a) != kind(code, *p) {
                                err(
                                    i,
                                    format!(
                                        "call to {} arg {j}: passing {:?} for {:?}",
                                        code.func_name(callee),
                                        rk(a),
                                        kind(code, *p)
                                    ),
                                );
                            }
                        }
                    }
                    let dst = call_dst(op);
                    let dk = rk(&dst);
                    let retk = kind(code, sig.ret);
                    if dk != Kind::Void && dk != retk {
                        err(i, format!("call result {:?} stored in {:?} register", retk, dk));
                    }
                }
                Err(e) => err(i, e.to_string()),
            }
        }
        match op {
            Opcode::Field { dst, obj, field } | Opcode::SetField { obj, field, src: dst } => {
                let ot = fun.regs[obj.0 as usize];
                match code.field_type(ot, *field) {
                    Ok(ft) if kind(code, ft) != rk(dst) => {
                        err(i, format!("field {} is {:?}, register is {:?}", field.0, kind(code, ft), rk(dst)))
                    }
                    Ok(_) => {}
                    Err(e) => err(i, e.to_string()),
                }
            }
            Opcode::Ret { ret } => {
                if let Some(sig) = fun.t.as_fun(&code.bc) {
                    let want = kind(code, sig.ret);
                    if want != Kind::Void && rk(ret) != want {
                        err(i, format!("returns {:?}, function returns {:?}", rk(ret), want));
                    }
                }
            }
            Opcode::Add { dst, a, b }
            | Opcode::Sub { dst, a, b }
            | Opcode::Mul { dst, a, b }
            | Opcode::SDiv { dst, a, b }
                if rk(dst) != rk(a) || rk(a) != rk(b) =>
            {
                err(i, format!("arithmetic on mixed kinds {:?} {:?} -> {:?}", rk(a), rk(b), rk(dst)));
            }
            Opcode::Float { dst, .. } if !matches!(rk(dst), Kind::F32 | Kind::F64) => {
                err(i, "Float into non-float register".into())
            }
            Opcode::Int { dst, .. } if !matches!(rk(dst), Kind::I32 | Kind::U8 | Kind::U16 | Kind::I64) => {
                err(i, "Int into non-int register".into())
            }
            _ => {}
        }
    }
    if !matches!(
        fun.ops.last(),
        Some(Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. } | Opcode::JAlways { .. })
    ) {
        err(n - 1, "function can fall off its end".into());
    }
    errs
}

fn call_dst(op: &Opcode) -> hlbc::types::Reg {
    use Opcode::*;
    match op {
        Call0 { dst, .. }
        | Call1 { dst, .. }
        | Call2 { dst, .. }
        | Call3 { dst, .. }
        | Call4 { dst, .. }
        | CallN { dst, .. } => *dst,
        _ => unreachable!(),
    }
}

/// All register operands of an op. Uses the Debug representation so it stays correct for
/// every opcode without a 98-arm match.
pub fn regs_of(op: &Opcode) -> Vec<usize> {
    let s = format!("{op:?}");
    let mut out = Vec::new();
    let mut rest = s.as_str();
    while let Some(p) = rest.find("Reg(") {
        rest = &rest[p + 4..];
        let end = rest.find(')').unwrap_or(0);
        if let Ok(v) = rest[..end].parse() {
            out.push(v);
        }
    }
    out
}
