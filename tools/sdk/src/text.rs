//! Texts shown by the game go through `Localisation.loc(key)` (translations from `loc.dat`, keys
//! like `TOOL_BOX` for item labels). [`set`] provides a text for a key through the core `loc`
//! hook, for every language.

use anyhow::Result;
use hlbc::opcodes::Opcode;

use crate::{hooks, Code};

/// Show `value` wherever the game looks up `key` (e.g. `TOOL_PORTAL` for the item `portal`).
pub fn set(code: &mut Code, key: &str, value: &str) -> Result<()> {
    let mut f = hooks::handler(code, "loc", &format!("text/{key}"))?;
    let k = f.arg(0);
    let not = f.label();
    f.jstr_ne(k, key, not)?;
    let v = f.string_obj(value)?;
    f.ret(v);
    f.place(not);
    let ret_t = f.reg_type(k);
    let r = f.reg(ret_t);
    f.op(Opcode::Null { dst: r });
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, "loc", h)
}
