//! `portal-gun`: a new item. Firing it places a blue portal, then an orange one, at the aim point
//! (alternating); anything that enters one comes out of the other with its momentum. Lina too.
//!
//! - The item type `portal` is added to the item pool with `openlina_sdk::items::register`
//!   (HUD icon `assets/images/openlina/portal-gun.png`).
//! - Firing: an `item_use` handler. The game has already decremented the ammo. The crosshair
//!   (`items::crosshair_pos`) is the aiming reticle just in front of Lina, so the portal goes
//!   `range` units from Lina through the reticle, clamped to the play field. Returning true skips
//!   the game's own item behaviors.
//! - Portals are plain `Sprite15` objects (`Layout.createObject(layout, "Sprite15", …)`) showing
//!   our own animations (`openlina_sdk::anims`), kept in globals with the portal positions.
//! - Teleporting: a `tick` handler checks every object of `physics_obj` (which includes the
//!   player): within `radius` of one portal, it is moved out of the other portal, offset along
//!   its direction of travel so it doesn't fall straight back in. Moving `sprite.position` keeps
//!   the Box2D velocity. A short per-object cooldown stops ping-ponging.
//! - Portals reset on every new level (layout tick 1). `preset` places both at level start.
//!
//! Static bodies (`physics.immovable`) never teleport.

use anyhow::{bail, Context, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefGlobal, RefType, Reg};
use openlina_sdk::items::{self, Aim, Item};
use openlina_sdk::{anims, hooks, Code, ModConfig};

const ITEM: &str = "portal";
const COOLDOWN: i32 = 45;
const COLORS: [&str; 2] = ["blue", "orange"];
/// A plain decorative object type (only the sprite behavior) that `Layout.createObject` can
/// create (plain `Sprite` is not registered in `ObjectClasses.createInstance`).
const SPRITE_TYPE: &str = "Sprite15";

fn main() {
    openlina_sdk::run_mod(apply)
}

/// Mod state, kept in globals.
struct State {
    x: [RefGlobal; 2],
    y: [RefGlobal; 2],
    sprite: [RefGlobal; 2],
    placed: RefGlobal,
    next: RefGlobal,
    cool_uid: RefGlobal,
    cool_tick: RefGlobal,
}

struct Opts {
    radius: f64,
    range: f64,
    preset: Option<[f64; 4]>,
    trace: bool,
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let ammo = cfg.i64("ammo", 4)? as i32;
    let preset = match cfg.str("preset", "")? {
        "" => None,
        s => {
            let v: Vec<f64> = s.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>().context("preset")?;
            let [a, b, c, d] = v[..] else { bail!("preset must be \"ax,ay,bx,by\"") };
            Some([a, b, c, d])
        }
    };
    let o = Opts { radius: cfg.f64("radius", 14.0)?, range: cfg.f64("range", 160.0)?, preset, trace: cfg.bool("trace", false)? };

    items::register(
        code,
        &Item { name: ITEM, label: "Portal Gun", ammo, aim: Aim::Long, second_layer: false, icon: "images/openlina/portal-gun.png", icon_size: (12.0, 12.0) },
    )?;

    let (f64_t, i32_t) = (code.ty_f64(), code.ty_i32());
    let obj_t = code.class("fish.system.ObjectClass")?;
    let g = |code: &mut Code, t| code.add_global(t);
    let st = State {
        x: [g(code, f64_t), g(code, f64_t)],
        y: [g(code, f64_t), g(code, f64_t)],
        sprite: [g(code, obj_t), g(code, obj_t)],
        placed: g(code, i32_t),
        next: g(code, i32_t),
        cool_uid: g(code, i32_t),
        cool_tick: g(code, i32_t),
    };
    let place = build_place(code, &st, &o)?;
    build_use(code, &st, place, &o)?;
    build_tick(code, &st, place, &o)
}

fn get_g(f: &mut FnBuilder, g: RefGlobal) -> Reg {
    let t = f.code().bc.globals[g.0];
    let r = f.reg(t);
    f.op(Opcode::GetGlobal { dst: r, global: g });
    r
}

fn set_g(f: &mut FnBuilder, g: RefGlobal, v: Reg) {
    f.op(Opcode::SetGlobal { global: g, src: v });
}

/// `place(layout, which, x, y)`: put portal `which` (0 blue, 1 orange) at x,y.
fn build_place(code: &mut Code, st: &State, o: &Opts) -> Result<openlina_sdk::hlbc::types::RefFun> {
    let layout_t = code.class("fish.system.Layout")?;
    let (f64_t, i32_t, bool_t, void) = (code.ty_f64(), code.ty_i32(), code.ty_bool(), code.ty_void());
    let create = code.method("fish.system.Layout", "createObject")?;
    let cb_t = code.func_type(create)?.args[7];
    let set_anim = code.method("fish.system.Sprite", "set_anim")?;

    let mut f = FnBuilder::new(code, "portal-gun/place", &[layout_t, i32_t, f64_t, f64_t], void);
    let (layout, which, x, y) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3));
    #[allow(clippy::needless_range_loop)] // k indexes several arrays and is a constant
    for k in 0..2 {
        let other = f.label();
        let kr = f.const_i32(k as i32);
        f.jne(which, kr, other);
        set_g(&mut f, st.x[k], x);
        set_g(&mut f, st.y[k], y);
        // placed |= 1 << k
        let p = get_g(&mut f, st.placed);
        let bit = f.const_i32(1 << k);
        f.op(Opcode::Or { dst: p, a: p, b: bit });
        set_g(&mut f, st.placed, p);
        // the sprite: create it the first time in this level, else move it
        let spr = get_g(&mut f, st.sprite[k]);
        let have = f.label();
        f.jnotnull(spr, have);
        let name = f.string_obj(SPRITE_TYPE)?;
        let layer = f.const_i32(0);
        let no = f.reg(bool_t);
        f.bool(no, false);
        let empty = f.string_obj("")?;
        let cb = f.reg(cb_t);
        f.op(Opcode::Null { dst: cb });
        let created = f.call_new(create, &[layout, name, layer, x, y, no, empty, cb])?;
        f.mov(spr, created);
        set_g(&mut f, st.sprite[k], spr);
        f.place(have);
        let sprite = f.get_new(spr, "sprite")?;
        let pos = f.get_new(sprite, "position")?;
        f.set(pos, "x", x)?;
        f.set(pos, "y", y)?;
        let skip_anim = f.label();
        let anim = anims::ensure(
            &mut f,
            &format!("fish.game.oclass.OClass_{SPRITE_TYPE}"),
            &format!("openlina_portal_{}", COLORS[k]),
            &format!("images/openlina/portal-{}.png", COLORS[k]),
            (16.0, 24.0),
            skip_anim,
        )?;
        f.call_new(set_anim, &[sprite, anim])?;
        let (w, h) = (f.const_f64(16.0), f.const_f64(24.0));
        f.set(sprite, "width", w)?;
        f.set(sprite, "height", h)?;
        f.place(skip_anim);
        if o.trace {
            f.print(&[Print::Str(&format!("[portal-gun] placed {} at (", COLORS[k])), Print::Val(x), Print::Str(", "), Print::Val(y), Print::Str(")")])?;
        }
        f.place(other);
    }
    f.ret_void();
    f.finish()
}

/// `item_use`: fire the portal gun.
fn build_use(code: &mut Code, st: &State, place: openlina_sdk::hlbc::types::RefFun, o: &Opts) -> Result<()> {
    let (bool_t, f64_t) = (code.ty_bool(), code.ty_f64());
    let first = code.method("fish.system.Picker", "first")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let sqrt = code.native("math_sqrt")?;
    let mut f = hooks::handler(code, "item_use", "portal-gun/use")?;
    let (slot, sheet, player_picker, cross) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3));
    let (not_mine, handled) = (f.label(), f.label());
    items::is_item(&mut f, slot, ITEM, not_mine)?;
    let (cx, cy) = items::crosshair_pos(&mut f, cross, handled)?;
    // The crosshair is the aiming reticle just in front of Lina: fire along Lina -> reticle,
    // `range` far, kept inside the play field.
    f.jnull(player_picker, handled);
    let p_dyn = f.call_new(first, &[player_picker])?;
    let player = f.cast(p_dyn, player_t);
    f.jnull(player, handled);
    let psprite = f.get_new(player, "sprite")?;
    let ppos = f.get_new(psprite, "position")?;
    let (px, py) = (f.get_new(ppos, "x")?, f.get_new(ppos, "y")?);
    let (dx, dy, d2, t, len) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
    f.sub(dx, cx, px);
    f.sub(dy, cy, py);
    f.mul(d2, dx, dx);
    f.mul(t, dy, dy);
    f.add(d2, d2, t);
    let eps = f.const_f64(0.01);
    f.jle(d2, eps, handled);
    f.call(len, sqrt, &[d2]);
    let range = f.const_f64(o.range);
    let (x, y) = (f.reg(f64_t), f.reg(f64_t));
    f.op(Opcode::SDiv { dst: x, a: dx, b: len });
    f.mul(x, x, range);
    f.add(x, x, px);
    f.op(Opcode::SDiv { dst: y, a: dy, b: len });
    f.mul(y, y, range);
    f.add(y, y, py);
    for (v, lo, hi) in [(x, 37.0, 563.0), (y, 41.0, 297.0)] {
        let (ok_lo, ok_hi) = (f.label(), f.label());
        let (l, h) = (f.const_f64(lo), f.const_f64(hi));
        f.jge(v, l, ok_lo);
        f.mov(v, l);
        f.place(ok_lo);
        f.jle(v, h, ok_hi);
        f.mov(v, h);
        f.place(ok_hi);
    }
    let layout = f.get_new(sheet, "layout")?;
    let which = get_g(&mut f, st.next);
    let void = f.code().ty_void();
    let r = f.reg(void);
    f.call(r, place, &[layout, which, x, y]);
    // next ^= 1
    let one = f.const_i32(1);
    f.op(Opcode::Xor { dst: which, a: which, b: one });
    set_g(&mut f, st.next, which);
    f.place(handled);
    let t = f.reg(bool_t);
    f.bool(t, true);
    f.ret(t);
    f.place(not_mine);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, "item_use", h)
}

/// `tick`: reset per level, apply the preset, teleport objects.
fn build_tick(code: &mut Code, st: &State, place: openlina_sdk::hlbc::types::RefFun, o: &Opts) -> Result<()> {
    // Element type of `physics_obj` (see solid-edges: the object whose `edgewith` the edge test reads).
    let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
    let fun = code.func(update)?.clone();
    let ew = openlina_sdk::edit::expect_one(openlina_sdk::edit::find_field_access(code, &fun, "edgewith", false), "`.edgewith` read")?;
    let Opcode::Field { obj: item, .. } = fun.ops[ew] else { bail!("`.edgewith` read is not a Field") };
    let obj_t: RefType = fun.regs[item.0 as usize];
    let get_vx = code.method("fish.system.beh.Physics", "getVelocityX")?;
    let get_vy = code.method("fish.system.beh.Physics", "getVelocityY")?;
    let sqrt = code.native("math_sqrt")?;
    let (f64_t, void) = (code.ty_f64(), code.ty_void());

    let mut f = hooks::handler(code, "tick", "portal-gun/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    let tick = f.get_new(layout, "currentTick")?;

    // New level: forget the portals (their sprites went with the old layout).
    let not_new = f.label();
    let one = f.const_i32(1);
    f.jne(tick, one, not_new);
    let zero = f.const_i32(0);
    set_g(&mut f, st.placed, zero);
    set_g(&mut f, st.next, zero);
    for k in 0..2 {
        let t = f.code().bc.globals[st.sprite[k].0];
        let n = f.reg(t);
        f.op(Opcode::Null { dst: n });
        set_g(&mut f, st.sprite[k], n);
    }
    f.jmp(end);
    f.place(not_new);

    if let Some([ax, ay, bx, by]) = o.preset {
        let not_yet = f.label();
        let two = f.const_i32(2);
        f.jne(tick, two, not_yet);
        for (k, (x, y)) in [(ax, ay), (bx, by)].into_iter().enumerate() {
            let (w, xr, yr) = (f.const_i32(k as i32), f.const_f64(x), f.const_f64(y));
            let r = f.reg(void);
            f.call(r, place, &[layout, w, xr, yr]);
        }
        f.place(not_yet);
    }

    let placed = get_g(&mut f, st.placed);
    let both = f.const_i32(3);
    f.jne(placed, both, end);
    let (px, py) = ([get_g(&mut f, st.x[0]), get_g(&mut f, st.x[1])], [get_g(&mut f, st.y[0]), get_g(&mut f, st.y[1])]);
    let r2 = f.const_f64(o.radius * o.radius);
    let exit_dist = f.const_f64(o.radius + 6.0);
    let picker = f.get_new(sheet, "physics_obj")?;
    let insts = f.get_new(picker, "insts")?;
    let n = f.array_len(insts)?;
    f.for_range(n, |f, i| {
        let next = f.label();
        let obj = f.array_get(insts, i, obj_t)?;
        f.jnull(obj, next);
        let sprite = f.get_new(obj, "sprite")?;
        f.jnull(sprite, next);
        let destroyed = f.get_new(sprite, "destroyed")?;
        f.jtrue(destroyed, next);
        let physics = f.get_new(obj, "physics")?;
        f.jnull(physics, next);
        let immovable = f.get_new(physics, "immovable")?;
        f.jtrue(immovable, next);
        // cooldown after a teleport
        let uid = f.get_new(obj, "uid")?;
        let cool = get_g(f, st.cool_uid);
        let not_cool = f.label();
        f.jne(uid, cool, not_cool);
        let since = f.reg(f.reg_type(tick));
        let ct = get_g(f, st.cool_tick);
        f.sub(since, tick, ct);
        let cd = f.const_i32(COOLDOWN);
        f.jlt(since, cd, next);
        f.place(not_cool);

        let pos = f.get_new(sprite, "position")?;
        let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
        for k in 0..2 {
            let far = f.label();
            // |p - portal k|^2 < r^2 ?
            let (dx, dy, d2, t) = (f.reg(f64_t), f.reg(f64_t), f.reg(f64_t), f.reg(f64_t));
            f.sub(dx, x, px[k]);
            f.sub(dy, y, py[k]);
            f.mul(d2, dx, dx);
            f.mul(t, dy, dy);
            f.add(d2, d2, t);
            f.jle(r2, d2, far);
            // exit at the other portal, pushed out along the direction of travel
            let vx = f.call_new(get_vx, &[physics])?;
            let vy = f.call_new(get_vy, &[physics])?;
            let (s2, s) = (f.reg(f64_t), f.reg(f64_t));
            f.mul(s2, vx, vx);
            f.mul(t, vy, vy);
            f.add(s2, s2, t);
            let (nx, ny) = (f.reg(f64_t), f.reg(f64_t));
            let slow = f.label();
            let moved = f.label();
            let eps = f.const_f64(1.0);
            f.jle(s2, eps, slow);
            f.call(s, sqrt, &[s2]);
            f.op(Opcode::SDiv { dst: nx, a: vx, b: s });
            f.op(Opcode::SDiv { dst: ny, a: vy, b: s });
            f.jmp(moved);
            f.place(slow); // not moving: drop out below
            f.float(nx, 0.0);
            f.float(ny, 1.0);
            f.place(moved);
            let other = 1 - k;
            let (ex, ey) = (f.reg(f64_t), f.reg(f64_t));
            f.mul(ex, nx, exit_dist);
            f.add(ex, ex, px[other]);
            f.mul(ey, ny, exit_dist);
            f.add(ey, ey, py[other]);
            f.set(pos, "x", ex)?;
            f.set(pos, "y", ey)?;
            set_g(f, st.cool_uid, uid);
            set_g(f, st.cool_tick, tick);
            if o.trace {
                let ty = f.get_new(obj, "type")?;
                f.print(&[
                    Print::Str("[portal-gun] tick "), Print::Val(tick), Print::Str(" "), Print::Val(ty),
                    Print::Str(&format!(" through {} -> {} at (", COLORS[k], COLORS[other])), Print::Val(ex),
                    Print::Str(", "), Print::Val(ey), Print::Str(")"),
                ])?;
            }
            f.jmp(next);
            f.place(far);
        }
        f.place(next);
        Ok(())
    })?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}
