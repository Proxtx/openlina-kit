//! `infinite-ammo`: items never run out. Every shot leaves the slot's ammo as it was.
//!
//! Vanilla: firing an item (`EvSheet_gameplay` closure fn@8545, source L1146-1147) first does
//! `if (slot.ammo > 0) slot.ammo = slot.ammo - 1` on the selected slot (`OClass_b_item`), then
//! dispatches the item by name (and the core `item_use` hook, which the kit's items use). When an
//! item's use fails, small closures give the shot back with `slot.ammo = slot.ammo + 1` (L3288,
//! L3663, L3680; L2958 as `ammo - -1`). An item whose ammo reaches 0 can't be used any more.
//!
//! The mod guards each of those writes (`edit::guard_op`): `infinite-ammo/keep(slot)` returns
//! true and the write is skipped, so the count stays where the item started (ammo-boost's boosted
//! value included) and refunds don't pile up. The sites are found by meaning (a `SetField
//! slot.ammo = r` right after `r = slot.ammo ± 1` on a `b_item`, in the game's own code) and their
//! number is pinned, so a game update fails the build instead of patching the wrong op.
//!
//! Left alone: where ammo comes from (`baseAmmo`, ammo-boost), the HUD (it shows the unchanging
//! count), item blocks in levels (taking one swaps its ammo with the slot's as usual), and
//! everything an item does.

use anyhow::{ensure, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit;
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefFun, Reg};
use openlina_sdk::{Code, ModConfig};

/// `slot.ammo - 1` sites (using a shot) and `slot.ammo + 1` sites (refunds) in build 22056877.
const EXPECTED_USES: usize = 1;
const EXPECTED_REFUNDS: usize = 4;

fn main() {
    openlina_sdk::run_mod(apply)
}

/// A `SetField slot.ammo = r` where `r = slot.ammo + delta`.
struct Site {
    findex: RefFun,
    /// The SetField op.
    at: usize,
    slot: Reg,
    delta: i32,
}

fn find_sites(code: &Code) -> Result<Vec<Site>> {
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let mut sites = Vec::new();
    for fun in &code.bc.functions {
        // Only the game's own code (not functions injected by other mods).
        if code.func_location(fun).is_some_and(|l| l.starts_with("openlina/")) {
            continue;
        }
        for at in edit::find_field_access(code, fun, "ammo", true) {
            let Opcode::SetField { obj, src, .. } = fun.ops[at] else { continue };
            if at < 3 || fun.regs[obj.0 as usize] != b_item_t {
                continue;
            }
            // r = slot.ammo; c = ±1; r = r ± c; slot.ammo = r
            let (read, konst, arith) = (&fun.ops[at - 3], &fun.ops[at - 2], &fun.ops[at - 1]);
            let Opcode::Field { dst: r0, obj: o0, .. } = *read else { continue };
            if o0 != obj || r0 != src || !edit::is_field(code, fun, read, "ammo") {
                continue;
            }
            let Opcode::Int { dst: c, ptr } = *konst else { continue };
            let k = code.bc.ints[ptr.0];
            let delta = match *arith {
                Opcode::Add { dst, a, b } if dst == src && a == src && b == c => k,
                Opcode::Sub { dst, a, b } if dst == src && a == src && b == c => -k,
                _ => continue,
            };
            if delta.abs() == 1 {
                sites.push(Site { findex: fun.findex, at, slot: obj, delta });
            }
        }
    }
    Ok(sites)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let trace = cfg.bool("trace", false)?;

    let sites = find_sites(code)?;
    let uses = sites.iter().filter(|s| s.delta < 0).count();
    let refunds = sites.len() - uses;
    ensure!(
        uses == EXPECTED_USES && refunds == EXPECTED_REFUNDS,
        "expected {EXPECTED_USES} `ammo - 1` and {EXPECTED_REFUNDS} `ammo + 1` sites on item slots, found {uses} and {refunds} (game update?)"
    );

    // keep(slot, delta) -> true: the write is skipped
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let (bool_t, i32_t) = (code.ty_bool(), code.ty_i32());
    let mut f = FnBuilder::new(code, "infinite-ammo/keep", &[b_item_t, i32_t], bool_t);
    let (slot, delta) = (f.arg(0), f.arg(1));
    if trace {
        let quiet = f.label();
        f.jnull(slot, quiet);
        let ammo = f.get_new(slot, "ammo")?;
        let item = f.get_new(slot, "item")?;
        f.jnull(item, quiet);
        let ty = f.get_new(item, "type")?;
        f.jnull(ty, quiet);
        let name = f.get_new(ty, "name")?;
        f.print(&[
            Print::Str("[infinite-ammo] "),
            Print::Val(name),
            Print::Str(": ammo stays "),
            Print::Val(ammo),
            Print::Str(" (the game's change: "),
            Print::Val(delta),
            Print::Str(")"),
        ])?;
        f.place(quiet);
    }
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.ret(yes);
    let keep = f.finish()?;

    // Guard each SetField, from the last op to the first so earlier indices stay valid.
    let mut sites = sites;
    sites.sort_by_key(|s| std::cmp::Reverse(s.at));
    for s in &sites {
        let ptr = code.int(s.delta);
        let fun = code.func_mut(s.findex)?;
        let (ok, d) = (edit::add_reg(fun, bool_t), edit::add_reg(fun, i32_t));
        // d = delta (for the trace), then `if (keep(slot, d)) skip the write`
        edit::insert_ops(fun, s.at, vec![Opcode::Int { dst: d, ptr }], edit::Incoming::ToInserted);
        edit::guard_op(fun, s.at + 1, ok, keep, &[s.slot, d]);
    }
    Ok(())
}
