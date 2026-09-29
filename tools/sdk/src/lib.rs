//! Helpers for reading, querying and patching the Mosa Lina HashLink bytecode (`hlboot.dat`).
//!
//! This crate wraps [`hlbc`] with:
//! - [`Code`]: name-based lookups (classes, fields, methods), constant/type interning
//! - [`asm::FnBuilder`]: an assembler for writing brand new functions with labels
//! - [`edit`]: finding opcodes, replacing them, inserting code with automatic jump relocation
//! - [`validate`]: a static checker that catches most broken patches before the JIT does
//! - [`hooks`]: find and subscribe to the hook points injected by the `core` mod
//! - [`runner`]: the `main` of every mod (bytecode in on stdin, patched bytecode out on stdout)
//! - [`manifest`]: `mod.toml` and `modpack.toml`, shared by the tools and the website
//!
//! Throughout this crate a function is identified by its *findex* ([`RefFun`]), the index
//! HashLink uses for both bytecode functions and natives.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
pub use hlbc;
use hlbc::types::{
    FunPtr, Function, RefField, RefFloat, RefFun, RefGlobal, RefInt, RefString, RefType, Type, TypeFun, TypeObj,
};
use hlbc::{Bytecode, Resolve, Str};

pub mod asm;
pub mod edit;
pub mod hooks;
pub mod manifest;
pub mod modifiers;
pub mod runner;
pub mod validate;

pub use runner::run_mod;

/// A mod's options: the defaults declared in its `mod.toml`, overridden by the user's modpack.
/// The host passes them to the patch in the `OPENLINA_OPTIONS` environment variable (TOML).
#[derive(Default, Clone, Debug)]
pub struct ModConfig {
    pub table: toml::Table,
}

impl ModConfig {
    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(Self { table: toml::from_str(s).context("parsing mod options")? })
    }

    pub fn bool(&self, key: &str, default: bool) -> Result<bool> {
        match self.table.get(key) {
            None => Ok(default),
            Some(toml::Value::Boolean(b)) => Ok(*b),
            Some(v) => bail!("option `{key}` must be a boolean, got {v}"),
        }
    }
    pub fn f64(&self, key: &str, default: f64) -> Result<f64> {
        match self.table.get(key) {
            None => Ok(default),
            Some(toml::Value::Float(f)) => Ok(*f),
            Some(toml::Value::Integer(i)) => Ok(*i as f64),
            Some(v) => bail!("option `{key}` must be a number, got {v}"),
        }
    }
    pub fn i64(&self, key: &str, default: i64) -> Result<i64> {
        match self.table.get(key) {
            None => Ok(default),
            Some(toml::Value::Integer(i)) => Ok(*i),
            Some(v) => bail!("option `{key}` must be an integer, got {v}"),
        }
    }
    pub fn str<'a>(&'a self, key: &str, default: &'a str) -> Result<&'a str> {
        match self.table.get(key) {
            None => Ok(default),
            Some(toml::Value::String(s)) => Ok(s),
            Some(v) => bail!("option `{key}` must be a string, got {v}"),
        }
    }
}

/// A loaded bytecode module.
pub struct Code {
    pub bc: Bytecode,
    /// Functions added or modified by patches, validated by [`validate::check_touched`].
    pub touched: Vec<RefFun>,
}

impl Code {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let bc = Bytecode::deserialize(&mut std::io::Cursor::new(bytes)).map_err(|e| anyhow!("{e}"))?;
        Ok(Self { bc, touched: Vec::new() })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(12 << 20);
        self.bc.serialize(&mut out).map_err(|e| anyhow!("{e}"))?;
        Ok(out)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path.as_ref(), self.to_bytes()?)
            .with_context(|| format!("writing {}", path.as_ref().display()))
    }

    /// Serialize and parse again. hlbc keeps private lookup tables (findex -> function) that
    /// are only rebuilt on load, so this must be called after adding functions if you want to
    /// look them up by findex with the hlbc API.
    pub fn reload(&mut self) -> Result<()> {
        let touched = std::mem::take(&mut self.touched);
        *self = Self::from_bytes(&self.to_bytes()?)?;
        self.touched = touched;
        Ok(())
    }

    // ------------------------------------------------------------------ constants

    pub fn string(&mut self, s: &str) -> RefString {
        match self.bc.strings.iter().position(|x| x.as_str() == s) {
            Some(i) => RefString(i),
            None => {
                self.bc.strings.push(Str::from_ref(s));
                RefString(self.bc.strings.len() - 1)
            }
        }
    }

    pub fn float(&mut self, f: f64) -> RefFloat {
        match self.bc.floats.iter().position(|x| x.to_bits() == f.to_bits()) {
            Some(i) => RefFloat(i),
            None => {
                self.bc.floats.push(f);
                RefFloat(self.bc.floats.len() - 1)
            }
        }
    }

    pub fn int(&mut self, v: i32) -> RefInt {
        match self.bc.ints.iter().position(|x| *x == v) {
            Some(i) => RefInt(i),
            None => {
                self.bc.ints.push(v);
                RefInt(self.bc.ints.len() - 1)
            }
        }
    }

    /// Index of a debug file name, added if missing. Used for debug info of injected code so
    /// stack traces point at the mod (e.g. `openlina/screen-wrap/wrap`).
    pub fn debug_file(&mut self, name: &str) -> usize {
        let files = self.bc.debug_files.get_or_insert_with(Vec::new);
        match files.iter().position(|x| x.as_str() == name) {
            Some(i) => i,
            None => {
                files.push(Str::from_ref(name));
                files.len() - 1
            }
        }
    }

    /// Raw string from the pool. Unlike hlbc's `Resolve`, index 0 is a real string here
    /// (in this game it is `"String"`), not "none".
    pub fn str(&self, s: RefString) -> &str {
        self.bc.strings.get(s.0).map(|x| x.as_str()).unwrap_or("")
    }

    /// Add a new global variable of type `t` (zero/null-initialized by the VM). Globals are
    /// the simplest way for injected code to keep state across calls.
    pub fn add_global(&mut self, t: RefType) -> RefGlobal {
        self.bc.globals.push(t);
        RefGlobal(self.bc.globals.len() - 1)
    }

    /// The text of a global initialized with a constant `String` (the game keeps most string
    /// literals, e.g. animation and object type names, in such globals).
    pub fn global_string(&self, g: RefGlobal) -> Option<&str> {
        let c = self.bc.constants.as_ref()?.iter().find(|c| c.global == g)?;
        match self.bc.types.get(self.bc.globals.get(g.0)?.0)? {
            Type::Obj(o) if self.str(o.name) == "String" => Some(self.str(RefString(*c.fields.first()?))),
            _ => None,
        }
    }

    // ---------------------------------------------------------------------- types

    /// Find a structurally equal type or append it.
    pub fn intern_type(&mut self, t: Type) -> RefType {
        match self.bc.types.iter().position(|x| *x == t) {
            Some(i) => RefType(i),
            None => {
                self.bc.types.push(t);
                RefType(self.bc.types.len() - 1)
            }
        }
    }

    pub fn ty_void(&mut self) -> RefType {
        self.intern_type(Type::Void)
    }
    pub fn ty_i32(&mut self) -> RefType {
        self.intern_type(Type::I32)
    }
    pub fn ty_f64(&mut self) -> RefType {
        self.intern_type(Type::F64)
    }
    pub fn ty_bool(&mut self) -> RefType {
        self.intern_type(Type::Bool)
    }
    pub fn ty_dyn(&mut self) -> RefType {
        self.intern_type(Type::Dyn)
    }
    pub fn ty_bytes(&mut self) -> RefType {
        self.intern_type(Type::Bytes)
    }

    /// A function type `(args) -> ret`.
    pub fn ty_fun(&mut self, args: &[RefType], ret: RefType) -> RefType {
        self.intern_type(Type::Fun(TypeFun { args: args.to_vec(), ret }))
    }

    /// Name of any type, in a compact readable form.
    pub fn type_name(&self, t: RefType) -> String {
        match &self.bc.types[t.0] {
            Type::Obj(o) | Type::Struct(o) => self.str(o.name).to_string(),
            Type::Null(inner) => format!("Null<{}>", self.type_name(*inner)),
            Type::Ref(inner) => format!("Ref<{}>", self.type_name(*inner)),
            Type::Packed(inner) => format!("Packed<{}>", self.type_name(*inner)),
            Type::Abstract { name } => self.str(*name).to_string(),
            Type::Enum { name, .. } => self.str(*name).to_string(),
            Type::Virtual { fields } => {
                let f: Vec<_> = fields.iter().map(|f| self.str(f.name).to_string()).collect();
                format!("virtual{{{}}}", f.join(","))
            }
            Type::Fun(f) | Type::Method(f) => {
                let a: Vec<_> = f.args.iter().map(|a| self.type_name(*a)).collect();
                format!("({}) -> {}", a.join(", "), self.type_name(f.ret))
            }
            other => format!("{other:?}"),
        }
    }

    /// Find a class (Obj type) by its fully qualified name, e.g. `fish.system.beh.Physics`.
    pub fn class(&self, name: &str) -> Result<RefType> {
        self.bc
            .types
            .iter()
            .position(|t| matches!(t, Type::Obj(o) if self.str(o.name) == name))
            .map(RefType)
            .ok_or_else(|| anyhow!("class `{name}` not found"))
    }

    pub fn obj(&self, t: RefType) -> Result<&TypeObj> {
        self.bc.types[t.0]
            .get_type_obj()
            .ok_or_else(|| anyhow!("type@{} ({}) is not an object", t.0, self.type_name(t)))
    }

    /// Index of a field on an object (including inherited fields) or virtual type, usable with
    /// the `Field`/`SetField` opcodes.
    pub fn field(&self, t: RefType, name: &str) -> Result<RefField> {
        let fields = match &self.bc.types[t.0] {
            Type::Obj(o) | Type::Struct(o) => &o.fields,
            Type::Virtual { fields } => fields,
            _ => bail!("type {} has no fields", self.type_name(t)),
        };
        fields
            .iter()
            .position(|f| self.str(f.name) == name)
            .map(RefField)
            .ok_or_else(|| anyhow!("field `{name}` not found on {}", self.type_name(t)))
    }

    /// Type of a field on an object or virtual type.
    pub fn field_type(&self, t: RefType, field: RefField) -> Result<RefType> {
        let fields = match &self.bc.types[t.0] {
            Type::Obj(o) | Type::Struct(o) => &o.fields,
            Type::Virtual { fields } => fields,
            _ => bail!("type {} has no fields", self.type_name(t)),
        };
        fields
            .get(field.0)
            .map(|f| f.t)
            .ok_or_else(|| anyhow!("field index {} out of range on {}", field.0, self.type_name(t)))
    }

    // ------------------------------------------------------------------ functions

    /// Find a function by class and name (instance method, bound closure or static).
    ///
    /// Instance methods are looked up in the class protos (walking up the hierarchy), static
    /// methods among the functions whose parent is the class' static type (`pkg.$Class`).
    pub fn method(&self, class: &str, name: &str) -> Result<RefFun> {
        if let Ok(mut t) = self.class(class) {
            loop {
                let o = self.obj(t)?;
                if let Some(p) = o.protos.iter().find(|p| self.str(p.name) == name) {
                    return Ok(p.findex);
                }
                for (f, fun) in &o.bindings {
                    if self.str(o.fields[f.0].name) == name {
                        return Ok(*fun);
                    }
                }
                match o.super_ {
                    Some(s) => t = s,
                    None => break,
                }
            }
        }
        let static_name = match class.rfind('.') {
            Some(i) => format!("{}.${}", &class[..i], &class[i + 1..]),
            None => format!("${class}"),
        };
        for f in &self.bc.functions {
            if self.str(f.name) != name {
                continue;
            }
            if let Some(p) = f.parent {
                let pn = self.type_name(p);
                if pn == class || pn == static_name {
                    return Ok(f.findex);
                }
            }
        }
        bail!("method `{class}.{name}` not found")
    }

    /// Find a native function by name, e.g. `sys_print`.
    pub fn native(&self, name: &str) -> Result<RefFun> {
        self.bc
            .natives
            .iter()
            .find(|n| self.str(n.name) == name)
            .map(|n| n.findex)
            .ok_or_else(|| anyhow!("native `{name}` not found"))
    }

    /// Get a function (with code) by findex.
    pub fn func(&self, f: RefFun) -> Result<&Function> {
        self.bc
            .functions
            .iter()
            .find(|fun| fun.findex == f)
            .ok_or_else(|| anyhow!("findex {} is not a bytecode function", f.0))
    }

    /// Mutable access to a function by findex. Marks it as touched for validation.
    pub fn func_mut(&mut self, f: RefFun) -> Result<&mut Function> {
        if !self.touched.contains(&f) {
            self.touched.push(f);
        }
        self.bc
            .functions
            .iter_mut()
            .find(|fun| fun.findex == f)
            .ok_or_else(|| anyhow!("findex {} is not a bytecode function", f.0))
    }

    /// Signature of any function or native.
    pub fn func_type(&self, f: RefFun) -> Result<&TypeFun> {
        if let Some(fun) = self.bc.functions.iter().find(|x| x.findex == f) {
            return fun.t.as_fun(&self.bc).ok_or_else(|| anyhow!("bad function type"));
        }
        if let Some(n) = self.bc.natives.iter().find(|x| x.findex == f) {
            return n.t.as_fun(&self.bc).ok_or_else(|| anyhow!("bad native type"));
        }
        bail!("unknown findex {}", f.0)
    }

    /// Next free findex (functions and natives share the index space).
    pub fn next_findex(&self) -> RefFun {
        let a = self.bc.functions.iter().map(|f| f.findex.0 + 1).max().unwrap_or(0);
        let b = self.bc.natives.iter().map(|n| n.findex.0 + 1).max().unwrap_or(0);
        RefFun(a.max(b))
    }

    /// Human readable `Class.method` name of a function.
    pub fn func_name(&self, f: RefFun) -> String {
        if let Some(fun) = self.bc.functions.iter().find(|x| x.findex == f) {
            return match fun.parent {
                Some(p) => format!("{}.{}", self.type_name(p), self.str(fun.name)),
                // Free functions have no name in the bytecode (hlbc reports string 0). Injected
                // functions are recognizable by their `openlina/<name>` debug file.
                None => match self.func_location(fun) {
                    Some(loc) if loc.starts_with("openlina/") => loc[9..loc.rfind(':').unwrap_or(loc.len())].to_string(),
                    _ if fun.name.0 == 0 => "<anonymous>".to_string(),
                    _ => self.str(fun.name).to_string(),
                },
            };
        }
        if let Some(n) = self.bc.natives.iter().find(|x| x.findex == f) {
            return format!("native {}@{}", self.str(n.name), self.str(n.lib));
        }
        format!("<unknown fn@{}>", f.0)
    }

    /// Source location (`file:line`) of a function's first instruction, if debug info is present.
    pub fn func_location(&self, fun: &Function) -> Option<String> {
        self.op_location(fun, 0)
    }

    /// Source location (`file:line`) of an instruction.
    pub fn op_location(&self, fun: &Function, op: usize) -> Option<String> {
        let (file, line) = *fun.debug_info.as_ref()?.get(op)?;
        let files = self.bc.debug_files.as_ref()?;
        Some(format!("{}:{}", files.get(file)?, line))
    }

    /// Resolve `FunPtr` for callers that need hlbc's API. Only valid for findexes that existed
    /// at load time (or after [`Code::reload`]).
    pub fn fun_ptr(&self, f: RefFun) -> FunPtr<'_> {
        self.bc.get(f)
    }
}
