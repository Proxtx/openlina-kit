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
//!             case "coin":   coinedgecheck(o);           // kind 1 (deleting collects the fruit)
//!             case "frog":   frogland_count++; destroy;  // kind 3 (the count stays)
//!             case "player": // handled by player_death
//!             default:       o.sprite.destroy();         // kind 0
//!         }
//! }
//! for (o in secondary_physics.insts)
//!     if (outside(o.sprite.position, 0)) o.sprite.destroy();  // kind 2, edgewith = 0
//! ```
//!
//! Each of the four calls is guarded with `if (edge_exit(...)) skip the call;`. With no
//! subscriber the hook returns false and the game behaves exactly as vanilla.
//!
//! ## `player_edge(pos, margin, sheet, player) -> Bool`
//! Before the objects, the same `update` tests the players (source L16200-16213): a player in
//! state "normal" outside [25,575]x[25,313] (x only outside boss mode, no `edgewith`) gets
//! `player_death(playerId)`. That call is guarded the same way; `update`'s other `player_death`
//! call (explosions, L10403) is not. It runs on the hub too (layout `help`), where walking off the
//! right edge is how a run starts.

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::edit::{
    add_reg, call, call_target, expect_one, find_calls, find_field_access, insert_ops, insert_ops_with_exits, is_field,
    is_set_field, next_match, prev_match, Exit, Incoming,
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
    // frogs: `frogland_count++` (alive ones), then destroy: the destroy is guarded, the count stays
    let frog_count =
        expect_one(find_field_access(code, &fun, "frogland_count", true), "update writes `.frogland_count`")?;
    ensure!(frog_count > ew_op, "the frogland_count write is not after the edge test");
    let destroy_frog =
        next_match(&fun, frog_count, |op| calls(op, destroy)).context("destroy() after frogland_count++")?;
    ensure!(
        destroy_frog - frog_count <= 6,
        "the frog destroy() is {} ops after frogland_count++",
        destroy_frog - frog_count
    );
    let frog = destroyed_object(code, &fun, destroy_frog)?;

    // ---- anchors: secondary_physics loop
    let sp =
        expect_one(find_field_access(code, &fun, "secondary_physics", false), "update reads `.secondary_physics`")?;
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

    // ---- guard the four calls, last site first so earlier indices stay valid
    let mut sites = [
        Site { site: destroy_default, kind: 0, pos, ew: Some(ew), margin, sheet, obj: item },
        Site { site: coin_call, kind: 1, pos, ew: Some(ew), margin, sheet, obj: item },
        Site { site: destroy_frog, kind: 3, pos, ew: Some(ew), margin, sheet, obj: frog },
        Site { site: destroy_secondary, kind: 2, pos: pos2, ew: None, margin: margin2, sheet: sheet2, obj: item2 },
    ];
    sites.sort_by_key(|s| std::cmp::Reverse(s.site));
    let zero_c = code.float(0.0);
    let kind_c: Vec<_> = (0..4).map(|k| code.int(k)).collect();
    let physics_fields: Vec<_> =
        sites.iter().map(|s| code.field(fun.regs[s.obj.0 as usize], "physics")).collect::<Result<_>>()?;
    let f = code.func_mut(update)?;
    let ok = add_reg(f, bool_t);
    let kind_r = add_reg(f, i32_t);
    let zero = add_reg(f, f64_t);
    let phys = add_reg(f, physics_t);
    for (s, &field) in sites.iter().zip(&physics_fields) {
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

    // ---- player_edge: its site comes before the three above, so its indices are still valid
    let p = player_site(code, &fun)?;
    ensure!(sites.iter().all(|s| s.site > p.site), "the player edge test is not before the object loops");
    let player_t = fun.regs[p.player.0 as usize];
    let player_edge = hooks::define(code, "player_edge", &[pos_t, f64_t, sheet_t, player_t], bool_t)?;
    let f = code.func_mut(update)?;
    let ok = add_reg(f, bool_t);
    insert_ops_with_exits(
        f,
        p.site,
        vec![call(ok, player_edge, &[p.pos, p.margin, p.sheet, p.player]), Opcode::JTrue { cond: ok, offset: 0 }],
        &[Exit { op: 1, target: p.site + 1 }],
        Incoming::ToInserted,
    );

    // ---- tick at the start of update(this, layout)
    let dst = add_reg(f, void);
    insert_ops(f, 0, vec![call(dst, tick, &[Reg(0), Reg(1)])], Incoming::ToOriginal);

    modifier_hooks(code)?;
    item_hooks(code)?;
    packs_hook(code)?;
    loc_hook(code)
}

/// The edge test's `player_death` call and the hook's arguments.
struct PlayerSite {
    site: usize,
    pos: Reg,
    margin: Reg,
    sheet: Reg,
    player: Reg,
}

fn player_site(code: &mut Code, fun: &Function) -> Result<PlayerSite> {
    let death = code.method(SHEET, "player_death")?;
    // update calls player_death twice: explosions (L10403) and the edge test (L16213), which comes
    // right after the `.bossMode` read of its x test.
    let sites: Vec<usize> = find_calls(fun, death)
        .into_iter()
        .filter(|&c| (c.saturating_sub(12)..c).any(|i| is_field(code, fun, &fun.ops[i], "bossMode")))
        .collect();
    let site = expect_one(sites, "update calls player_death() right after a `.bossMode` read (the edge test)")?;
    let boss = prev_match(fun, site, |op| is_field(code, fun, op, "bossMode")).unwrap();
    let Opcode::Field { obj: sheet, .. } = fun.ops[boss] else { unreachable!() };
    let (_, args) = call_target(&fun.ops[site]).unwrap();
    ensure!(args[0] == sheet, "player_death() is not called on the sheet whose `.bossMode` is tested");
    // pos = player.sprite.position, then the y test `338 - margin`.
    let pos_op = prev_match(fun, site, |op| is_field(code, fun, op, "position")).context("player `.position`")?;
    ensure!(site - pos_op <= 40, "player `.position` read is {} ops before player_death()", site - pos_op);
    let Opcode::Field { dst: pos, obj: sprite, .. } = fun.ops[pos_op] else { bail!("`.position` is not a Field") };
    let sprite_load = prev_match(fun, pos_op, |op| matches!(op, Opcode::Field { dst, .. } if *dst == sprite))
        .context("player `.sprite` load")?;
    ensure!(is_field(code, fun, &fun.ops[sprite_load], "sprite"), "`.position` is not read from a `.sprite`");
    let Opcode::Field { obj: player, .. } = fun.ops[sprite_load] else { unreachable!() };
    let margin = margin_after(fun, pos_op)?;
    ensure!(fun.regs[margin.0 as usize] == code.ty_f64(), "margin is not an F64");
    ensure!(fun.regs[sheet.0 as usize] == code.class(SHEET)?, "player_death()'s `this` is not the gameplay sheet");
    Ok(PlayerSite { site, pos, margin, sheet, player })
}

/// `packs(packManager)`: in the PackManager constructor, right after the local and downloaded
/// level packs were loaded (before `loadHidden`), so mods can add level packs. See
/// `openlina_sdk::levels`.
fn packs_hook(code: &mut Code) -> Result<()> {
    let ctor = code.method("fish.system.PackManager", "__constructor__")?;
    let downloaded = code.method("fish.system.PackManager", "loadDownloadedLevels")?;
    let pm_t = code.class("fish.system.PackManager")?;
    let void = code.ty_void();
    let hook = hooks::define(code, "packs", &[pm_t], void)?;
    let at =
        expect_one(find_calls(code.func(ctor)?, downloaded), "the PackManager constructor calls loadDownloadedLevels")?;
    let f = code.func_mut(ctor)?;
    let r = add_reg(f, void);
    insert_ops(f, at + 1, vec![call(r, hook, &[Reg(0)])], Incoming::ToInserted);
    Ok(())
}

/// `loc(key) -> String`: at the start of `Localisation.loc(this, key, args)`,
/// `r = loc(key); if (r != null) return r;` so mods can add texts (item labels, …).
fn loc_hook(code: &mut Code) -> Result<()> {
    let loc = code.method("Localisation", "loc")?;
    let t = code.func_type(loc)?.clone();
    ensure!(t.args.len() == 3, "Localisation.loc has an unexpected signature");
    let (string_t, ret_t) = (t.args[1], t.ret);
    ensure!(string_t == ret_t, "Localisation.loc does not return its key type");
    let hook = hooks::define(code, "loc", &[string_t], ret_t)?;
    let f = code.func_mut(loc)?;
    let r = add_reg(f, ret_t);
    insert_ops(
        f,
        0,
        vec![call(r, hook, &[Reg(1)]), Opcode::JNull { reg: r, offset: 1 }, Opcode::Ret { ret: r }],
        Incoming::ToOriginal,
    );
    Ok(())
}

/// `item_pool` and `item_use`, see `openlina_sdk::items`.
fn item_hooks(code: &mut Code) -> Result<()> {
    let void = code.ty_void();
    let bool_t = code.ty_bool();

    // ItemManager.initBaseItems(items) builds `itemPool` from the item objects: call
    // item_pool(this) before every return, so mods can add their item types.
    let init = code.method("fish.system.ItemManager", "initBaseItems")?;
    let im_t = code.class("fish.system.ItemManager")?;
    let pool_hook = hooks::define(code, "item_pool", &[im_t], void)?;
    let rets = openlina_sdk::edit::find(code.func(init)?, |_, op| matches!(op, Opcode::Ret { .. }));
    ensure!(!rets.is_empty(), "initBaseItems has no return");
    let f = code.func_mut(init)?;
    let r = add_reg(f, void);
    for &at in rets.iter().rev() {
        insert_ops(f, at, vec![call(r, pool_hook, &[Reg(0)])], Incoming::ToInserted);
    }

    // EvSheet_gameplay.shoot(playerId) runs a closure over the item slots (`foreach(b_item, …)`,
    // source L1140-3720) that finds the player's selected slot, does `if (ammo > 0) ammo--` and
    // then the item's behavior (`if (slot.item.type.name == "unbox") … else if …`). Right after the
    // ammo line: `if (item_use(slot, sheet, player, crosshair)) return false;` (what the
    // closure returns after vanilla behaviors too).
    let shoot = code.method("fish.game.evsheet.EvSheet_gameplay", "shoot")?;
    let foreach = code.method("fish.game.evsheet.EvSheet", "foreach")?;
    let sfun = code.func(shoot)?.clone();
    let fe = expect_one(find_calls(&sfun, foreach), "shoot calls foreach")?;
    let Some((_, fargs)) = call_target(&sfun.ops[fe]) else { unreachable!() };
    let clo_op = prev_match(&sfun, fe, |op| matches!(op, Opcode::InstanceClosure { dst, .. } if *dst == fargs[2]))
        .context("the foreach closure")?;
    let Opcode::InstanceClosure { fun: closure, .. } = sfun.ops[clo_op] else { unreachable!() };

    let cfun = code.func(closure)?.clone();
    let ammo_set = expect_one(find_field_access(code, &cfun, "ammo", true), "the item closure writes `.ammo` once")?;
    let Opcode::SetField { obj: slot, .. } = cfun.ops[ammo_set] else { unreachable!() };
    let n = cfun.ops.len();
    ensure!(
        matches!(cfun.ops[n - 1], Opcode::Ret { .. }) && matches!(cfun.ops[n - 2], Opcode::Bool { .. }),
        "the item closure doesn't end with `return <bool>`"
    );
    let exit = n - 2;
    // The closure's environment (reg0) is an enum: .0 the sheet, .1 the player picker, .2 the
    // player's crosshair_point instances.
    let env_t = cfun.regs[0];
    let params = match &code.bc.types[env_t.0] {
        openlina_sdk::hlbc::types::Type::Enum { constructs, .. } => constructs[0].params.clone(),
        _ => bail!("the item closure's environment is not an enum"),
    };
    ensure!(params.len() >= 3, "unexpected item closure environment");
    let slot_t = cfun.regs[slot.0 as usize];
    let use_hook = hooks::define(code, "item_use", &[slot_t, params[0], params[1], params[2]], bool_t)?;
    let f = code.func_mut(closure)?;
    let (sheet, player, cross, ok) =
        (add_reg(f, params[0]), add_reg(f, params[1]), add_reg(f, params[2]), add_reg(f, bool_t));
    let ef = |dst, field: usize| Opcode::EnumField {
        dst,
        value: Reg(0),
        construct: openlina_sdk::hlbc::types::RefEnumConstruct(0),
        field: openlina_sdk::hlbc::types::RefField(field),
    };
    insert_ops_with_exits(
        f,
        ammo_set + 1,
        vec![
            ef(sheet, 0),
            ef(player, 1),
            ef(cross, 2),
            call(ok, use_hook, &[slot, sheet, player, cross]),
            Opcode::JTrue { cond: ok, offset: 0 },
        ],
        &[Exit { op: 4, target: exit }],
        Incoming::ToInserted,
    );
    Ok(())
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
    let set =
        next_match(&fun, m, |op| is_set_field(code, &fun, op, "animFrame")).context("animFrame after `.modifier`")?;
    ensure!(set - m <= 12, "animFrame is set {} ops after `.modifier`", set - m);
    let Opcode::SetField { obj: sprite, .. } = fun.ops[set] else { unreachable!() };
    let load =
        prev_match(&fun, set, |op| matches!(op, Opcode::Field { dst, .. } if *dst == sprite)).context("sprite load")?;
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

fn calls(op: &Opcode, target: RefFun) -> bool {
    call_target(op).is_some_and(|(f, _)| f == target)
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

/// The object whose `.sprite` the `destroy()` call at `at` destroys.
fn destroyed_object(code: &Code, fun: &Function, at: usize) -> Result<Reg> {
    let (_, args) = call_target(&fun.ops[at]).context("destroy() is not a call")?;
    let load = prev_match(fun, at, |op| matches!(op, Opcode::Field { dst, .. } if *dst == args[0]))
        .context("sprite load before destroy()")?;
    ensure!(is_field(code, fun, &fun.ops[load], "sprite"), "destroy() arg is not a `.sprite`");
    let Opcode::Field { obj, .. } = fun.ops[load] else { unreachable!() };
    Ok(obj)
}
