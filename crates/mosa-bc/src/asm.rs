//! A small assembler for writing new HashLink functions from Rust.
//!
//! ```ignore
//! let f64_ = code.ty_f64();
//! let void = code.ty_void();
//! let mut f = FnBuilder::new(code, "mosa_example", &[point_ty], void);
//! let p = f.arg(0);
//! let y = f.reg(f64_);
//! let skip = f.label();
//! f.get(y, p, "y")?;              // y = p.y
//! let limit = f.const_f64(100.0); // limit = 100.0
//! f.jlt(y, limit, skip);          // if y < limit goto skip
//! f.set(p, "y", limit)?;          // p.y = limit
//! f.place(skip);
//! f.ret_void();
//! let findex = f.finish()?;
//! ```
//!
//! Registers `0..args.len()` are the arguments. Every op gets a debug location in a synthetic
//! file named after the function (`mosa/<name>`) with the op index as the line, so a crash
//! inside injected code shows up clearly in HashLink stack traces.

use anyhow::{anyhow, bail, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefFun, RefType, Reg, ValBool};

use crate::edit::{call, jump_offsets_mut};
use crate::Code;

/// A piece of a [`FnBuilder::print`] line.
pub enum Print<'s> {
    Str(&'s str),
    Val(Reg),
}

/// A jump target inside the function being built.
#[derive(Clone, Copy, Debug)]
pub struct Label(usize);

pub struct FnBuilder<'a> {
    code: &'a mut Code,
    name: String,
    t: RefType,
    ret: RefType,
    regs: Vec<RefType>,
    ops: Vec<Opcode>,
    labels: Vec<Option<usize>>,
    fixups: Vec<(usize, Label)>,
    parent: Option<RefType>,
}

impl<'a> FnBuilder<'a> {
    /// Start a new function `name(args) -> ret`.
    pub fn new(code: &'a mut Code, name: &str, args: &[RefType], ret: RefType) -> Self {
        let t = code.ty_fun(args, ret);
        Self {
            code,
            name: name.to_string(),
            t,
            ret,
            regs: args.to_vec(),
            ops: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            parent: None,
        }
    }

    /// Access the underlying bytecode, e.g. to look up methods while building.
    pub fn code(&mut self) -> &mut Code {
        self.code
    }

    /// Register holding argument `i`.
    pub fn arg(&self, i: usize) -> Reg {
        Reg(i as u32)
    }

    /// Allocate a new register of type `t`.
    pub fn reg(&mut self, t: RefType) -> Reg {
        self.regs.push(t);
        Reg(self.regs.len() as u32 - 1)
    }

    pub fn reg_type(&self, r: Reg) -> RefType {
        self.regs[r.0 as usize]
    }

    /// Emit a raw opcode. Returns its index.
    pub fn op(&mut self, op: Opcode) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    // ------------------------------------------------------------------ control flow

    pub fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }

    /// Bind a label to the next op. Emits a `Label` op, which HashLink requires as the
    /// target of backward jumps and which is harmless otherwise.
    pub fn place(&mut self, l: Label) {
        self.labels[l.0] = Some(self.ops.len());
        self.op(Opcode::Label);
    }

    /// Emit a jump opcode (constructed with any offset) that targets `l`.
    pub fn jump(&mut self, op: Opcode, l: Label) {
        let i = self.op(op);
        self.fixups.push((i, l));
    }

    pub fn jmp(&mut self, l: Label) {
        self.jump(Opcode::JAlways { offset: 0 }, l)
    }
    pub fn jtrue(&mut self, cond: Reg, l: Label) {
        self.jump(Opcode::JTrue { cond, offset: 0 }, l)
    }
    pub fn jfalse(&mut self, cond: Reg, l: Label) {
        self.jump(Opcode::JFalse { cond, offset: 0 }, l)
    }
    pub fn jnull(&mut self, reg: Reg, l: Label) {
        self.jump(Opcode::JNull { reg, offset: 0 }, l)
    }
    pub fn jnotnull(&mut self, reg: Reg, l: Label) {
        self.jump(Opcode::JNotNull { reg, offset: 0 }, l)
    }
    /// `if a < b goto l` (signed ints or floats)
    pub fn jlt(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JSLt { a, b, offset: 0 }, l)
    }
    /// `if a >= b goto l` (signed ints or floats)
    pub fn jge(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JSGte { a, b, offset: 0 }, l)
    }
    /// `if a > b goto l` (signed ints or floats)
    pub fn jgt(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JSGt { a, b, offset: 0 }, l)
    }
    /// `if a <= b goto l` (signed ints or floats)
    pub fn jle(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JSLte { a, b, offset: 0 }, l)
    }
    pub fn jeq(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JEq { a, b, offset: 0 }, l)
    }
    pub fn jne(&mut self, a: Reg, b: Reg, l: Label) {
        self.jump(Opcode::JNotEq { a, b, offset: 0 }, l)
    }

    pub fn ret(&mut self, r: Reg) {
        self.op(Opcode::Ret { ret: r });
    }

    /// Return from a `Void` function.
    pub fn ret_void(&mut self) {
        let void = self.code.ty_void();
        let r = self.reg(void);
        self.ret(r);
    }

    // --------------------------------------------------------------------- values

    pub fn int(&mut self, dst: Reg, v: i32) {
        let ptr = self.code.int(v);
        self.op(Opcode::Int { dst, ptr });
    }
    pub fn float(&mut self, dst: Reg, v: f64) {
        let ptr = self.code.float(v);
        self.op(Opcode::Float { dst, ptr });
    }
    pub fn bool(&mut self, dst: Reg, v: bool) {
        self.op(Opcode::Bool { dst, value: ValBool(v) });
    }
    /// Load a string constant as raw UTF-16 `hl.Bytes` (what the `String` opcode produces;
    /// `dst` must be a `Bytes` register). Use [`FnBuilder::string_obj`] for a Haxe `String`.
    pub fn string(&mut self, dst: Reg, s: &str) {
        let ptr = self.code.string(s);
        self.op(Opcode::String { dst, ptr });
    }

    /// New register holding a Haxe `String` object: `String.__alloc__(bytes, length)`, which is
    /// how the Haxe compiler materializes string literals.
    pub fn string_obj(&mut self, s: &str) -> Result<Reg> {
        let bytes_t = self.code.ty_bytes();
        let alloc = self.code.method("String", "__alloc__")?;
        let bytes = self.reg(bytes_t);
        self.string(bytes, s);
        let len = self.const_i32(s.encode_utf16().count() as i32);
        self.call_new(alloc, &[bytes, len])
    }

    /// New `F64` register holding `v`.
    pub fn const_f64(&mut self, v: f64) -> Reg {
        let t = self.code.ty_f64();
        let r = self.reg(t);
        self.float(r, v);
        r
    }
    /// New `I32` register holding `v`.
    pub fn const_i32(&mut self, v: i32) -> Reg {
        let t = self.code.ty_i32();
        let r = self.reg(t);
        self.int(r, v);
        r
    }

    pub fn mov(&mut self, dst: Reg, src: Reg) {
        self.op(Opcode::Mov { dst, src });
    }
    pub fn add(&mut self, dst: Reg, a: Reg, b: Reg) {
        self.op(Opcode::Add { dst, a, b });
    }
    pub fn sub(&mut self, dst: Reg, a: Reg, b: Reg) {
        self.op(Opcode::Sub { dst, a, b });
    }
    pub fn mul(&mut self, dst: Reg, a: Reg, b: Reg) {
        self.op(Opcode::Mul { dst, a, b });
    }

    // --------------------------------------------------------------- fields/calls

    /// `dst = obj.field` (object or virtual), with a null check on `obj`.
    pub fn get(&mut self, dst: Reg, obj: Reg, field: &str) -> Result<()> {
        let f = self.code.field(self.reg_type(obj), field)?;
        self.op(Opcode::NullCheck { reg: obj });
        self.op(Opcode::Field { dst, obj, field: f });
        Ok(())
    }

    /// New register holding `obj.field`, typed like the field.
    pub fn get_new(&mut self, obj: Reg, field: &str) -> Result<Reg> {
        let ot = self.reg_type(obj);
        let f = self.code.field(ot, field)?;
        let t = self.code.field_type(ot, f)?;
        let dst = self.reg(t);
        self.get(dst, obj, field)?;
        Ok(dst)
    }

    /// `obj.field = src` (object or virtual), with a null check on `obj`.
    pub fn set(&mut self, obj: Reg, field: &str, src: Reg) -> Result<()> {
        let f = self.code.field(self.reg_type(obj), field)?;
        self.op(Opcode::NullCheck { reg: obj });
        self.op(Opcode::SetField { obj, field: f, src });
        Ok(())
    }

    /// `dst = fun(args...)`
    pub fn call(&mut self, dst: Reg, fun: RefFun, args: &[Reg]) {
        self.op(call(dst, fun, args));
    }

    /// Call and return a new register typed like the function's return type.
    pub fn call_new(&mut self, fun: RefFun, args: &[Reg]) -> Result<Reg> {
        let ret = self.code.func_type(fun)?.ret;
        let dst = self.reg(ret);
        self.call(dst, fun, args);
        Ok(dst)
    }

    /// Print a line to stdout (`Sys.println`), e.g.
    /// `f.print(&[Print::Str("y = "), Print::Val(y)])`. Values of any type are converted with
    /// `Std.string`. Useful to trace injected code at runtime.
    pub fn print(&mut self, parts: &[Print]) -> Result<()> {
        let dyn_t = self.code.ty_dyn();
        let void = self.code.ty_void();
        let std_string = self.code.method("Std", "string")?;
        let concat = self.code.method("String", "__add__")?;
        let println = self.code.method("Sys", "println")?;

        let acc = self.string_obj("")?;
        for part in parts {
            let piece = match part {
                Print::Str(s) => self.string_obj(s)?,
                Print::Val(r) => {
                    let d = if crate::validate::kind(self.code, self.reg_type(*r)) == crate::validate::Kind::Ptr {
                        *r
                    } else {
                        let d = self.reg(dyn_t);
                        self.op(Opcode::ToDyn { dst: d, src: *r });
                        d
                    };
                    self.call_new(std_string, &[d])?
                }
            };
            self.call(acc, concat, &[acc, piece]);
        }
        let out = self.reg(void);
        self.call(out, println, &[acc]);
        Ok(())
    }

    /// Mark the function as belonging to a class (only affects naming in dumps/traces).
    pub fn parent(&mut self, t: RefType) {
        self.parent = Some(t);
    }

    /// Resolve labels, append the function to the bytecode and return its findex.
    pub fn finish(mut self) -> Result<RefFun> {
        for (i, l) in std::mem::take(&mut self.fixups) {
            let target = self.labels[l.0].ok_or_else(|| anyhow!("label {l:?} never placed in {}", self.name))?;
            let offs = jump_offsets_mut(&mut self.ops[i]);
            if offs.len() != 1 {
                bail!("op {i} in {} is not a simple jump", self.name);
            }
            for o in offs {
                *o = target as i32 - i as i32 - 1;
            }
        }
        if !matches!(self.ops.last(), Some(Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::JAlways { .. })) {
            bail!("function {} does not end with a return", self.name);
        }
        let _ = self.ret;

        let findex = self.code.next_findex();
        let file = self.code.debug_file(&format!("mosa/{}", self.name));
        let name = self.code.string(&self.name);
        let debug_info = (0..self.ops.len()).map(|i| (file, i + 1)).collect();
        self.code.bc.functions.push(Function {
            name,
            t: self.t,
            findex,
            regs: self.regs,
            ops: self.ops,
            debug_info: Some(debug_info),
            assigns: Some(Vec::new()),
            parent: self.parent,
        });
        self.code.touched.push(findex);
        Ok(findex)
    }
}
