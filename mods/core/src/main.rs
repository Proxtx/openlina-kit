//! `core`: the OpenLina core. Defines the hook points other mods subscribe to
//! (see `openlina_sdk::hooks::CORE_HOOKS`) and wires them into the game.
//!
//! ## `tick(sheet, layout)`
//! Called at the start of `EvSheet_gameplay.update`, every gameplay tick.
//!
//! ## `edge_exit(pos, edgewith, margin, sheet, kind) -> Bool`
//! `EvSheet_gameplay.update` (source ~L16195-16268) deletes objects that leave the screen:
//!
//! ```haxe
//! for (o in physics_obj.insts) {
//!     var pos = o.sprite.position, ew = o.edgewith;
//!     if (outside(pos, ew, margin = 25))                 // x only tested if !bossMode
//!         switch (o.type) {
//!             case "coin":   coinedgecheck(o);           // kind 1 (win condition)
//!             case "frog":   frogland_count++; destroy;  // not hooked
//!             case "player": // handled by player_death
//!             default:       o.sprite.destroy();         // kind 0
//!         }
//! }
//! for (o in secondary_physics.insts)
//!     if (outside(o.sprite.position, 0)) o.sprite.destroy();  // kind 2, edgewith = 0
//! ```
//!
//! Each of the three calls is guarded with `if (edge_exit(...)) skip the call;`. With no
//! subscriber the hook returns false and the game behaves exactly as vanilla.

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::edit::{
    add_reg, call, call_target, expect_one, find_calls, find_field_access, insert_ops, insert_ops_with_exits,
    next_match, prev_match, Exit, Incoming,
};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{Function, RefFun, Reg};
use openlina_sdk::{hooks, Code};

const SHEET: &str = "fish.game.evsheet.EvSheet_gameplay";

fn main() {
    openlina_sdk::run_mod(|code, _cfg| apply(code))
}

/// A call to guard: `site` is its op index, the rest are the hook arguments.
struct Site {
    site: usize,
    kind: i32,
    pos: Reg,
    ew: Option<Reg>,
    margin: Reg,
    sheet: Reg,
}

fn apply(code: &mut Code) -> Result<()> {
    let update = code.method(SHEET, "update")?;
    let destroy = code.method("fish.system.Sprite", "destroy")?;
    let coinedgecheck = code.method(SHEET, "coinedgecheck")?;
    let fun = code.func(update)?.clone();

    // ---- anchors: physics_obj loop
    let ew_op = expect_one(find_field_access(code, &fun, "edgewith", false), "update reads `.edgewith`")?;
    let Opcode::Field { dst: ew, obj: item, .. } = fun.ops[ew_op] else { bail!("`.edgewith` read is not a Field op") };
    let pos = position_before(code, &fun, ew_op)?;
    let margin = margin_after(&fun, ew_op)?;
    let sheet = boss_mode_owner_after(code, &fun, ew_op)?;
    let destroy_default = next_match(&fun, ew_op, |op| calls(op, destroy)).context("destroy() after the edge test")?;
    ensure_destroys_sprite_of(code, &fun, destroy_default, item)?;
    let coin_call = expect_one(find_calls(&fun, coinedgecheck), "update calls coinedgecheck()")?;
    ensure!(coin_call > ew_op, "coinedgecheck() is not after the edge test");

    // ---- anchors: secondary_physics loop
    let sp = expect_one(find_field_access(code, &fun, "secondary_physics", false), "update reads `.secondary_physics`")?;
    let pos_op = next_match(&fun, sp, |op| is_field(code, &fun, op, "position")).context("secondary `.position`")?;
    let Opcode::Field { dst: pos2, .. } = fun.ops[pos_op] else { unreachable!() };
    let margin2 = margin_after(&fun, pos_op)?;
    let sheet2 = boss_mode_owner_after(code, &fun, pos_op)?;
    let destroy_secondary = next_match(&fun, pos_op, |op| calls(op, destroy)).context("secondary destroy()")?;

    let pos_t = fun.regs[pos.0 as usize];
    let sheet_t = fun.regs[sheet.0 as usize];
    ensure!(fun.regs[pos2.0 as usize] == pos_t && fun.regs[sheet2.0 as usize] == sheet_t, "loop register types differ");
    ensure!(fun.regs[0] == sheet_t, "update's `this` is not the gameplay sheet");

    // ---- hooks
    let layout_t = code.class("fish.system.Layout")?;
    let (void, bool_t, f64_t, i32_t) = (code.ty_void(), code.ty_bool(), code.ty_f64(), code.ty_i32());
    let tick = hooks::define(code, "tick", &[sheet_t, layout_t], void)?;
    let edge = hooks::define(code, "edge_exit", &[pos_t, f64_t, f64_t, sheet_t, i32_t], bool_t)?;

    // ---- guard the three calls, last site first so earlier indices stay valid
    let mut sites = vec![
        Site { site: destroy_default, kind: 0, pos, ew: Some(ew), margin, sheet },
        Site { site: coin_call, kind: 1, pos, ew: Some(ew), margin, sheet },
        Site { site: destroy_secondary, kind: 2, pos: pos2, ew: None, margin: margin2, sheet: sheet2 },
    ];
    sites.sort_by_key(|s| std::cmp::Reverse(s.site));
    let zero_c = code.float(0.0);
    let kind_c: Vec<_> = (0..3).map(|k| code.int(k)).collect();
    let f = code.func_mut(update)?;
    let ok = add_reg(f, bool_t);
    let kind_r = add_reg(f, i32_t);
    let zero = add_reg(f, f64_t);
    for s in &sites {
        let mut ops = vec![Opcode::Int { dst: kind_r, ptr: kind_c[s.kind as usize] }];
        let ew = match s.ew {
            Some(r) => r,
            None => {
                ops.push(Opcode::Float { dst: zero, ptr: zero_c });
                zero
            }
        };
        ops.push(call(ok, edge, &[s.pos, ew, s.margin, s.sheet, kind_r]));
        ops.push(Opcode::JTrue { cond: ok, offset: 0 });
        let jump = ops.len() - 1;
        insert_ops_with_exits(f, s.site, ops, &[Exit { op: jump, target: s.site + 1 }], Incoming::ToInserted);
    }

    // ---- tick at the start of update(this, layout)
    let dst = add_reg(f, void);
    insert_ops(f, 0, vec![call(dst, tick, &[Reg(0), Reg(1)])], Incoming::ToOriginal);
    Ok(())
}

fn calls(op: &Opcode, target: RefFun) -> bool {
    call_target(op).is_some_and(|(f, _)| f == target)
}

fn is_field(code: &Code, fun: &Function, op: &Opcode, name: &str) -> bool {
    match op {
        Opcode::Field { obj, field, .. } => code.field(fun.regs[obj.0 as usize], name).is_ok_and(|f| f.0 == field.0),
        _ => false,
    }
}

/// The `pos = sprite.position` read right before the `edgewith` read.
fn position_before(code: &Code, fun: &Function, at: usize) -> Result<Reg> {
    let i = prev_match(fun, at, |op| is_field(code, fun, op, "position")).context("`.position` read")?;
    ensure!(at - i <= 4, "`.position` read is {} ops before `.edgewith`, expected <= 4", at - i);
    let Opcode::Field { dst, .. } = fun.ops[i] else { unreachable!() };
    Ok(dst)
}

/// The margin register: the edge test starts with `hi = 338; hi = hi - margin`.
fn margin_after(fun: &Function, at: usize) -> Result<Reg> {
    let i = next_match(fun, at, |op| matches!(op, Opcode::Sub { .. })).context("`338 - margin`")?;
    ensure!(i - at <= 6, "edge test `Sub` is {} ops away, expected <= 6", i - at);
    let Opcode::Sub { b, .. } = fun.ops[i] else { unreachable!() };
    Ok(b)
}

/// The register holding the event sheet, found through its `.bossMode` read.
fn boss_mode_owner_after(code: &Code, fun: &Function, at: usize) -> Result<Reg> {
    let i = next_match(fun, at, |op| is_field(code, fun, op, "bossMode")).context("`.bossMode` read")?;
    ensure!(i - at <= 20, "`.bossMode` read is {} ops away, expected <= 20", i - at);
    let Opcode::Field { obj, .. } = fun.ops[i] else { unreachable!() };
    Ok(obj)
}

/// Check that `destroy(x)` at `at` destroys `item.sprite`.
fn ensure_destroys_sprite_of(code: &Code, fun: &Function, at: usize, item: Reg) -> Result<()> {
    let (_, args) = call_target(&fun.ops[at]).unwrap();
    let load = prev_match(fun, at, |op| matches!(op, Opcode::Field { dst, .. } if *dst == args[0]))
        .context("sprite load before destroy()")?;
    ensure!(is_field(code, fun, &fun.ops[load], "sprite"), "destroy() arg is not a `.sprite`");
    let Opcode::Field { obj, .. } = fun.ops[load] else { unreachable!() };
    ensure!(obj == item, "destroy() is not called on the edge-tested object");
    Ok(())
}
