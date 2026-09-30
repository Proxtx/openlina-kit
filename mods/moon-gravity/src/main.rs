//! `moon-gravity`: a modifier. In levels that roll it, gravity is weaker (`factor`, default 0.5):
//! Lina, boxes, fruits and everything else fall slower, and Lina jumps higher and longer.
//!
//! It registers a modifier (`openlina_sdk::modifiers`, key `moon-gravity`) with its own HUD icon
//! (`assets/images/openlina/moon-gravity.png`, from `art/modifier.toml`), rolled like the vanilla
//! modifiers (also in "dx" runs). With the option `always`, it applies in every level instead
//! (then it isn't registered as a modifier and shows no icon).
//!
//! Vanilla: gameplay layouts set the Box2D world gravity to 0 when they start (closure in
//! `EvSheet_gameplay.setupEvents`, L6086); gravity is the game's own. Inside the "gravity" event
//! group of `EvSheet_gameplay.update` (L16413-16472) it computes a strength `g = 1.05`, or
//! `0.05` while any `low_grav` object exists, and then applies to every dynamic body in
//! `physics_obj` the force
//! `(0, personal_gravity * flipModifier * water * g + water_lift) * (1 - is_overlap_nograv)`
//! (plus the `z_up` zone lift) with `body_apply_force`. Lina is a `physics_obj` too; her jump
//! sets a velocity, so weaker gravity makes her rise higher and fall slower.
//!
//! The mod multiplies `g` by `moon-gravity/scale(layout)` right after the `low_grav` choice,
//! once per tick: `factor` in levels that rolled the modifier (or always, with `always`),
//! otherwise 1. Everything that scales with `g` follows: the vanilla low-gravity objects stack
//! with it (0.05 × factor), flipped gravity stays flipped, zero-gravity zones stay zero.
//!
//! Left alone:
//! - the water lift and damping, the `z_up` zone lift (not multiplied by `g` in vanilla)
//! - Lina while digging (the cocoon item), whose fall speed the game integrates separately from
//!   `personal_gravity` (L16522)
//! - `secondary_physics` objects and `Bullet` behaviors (their own gravity, not the gravity group)
//! - speed caps: Box2D caps motion at about 100 units per tick, which moon gravity never reaches
//!
//! Anchors: the one `low_grav` read in `update`; the `Float` constant before it (1.05) is `g`;
//! the `JNull` after it jumps to the join point, where the scaling goes; and `g` must be
//! multiplied into the force before the next `body_apply_force` call.

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::{self, Incoming};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::modifiers::{self, Modifier};
use openlina_sdk::{Code, ModConfig};

/// Vanilla's gravity strength in `EvSheet_gameplay.update` (build 22056877).
const VANILLA_G: f64 = 1.05;

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let factor = cfg.f64("factor", 0.5)?;
    let always = cfg.bool("always", false)?;
    let trace = cfg.bool("trace", false)?;
    ensure!(
        factor.is_finite() && (0.0..=4.0).contains(&factor),
        "moon-gravity: factor must be between 0 and 4 (1 = vanilla), got {factor}"
    );

    let modifier = if always {
        None
    } else {
        Some(modifiers::register(
            code,
            &Modifier {
                key: "moon-gravity",
                icon: "images/openlina/moon-gravity.png",
                size: (16.0, 16.0),
                in_dx: true,
            },
        )?)
    };

    // --- find the gravity strength and the place to scale it
    let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
    let apply_force = code.native("body_apply_force")?;
    let f64_t = code.ty_f64();
    let fun = code.func(update)?;
    let low = edit::expect_one(edit::find_field_access(code, fun, "low_grav", false), "`.low_grav` read in update")?;
    let g_at = edit::prev_match(fun, low, |op| matches!(op, Opcode::Float { .. }))
        .context("no gravity constant before the `low_grav` read")?;
    let Opcode::Float { dst: g, ptr } = fun.ops[g_at] else { unreachable!() };
    let vanilla = code.bc.floats[ptr.0];
    ensure!(
        (vanilla - VANILLA_G).abs() < 1e-9,
        "gravity constant before `low_grav` is {vanilla}, expected {VANILLA_G} (game update?)"
    );
    let jnull = edit::next_match(fun, low, |op| matches!(op, Opcode::JNull { .. }))
        .context("no `low_grav.insts == null` test")?;
    let join = edit::jump_targets(&fun.ops[jnull], jnull)[0];
    ensure!(
        matches!(fun.ops[join - 1], Opcode::Mov { dst, .. } if dst == g),
        "op {} before the join isn't the low-gravity `g = 0.05` (game update?)",
        join - 1
    );
    let force = edit::next_match(fun, join, |op| edit::call_target(op).is_some_and(|(t, _)| t == apply_force))
        .context("no `body_apply_force` after the gravity strength")?;
    let uses = (join..force).filter(|&i| matches!(fun.ops[i], Opcode::Mul { a, b, .. } if a == g || b == g)).count();
    ensure!(uses == 1, "expected `g` multiplied into the force once before `body_apply_force`, found {uses}");
    ensure!(fun.regs[g.0 as usize] == f64_t, "gravity strength register is not F64");
    let layout = Reg(1);
    let layout_t = fun.regs[layout.0 as usize];
    ensure!(code.type_name(layout_t) == "fish.system.Layout", "update's second argument is not the layout");

    // --- scale(layout) -> F64: factor while the modifier applies, else 1
    let mut f = FnBuilder::new(code, "moon-gravity/scale", &[layout_t], f64_t);
    let lay = f.arg(0);
    let off = f.label();
    if let Some(id) = modifier {
        let active = modifiers::is_active(&mut f, id)?;
        f.jfalse(active, off);
    }
    let k = f.const_f64(factor);
    if trace {
        // once per layout, at its first tick
        let quiet = f.label();
        let tick = f.get_new(lay, "currentTick")?;
        let one = f.const_i32(1);
        f.jne(tick, one, quiet);
        let name = f.get_new(lay, "name")?;
        f.print(&[Print::Str("[moon-gravity] "), Print::Val(name), Print::Str(" tick 1: gravity x"), Print::Val(k)])?;
        f.place(quiet);
    }
    f.ret(k);
    f.place(off);
    let one = f.const_f64(1.0);
    f.ret(one);
    let scale = f.finish()?;

    // --- g = g * scale(layout), on both paths (normal and low gravity)
    let fun = code.func_mut(update)?;
    let tmp = edit::add_reg(fun, f64_t);
    let incoming = fun.ops.iter().enumerate().filter(|(p, op)| edit::jump_targets(op, *p).contains(&join)).count();
    if incoming == 0 {
        bail!("nothing jumps to the join after the `low_grav` test (game update?)");
    }
    edit::insert_ops(
        fun,
        join,
        vec![edit::call(tmp, scale, &[layout]), Opcode::Mul { dst: g, a: g, b: tmp }],
        Incoming::ToInserted,
    );
    Ok(())
}
