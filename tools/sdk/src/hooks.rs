//! Hook points: named functions injected into the game by the `core` mod.
//!
//! Instead of patching the same game code, mods subscribe handlers to hooks. Several mods can
//! subscribe to one hook without knowing about each other, and they keep working when another
//! mod changes the game code around the hook site.
//!
//! Two kinds of hooks:
//! - **notify** hooks return `Void`: every handler runs.
//! - **handled** hooks return `Bool`: handlers run until one returns `true` ("handled"), then the
//!   hook returns `true` and the game skips its vanilla behavior. If none does, vanilla runs.
//! - **value** hooks return an object (e.g. `String`): handlers run until one returns non-null,
//!   which is used instead of vanilla's value. `null` means "not mine".
//!
//! Handlers of mods applied later run first. Hooks are identified in the bytecode by the debug
//! file of their function, `openlina/hook/<name>`, which survives serialization.
//!
//! The hooks the core mod defines are listed in [`CORE_HOOKS`].

use anyhow::{bail, ensure, Context, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{RefFun, RefType};

use crate::asm::FnBuilder;
use crate::edit::{add_reg, call, insert_ops, Incoming};
use crate::validate::{kind, Kind};
use crate::Code;

/// A hook the core mod provides.
pub struct HookInfo {
    pub name: &'static str,
    pub args: &'static str,
    pub returns: &'static str,
    pub doc: &'static str,
}

/// Hooks defined by the `core` mod (see `mods/core`).
pub const CORE_HOOKS: &[HookInfo] = &[
    HookInfo {
        name: "tick",
        args: "(sheet: EvSheet_gameplay, layout: Layout)",
        returns: "Void",
        doc: "Start of every gameplay tick (`EvSheet_gameplay.update`). Runs in levels, and for the first \
              ticks of the title screen.",
    },
    HookInfo {
        name: "edge_exit",
        args: "(pos: Vector2Default, edgewith: F64, margin: F64, sheet: EvSheet_gameplay, kind: I32, physics: beh.Physics)",
        returns: "Bool",
        doc: "An object is outside the screen and vanilla is about to delete it. kind 0 = physics object, \
              1 = coin (fruit; deleting it is how levels are won), 2 = secondary physics object. Return true \
              to keep it (e.g. after moving it). `physics` is the object's physics behavior (velocity: \
              getVelocityX/Y, setVelocity).",
    },
    HookInfo {
        name: "modifier_pool",
        args: "(pool: ArrayBytes_Int, dx: Bool)",
        returns: "Void",
        doc: "A level's modifier is about to be drawn from `pool` (LevelManager.rollRaw and reroll). Push \
              modifier ids to make them possible. Use `openlina_sdk::modifiers::register` instead of this hook.",
    },
    HookInfo {
        name: "modifier_icon",
        args: "(icon: OClass_optionthingos)",
        returns: "Bool",
        doc: "The HUD modifier icon is about to show the current level's modifier. Return true after setting \
              the icon's sprite yourself. Use `openlina_sdk::modifiers::register` instead of this hook.",
    },
    HookInfo {
        name: "loc",
        args: "(key: String)",
        returns: "String",
        doc: "A text is looked up by key (`Localisation.loc`, e.g. item labels `TOOL_<NAME>`). Return the text, or \
              null to let the game look it up. Use `openlina_sdk::text::set` instead of this hook.",
    },
    HookInfo {
        name: "item_pool",
        args: "(itemManager: ItemManager)",
        returns: "Void",
        doc: "The item pool (`itemManager.itemPool`) was just built. Push item types to add them. Use \
              `openlina_sdk::items::register` instead of this hook.",
    },
    HookInfo {
        name: "item_use",
        args: "(slot: OClass_b_item, sheet: EvSheet_gameplay, player: Picker, crosshair: ArrayObj)",
        returns: "Bool",
        doc: "The player fires the item in `slot` (ammo already decremented). `crosshair` holds the player's \
              crosshair_point objects (the aim point). Return true when the item is yours and handled: the game's \
              own item behaviors are skipped. Use `openlina_sdk::items` helpers.",
    },
];

fn debug_file(name: &str) -> String {
    format!("openlina/hook/{name}")
}

/// Find a hook function. Fails with a clear message if the core mod was not applied.
pub fn find(code: &Code, name: &str) -> Result<RefFun> {
    let file = debug_file(name);
    let files = code.bc.debug_files.as_ref();
    for f in &code.bc.functions {
        let Some((fi, _)) = f.debug_info.as_ref().and_then(|d| d.first()).copied() else { continue };
        if files.and_then(|fs| fs.get(fi)).is_some_and(|s| s.as_str() == file) {
            return Ok(f.findex);
        }
    }
    bail!("hook `{name}` not found: the `core` mod must be applied first (add `requires = [\"core\"]`)")
}

/// Argument types and return type of a hook, to build handlers with [`FnBuilder::new`].
pub fn signature(code: &Code, name: &str) -> Result<(Vec<RefType>, RefType)> {
    let t = code.func_type(find(code, name)?)?;
    Ok((t.args.clone(), t.ret))
}

/// Start building a handler for a hook: a new function with the hook's signature.
pub fn handler<'a>(code: &'a mut Code, hook: &str, name: &str) -> Result<FnBuilder<'a>> {
    let (args, ret) = signature(code, hook)?;
    Ok(FnBuilder::new(code, name, &args, ret))
}

/// Define a hook (used by the core mod). The default body does nothing (`Void`) or returns
/// `false` (`Bool`, i.e. "not handled").
pub fn define(code: &mut Code, name: &str, args: &[RefType], ret: RefType) -> Result<RefFun> {
    ensure!(find(code, name).is_err(), "hook `{name}` is already defined");
    let mut f = FnBuilder::new(code, &format!("hook/{name}"), args, ret);
    match kind(f.code(), ret) {
        Kind::Void => f.ret_void(),
        Kind::Bool => {
            let r = f.reg(ret);
            f.bool(r, false);
            f.ret(r);
        }
        Kind::Ptr => {
            let r = f.reg(ret);
            f.op(Opcode::Null { dst: r });
            f.ret(r);
        }
        k => bail!("hooks return Void, Bool or an object, not {k:?}"),
    }
    f.finish()
}

/// Subscribe `handler` (same signature as the hook) to a hook.
pub fn subscribe(code: &mut Code, name: &str, handler: RefFun) -> Result<()> {
    let hook = find(code, name)?;
    let (hargs, hret) = signature(code, name)?;
    let t = code.func_type(handler).context("handler")?.clone();
    ensure!(
        t.args == hargs && t.ret == hret,
        "handler {} does not match hook `{name}` {}",
        code.func_name(handler),
        code.type_name(code.func(hook)?.t)
    );
    let args: Vec<_> = (0..hargs.len() as u32).map(hlbc::types::Reg).collect();
    let ret_kind = kind(code, hret);
    let f = code.func_mut(hook)?;
    let r = add_reg(f, hret);
    let ops = match ret_kind {
        Kind::Void => vec![call(r, handler, &args)],
        // r = handler(args); if (!r) skip the return; return r;
        Kind::Bool => vec![call(r, handler, &args), Opcode::JFalse { cond: r, offset: 1 }, Opcode::Ret { ret: r }],
        // r = handler(args); if (r == null) skip the return; return r;
        Kind::Ptr => vec![call(r, handler, &args), Opcode::JNull { reg: r, offset: 1 }, Opcode::Ret { ret: r }],
        k => bail!("unsupported hook return kind {k:?}"),
    };
    insert_ops(f, 0, ops, Incoming::ToOriginal);
    Ok(())
}
