//! `ammo-boost`: every item the player gets starts with `factor` times its normal ammo.
//!
//! Vanilla: each item type in `itemManager.itemPool` is a record `{aimType, baseAmmo, name,
//! secondLayer}`. Whenever the game hands an item to something that holds ammo (a HUD slot
//! `OClass_b_item`, or an item block `OClass_item_block` in the level), it copies the ammo
//! straight from the item type, always as the same two ops:
//!
//! ```text
//! Field    rA = rType.baseAmmo
//! SetField holder.ammo = rA
//! ```
//!
//! This happens in 11 places of build 22056877:
//! - `EvSheet_manager_ev.manage` (run flow, L233/L254) and `rollItemsRaw` (editor Play / level
//!   preview / level start in gameplay mode 2, L499/L508): the item slots
//! - `EvSheet_instancing_ev` closures (L287/L293, L332): the item slots
//! - `EvSheet_gameplay` closure (L6147) and `mld.objects.ItemBlock.createInstance`: item blocks
//!   placed in a level (their ammo is swapped with the selected slot when the player takes one)
//! - `StateSerializer.fromBin` (L215/L221): restoring a level's saved state (retries, replays)
//!
//! The mod inserts `rA = ammo_boost(rA, rType)` between the two ops. `ammo_boost` returns
//! `rA * factor` for positive values and leaves 0 (empty) and negative values alone.
//!
//! Left alone:
//! - ammo consumption and refunds (`ammo - 1`, `ammo + 1` on shoot and return), and the swap
//!   between slot and item block (it moves the already boosted value)
//! - `baseAmmo` itself, so the item pool and serialized item data stay vanilla
//! - ammo that doesn't come from `baseAmmo`: `OClass_test_item` objects copy their own
//!   level-authored `ammo` into slot 0 (`EvSheet_gameplay` closure L565; the title screen uses
//!   them), and the `main` layout's preset slots (their ammo comes from the layout data)
//!
//! Every site is found by meaning (a `baseAmmo` read whose value is stored into an `ammo` field
//! by the next op), and the total count is pinned, so a game update fails the build.

use anyhow::{ensure, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::{self, Incoming};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefFun, Reg};
use openlina_sdk::{hooks, Code, ModConfig};

/// Number of `holder.ammo = type.baseAmmo` sites in the vanilla game (build 22056877).
const EXPECTED_SITES: usize = 11;

fn main() {
    openlina_sdk::run_mod(apply)
}

/// One `Field dst = obj.baseAmmo` immediately followed by `SetField _.ammo = dst`.
struct Site {
    findex: RefFun,
    at: usize,
    dst: Reg,
    obj: Reg,
}

fn find_sites(code: &Code) -> Vec<Site> {
    let mut sites = Vec::new();
    for fun in &code.bc.functions {
        // Skip functions injected by earlier mods (e.g. the test harness): only vanilla code.
        if code.func_location(fun).is_some_and(|l| l.starts_with("openlina/")) {
            continue;
        }
        let writes = edit::find_field_access(code, fun, "ammo", true);
        if writes.is_empty() {
            continue;
        }
        for at in edit::find_field_access(code, fun, "baseAmmo", false) {
            let Opcode::Field { dst, obj, .. } = fun.ops[at] else { continue };
            if !writes.contains(&(at + 1)) {
                continue;
            }
            if let Opcode::SetField { src, .. } = fun.ops[at + 1] {
                if src == dst {
                    sites.push(Site { findex: fun.findex, at, dst, obj });
                }
            }
        }
    }
    sites
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let factor = cfg.i64("factor", 2)?;
    let trace = cfg.bool("trace", false)?;
    ensure!((0..=1000).contains(&factor), "factor must be between 0 and 1000, got {factor}");

    let sites = find_sites(code);
    ensure!(
        sites.len() == EXPECTED_SITES,
        "expected {EXPECTED_SITES} `ammo = type.baseAmmo` sites, found {} (game update?)",
        sites.len()
    );

    // All sites read `baseAmmo` from the same item type record.
    let item_t = code.func(sites[0].findex)?.regs[sites[0].obj.0 as usize];
    for s in &sites {
        let fun = code.func(s.findex)?;
        ensure!(
            fun.regs[s.obj.0 as usize] == item_t,
            "{}: `baseAmmo` read from an unexpected type {}",
            code.op_location(fun, s.at).unwrap_or_default(),
            code.type_name(fun.regs[s.obj.0 as usize])
        );
    }

    // ammo_boost(ammo, type) -> ammo * factor for positive ammo
    let i32_t = code.ty_i32();
    let mut f = FnBuilder::new(code, "ammo-boost/boost", &[i32_t, item_t], i32_t);
    let (ammo, ty) = (f.arg(0), f.arg(1));
    let keep = f.label();
    let zero = f.const_i32(0);
    f.jle(ammo, zero, keep);
    let k = f.const_i32(factor as i32);
    let out = f.reg(i32_t);
    f.mul(out, ammo, k);
    if trace {
        let name = f.get_new(ty, "name")?;
        f.print(&[
            Print::Str("[ammo-boost] "),
            Print::Val(name),
            Print::Str(": ammo "),
            Print::Val(ammo),
            Print::Str(" -> "),
            Print::Val(out),
        ])?;
    }
    f.ret(out);
    f.place(keep);
    f.ret(ammo);
    let boost = f.finish()?;

    // Patch from the last site to the first, so indices within a function stay valid.
    for s in sites.iter().rev() {
        let fun = code.func_mut(s.findex)?;
        let set = s.at + 1;
        // Nothing may jump straight to the SetField (it would skip the baseAmmo read).
        let incoming = fun.ops.iter().enumerate().any(|(p, op)| edit::jump_targets(op, p).contains(&set));
        ensure!(!incoming, "fn@{} op {set}: unexpected jump into an ammo site", s.findex.0);
        edit::insert_ops(fun, set, vec![edit::call(s.dst, boost, &[s.dst, s.obj])], Incoming::ToOriginal);
    }

    if trace {
        report_slots(code)?;
    }
    Ok(())
}

/// With `trace`: on tick 1 of every gameplay layout, print what the item slots actually hold,
/// compared with the item type's vanilla `baseAmmo`:
/// `[ammo-boost] level start slot <k>: <item> ammo <n> = <baseAmmo> x <n / baseAmmo>`.
/// This reads the game state, not what the patch computed, so tests can check the result even
/// though the rolled items differ from run to run. Empty items (`baseAmmo` 0) are skipped.
fn report_slots(code: &mut Code) -> Result<()> {
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let mut f = hooks::handler(code, "tick", "ammo-boost/report")?;
    let layout = f.arg(1);
    let done = f.label();
    let tick = f.get_new(layout, "currentTick")?;
    let one = f.const_i32(1);
    f.jne(tick, one, done);
    let st = f.static_obj("fish.system.Main")?;
    let main = f.get_new(st, "i")?;
    let game = f.get_new(main, "game")?;
    let mgr = f.get_new(game, "ev_manager_ev")?;
    let picker = f.get_new(mgr, "b_item")?;
    let slots = f.get_new(picker, "insts")?;
    let n = f.array_len(slots)?;
    f.for_range(n, |f, i| {
        let skip = f.label();
        let slot = f.array_get(slots, i, b_item_t)?;
        let item = f.get_new(slot, "item")?;
        f.jnull(item, skip);
        let ty = f.get_new(item, "type")?;
        f.jnull(ty, skip);
        let name = f.get_new(ty, "name")?;
        let ammo = f.get_new(slot, "ammo")?;
        let base = f.get_new(ty, "baseAmmo")?;
        let zero = f.const_i32(0);
        f.jle(base, zero, skip); // "nothing" and other empty items
        let i32_t = f.code().ty_i32();
        let (ratio, rem) = (f.reg(i32_t), f.reg(i32_t));
        f.op(Opcode::SDiv { dst: ratio, a: ammo, b: base });
        f.op(Opcode::SMod { dst: rem, a: ammo, b: base });
        let odd = f.label();
        f.jne(rem, zero, odd);
        f.print(&[
            Print::Str("[ammo-boost] level start slot "),
            Print::Val(i),
            Print::Str(": "),
            Print::Val(name),
            Print::Str(" ammo "),
            Print::Val(ammo),
            Print::Str(" = "),
            Print::Val(base),
            Print::Str(" x "),
            Print::Val(ratio),
        ])?;
        f.jmp(skip);
        f.place(odd);
        f.print(&[
            Print::Str("[ammo-boost] level start slot "),
            Print::Val(i),
            Print::Str(": "),
            Print::Val(name),
            Print::Str(" ammo "),
            Print::Val(ammo),
            Print::Str(", base "),
            Print::Val(base),
            Print::Str(" (not a multiple)"),
        ])?;
        f.place(skip);
        Ok(())
    })?;
    f.place(done);
    f.ret_void();
    let handler = f.finish()?;
    hooks::subscribe(code, "tick", handler)
}
