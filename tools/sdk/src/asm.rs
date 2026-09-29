//! A small assembler for writing new HashLink functions from Rust.
//!
//! ```ignore
//! let f64_ = code.ty_f64();
//! let void = code.ty_void();
//! let mut f = FnBuilder::new(code, "example/clamp", &[point_ty], void);
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
//! file named after the function (`openlina/<name>`) with the op index as the line, so a crash
//! inside injected code shows up clearly in HashLink stack traces.

use anyhow::{anyhow, bail, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefFun, RefGlobal, RefType, Reg, ValBool};

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

    /// New register holding a class' static object (`pkg.$Class`, where static fields live), e.g.
    /// `let main = f.static_obj("fish.system.Main")?; let i = f.get_new(main, "i")?;`.
    pub fn static_obj(&mut self, class: &str) -> Result<Reg> {
        let t = self.code.class(class)?;
        let g = self.code.obj(t)?.global.0;
        if g == 0 {
            bail!("class {class} has no static object");
        }
        let global = RefGlobal(g - 1);
        let st = self.code.bc.globals[global.0];
        let r = self.reg(st);
        self.op(Opcode::GetGlobal { dst: r, global });
        Ok(r)
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

    /// New register holding a `String` built from pieces: literals and values of any type
    /// (converted with `Std.string`).
    pub fn string_of(&mut self, parts: &[Print]) -> Result<Reg> {
        let dyn_t = self.code.ty_dyn();
        let std_string = self.code.method("Std", "string")?;
        let concat = self.code.method("String", "__add__")?;
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
        Ok(acc)
    }

    /// Print a line to stdout (`Sys.println`), e.g.
    /// `f.print(&[Print::Str("y = "), Print::Val(y)])`. Useful to trace injected code at runtime.
    pub fn print(&mut self, parts: &[Print]) -> Result<()> {
        let void = self.code.ty_void();
        let println = self.code.method("Sys", "println")?;
        let line = self.string_of(parts)?;
        let out = self.reg(void);
        self.call(out, println, &[line]);
        Ok(())
    }

    // --------------------------------------------------------------- objects/arrays

    /// Cast `src` to type `to`: `ToVirtual` for virtual types, `SafeCast` otherwise (a runtime
    /// checked downcast). Returns `src` itself if it already has that type.
    pub fn cast(&mut self, src: Reg, to: RefType) -> Reg {
        if self.reg_type(src) == to {
            return src;
        }
        let dst = self.reg(to);
        if matches!(self.code.bc.types[to.0], hlbc::types::Type::Virtual { .. }) {
            self.op(Opcode::ToVirtual { dst, src });
        } else {
            self.op(Opcode::SafeCast { dst, src });
        }
        dst
    }

    /// Allocate an object of `class` without calling a constructor (call an init method yourself).
    pub fn new_obj(&mut self, class: &str) -> Result<Reg> {
        let t = self.code.class(class)?;
        let dst = self.reg(t);
        self.op(Opcode::New { dst });
        Ok(dst)
    }

    /// `arr` as an `hl.types.ArrayObj`. Fields typed `hl.types.ArrayDyn` (e.g. `Picker.insts`)
    /// hold an `ArrayObj` at runtime; the game's code casts them the same way before use.
    pub fn as_array_obj(&mut self, arr: Reg) -> Result<Reg> {
        let obj_t = self.code.class("hl.types.ArrayObj")?;
        match self.code.type_name(self.reg_type(arr)).as_str() {
            "hl.types.ArrayObj" => Ok(arr),
            "hl.types.ArrayDyn" | "hl.types.ArrayBase" | "Dyn" => Ok(self.cast(arr, obj_t)),
            other => bail!("{other} is not an object array"),
        }
    }

    /// New register holding a type value (`Type` opcode), e.g. for `alloc_array`.
    pub fn type_value(&mut self, t: RefType) -> Reg {
        let tt = self.code.intern_type(hlbc::types::Type::Type);
        let r = self.reg(tt);
        self.op(Opcode::Type { dst: r, ty: t });
        r
    }

    /// New Haxe array (`hl.types.ArrayObj`) of `elem`-typed values holding `items`, built like the
    /// compiler does: native `alloc_array(type, n)`, `SetArray`s, then `ArrayObj.alloc`.
    pub fn new_array_obj(&mut self, elem: RefType, items: &[Reg]) -> Result<Reg> {
        let alloc_array = self.code.native("alloc_array")?;
        let array_obj_t = self.code.class("hl.types.ArrayObj")?;
        let wrap = self
            .code
            .bc
            .functions
            .iter()
            .find(|f| {
                f.t.as_fun(&self.code.bc).is_some_and(|t| {
                    t.ret == array_obj_t && t.args.len() == 1 && matches!(self.code.bc.types[t.args[0].0], hlbc::types::Type::Array)
                })
            })
            .map(|f| f.findex)
            .ok_or_else(|| anyhow!("ArrayObj.alloc not found"))?;
        let ty = self.type_value(elem);
        let n = self.const_i32(items.len() as i32);
        let native = self.call_new(alloc_array, &[ty, n])?;
        for (i, it) in items.iter().enumerate() {
            let idx = self.const_i32(i as i32);
            self.op(Opcode::SetArray { array: native, index: idx, src: *it });
        }
        self.call_new(wrap, &[native])
    }

    /// New empty `hl.types.ArrayBytes_Float` (`ArrayBase.allocF64(alloc_bytes(0), 0)`).
    pub fn empty_f64_array(&mut self) -> Result<Reg> {
        let alloc_bytes = self.code.native("alloc_bytes")?;
        let alloc_f64 = self.code.method("hl.types.ArrayBase", "allocF64")?;
        let zero = self.const_i32(0);
        let bytes = self.call_new(alloc_bytes, &[zero])?;
        self.call_new(alloc_f64, &[bytes, zero])
    }

    /// `arr.length` of a Haxe object array (`ArrayObj`, or an `ArrayDyn` holding one).
    pub fn array_len(&mut self, arr: Reg) -> Result<Reg> {
        let arr = self.as_array_obj(arr)?;
        self.get_new(arr, "length")
    }

    /// `arr[idx]` of a Haxe object array, cast to `elem`. No bounds check: stay below `length`.
    pub fn array_get(&mut self, arr: Reg, idx: Reg, elem: RefType) -> Result<Reg> {
        let arr = self.as_array_obj(arr)?;
        let native = self.get_new(arr, "array")?;
        let dyn_t = self.code.ty_dyn();
        let v = self.reg(dyn_t);
        self.op(Opcode::GetArray { dst: v, array: native, index: idx });
        Ok(self.cast(v, elem))
    }

    /// `for (i in 0...count) body(i)`. `count` is read once.
    pub fn for_range(&mut self, count: Reg, mut body: impl FnMut(&mut Self, Reg) -> Result<()>) -> Result<()> {
        let i = self.const_i32(0);
        let (head, end) = (self.label(), self.label());
        self.place(head);
        self.jge(i, count, end);
        body(self, i)?;
        self.op(Opcode::Incr { dst: i });
        self.jmp(head);
        self.place(end);
        Ok(())
    }

    /// Jump to `l` unless the `String` in `s` equals `lit` (null check, length check and native
    /// `string_compare`, which is how the game compiles string equality).
    pub fn jstr_ne(&mut self, s: Reg, lit: &str, l: Label) -> Result<()> {
        let cmp = self.code.native("string_compare")?;
        let bytes_t = self.code.ty_bytes();
        self.jnull(s, l);
        let len = self.get_new(s, "length")?;
        let want = self.const_i32(lit.encode_utf16().count() as i32);
        self.jne(len, want, l);
        let a = self.get_new(s, "bytes")?;
        let b = self.reg(bytes_t);
        self.string(b, lit);
        let r = self.call_new(cmp, &[a, b, len])?;
        let zero = self.const_i32(0);
        self.jne(r, zero, l);
        Ok(())
    }

    /// End the process with `code` (native `sys_exit`).
    pub fn exit(&mut self, code: i32) -> Result<()> {
        let f = self.code.native("sys_exit")?;
        let c = self.const_i32(code);
        self.call_new(f, &[c])?;
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
        let file = self.code.debug_file(&format!("openlina/{}", self.name));
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
