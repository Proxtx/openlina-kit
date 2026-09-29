//! `core`: the OpenLina core. Defines the hook points other mods subscribe to
//! (see `openlina_sdk::hooks::CORE_HOOKS`) and wires them into the game.
//!
//! ## `tick(sheet, layout)`
//! Called at the start of `EvSheet_gameplay.update`, every gameplay tick.
//!
//! ## `edge_exit(pos, edgewith, margin, sheet, kind, physics) -> Bool`
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
    /// The object whose `physics` behavior is passed to the hook.
    obj: Reg,
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
    let Opcode::Field { dst: pos2, obj: sprite2, .. } = fun.ops[pos_op] else { unreachable!() };
    let sprite_load = prev_match(&fun, pos_op, |op| matches!(op, Opcode::Field { dst, .. } if *dst == sprite2))
        .context("secondary `.sprite` load")?;
    let Opcode::Field { obj: item2, .. } = fun.ops[sprite_load] else { unreachable!() };
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
    let physics_t = code.class("fish.system.beh.Physics")?;
    let edge = hooks::define(code, "edge_exit", &[pos_t, f64_t, f64_t, sheet_t, i32_t, physics_t], bool_t)?;
    let item_t = fun.regs[item.0 as usize];
    let item2_t = fun.regs[item2.0 as usize];
    let physics_field = code.field(item_t, "physics")?;
    let physics_field2 = code.field(item2_t, "physics")?;

    // ---- guard the three calls, last site first so earlier indices stay valid
    let mut sites = vec![
        Site { site: destroy_default, kind: 0, pos, ew: Some(ew), margin, sheet, obj: item },
        Site { site: coin_call, kind: 1, pos, ew: Some(ew), margin, sheet, obj: item },
        Site { site: destroy_secondary, kind: 2, pos: pos2, ew: None, margin: margin2, sheet: sheet2, obj: item2 },
    ];
    sites.sort_by_key(|s| std::cmp::Reverse(s.site));
    let zero_c = code.float(0.0);
    let kind_c: Vec<_> = (0..3).map(|k| code.int(k)).collect();
    let f = code.func_mut(update)?;
    let ok = add_reg(f, bool_t);
    let kind_r = add_reg(f, i32_t);
    let zero = add_reg(f, f64_t);
    let phys = add_reg(f, physics_t);
    for s in &sites {
        let field = if s.obj == item { physics_field } else { physics_field2 };
        let mut ops = vec![
            Opcode::Int { dst: kind_r, ptr: kind_c[s.kind as usize] },
            Opcode::Field { dst: phys, obj: s.obj, field },
        ];
        let ew = match s.ew {
            Some(r) => r,
            None => {
                ops.push(Opcode::Float { dst: zero, ptr: zero_c });
                zero
            }
        };
        ops.push(call(ok, edge, &[s.pos, ew, s.margin, s.sheet, kind_r, phys]));
        ops.push(Opcode::JTrue { cond: ok, offset: 0 });
        let jump = ops.len() - 1;
        insert_ops_with_exits(f, s.site, ops, &[Exit { op: jump, target: s.site + 1 }], Incoming::ToInserted);
    }

    // ---- tick at the start of update(this, layout)
    let dst = add_reg(f, void);
    insert_ops(f, 0, vec![call(dst, tick, &[Reg(0), Reg(1)])], Incoming::ToOriginal);

    modifier_hooks(code)
}

/// `modifier_pool` and `modifier_icon`, see `openlina_sdk::modifiers`.
fn modifier_hooks(code: &mut Code) -> Result<()> {
    let alloc_i32 = code.method("hl.types.ArrayBase", "allocI32")?;
    let pool_t = code.func_type(alloc_i32)?.ret;
    let (void, bool_t) = (code.ty_void(), code.ty_bool());
    let pool_hook = hooks::define(code, "modifier_pool", &[pool_t, bool_t], void)?;

    // LevelManager.rollRaw and .reroll build the pool in two branches:
    //   if (dx) pool = allocI32(…, 3) else pool = allocI32(…, 7);   <- call the hook at the join
    for name in ["rollRaw", "reroll"] {
        let fun_ref = code.method("fish.system.LevelManager", name)?;
        let fun = code.func(fun_ref)?.clone();
        let allocs = find_calls(&fun, alloc_i32);
        let first = *allocs.first().with_context(|| format!("{name}: no allocI32"))?;
        let Opcode::JAlways { offset } = fun.ops[first + 1] else { bail!("{name}: no join after the first pool") };
        let join = (first as i64 + 2 + offset as i64) as usize;
        let second = join - 1;
        ensure!(allocs.contains(&second), "{name}: the second pool doesn't end at the join");
        let (Some((_, _)), Opcode::Call2 { dst: p1, .. }, Opcode::Call2 { dst: p2, .. }) =
            (call_target(&fun.ops[first]), &fun.ops[first], &fun.ops[second])
        else {
            bail!("{name}: unexpected pool calls")
        };
        ensure!(p1 == p2, "{name}: the two pools go to different registers");
        let jf = prev_match(&fun, first, |op| matches!(op, Opcode::JFalse { .. })).context("dx branch")?;
        let Opcode::JFalse { cond: dx, offset } = fun.ops[jf] else { unreachable!() };
        ensure!(
            (jf as i64 + 1 + offset as i64) as usize == first + 2,
            "{name}: the dx test doesn't branch to the second pool"
        );
        let pool = *p1;
        let f = code.func_mut(fun_ref)?;
        let r = add_reg(f, void);
        insert_ops(f, join, vec![call(r, pool_hook, &[pool, dx])], Incoming::ToInserted);
    }

    // EvSheet_edge_ev.update sets the HUD modifier icon: `sprite.animFrame = modifier (9 -> 7)`.
    let icon_t = code.class("fish.game.oclass.OClass_optionthingos")?;
    let icon_hook = hooks::define(code, "modifier_icon", &[icon_t], bool_t)?;
    let edge = code.method("fish.game.evsheet.EvSheet_edge_ev", "update")?;
    let fun = code.func(edge)?.clone();
    let m = expect_one(find_field_access(code, &fun, "modifier", false), "edge_ev.update reads `.modifier`")?;
    let set = next_match(&fun, m, |op| is_set_field(code, &fun, op, "animFrame")).context("animFrame after `.modifier`")?;
    ensure!(set - m <= 12, "animFrame is set {} ops after `.modifier`", set - m);
    let Opcode::SetField { obj: sprite, .. } = fun.ops[set] else { unreachable!() };
    let load = prev_match(&fun, set, |op| matches!(op, Opcode::Field { dst, .. } if *dst == sprite)).context("sprite load")?;
    let Opcode::Field { obj: icon, .. } = fun.ops[load] else { unreachable!() };
    ensure!(fun.regs[icon.0 as usize] == icon_t, "icon register is not an OClass_optionthingos");
    let f = code.func_mut(edge)?;
    let ok = add_reg(f, bool_t);
    insert_ops_with_exits(
        f,
        set,
        vec![call(ok, icon_hook, &[icon]), Opcode::JTrue { cond: ok, offset: 0 }],
        &[Exit { op: 1, target: set + 1 }],
        Incoming::ToInserted,
    );
    Ok(())
}

fn is_set_field(code: &Code, fun: &Function, op: &Opcode, name: &str) -> bool {
    match op {
        Opcode::SetField { obj, field, .. } => code.field(fun.regs[obj.0 as usize], name).is_ok_and(|f| f.0 == field.0),
        _ => false,
    }
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
