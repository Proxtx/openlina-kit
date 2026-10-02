//! `gungun-mode`: you play as the Gun Gun. In every level Lina is gone; the game's own Gun Gun
//! sits where she would have spawned, every roll holds a Gun Gun and it never runs out of ammo.
//!
//! Vanilla Gun Gun (closure fn@8545, source L1632-1668): firing it with no gun of your own out
//! creates a `secondary` object (`OClass_secondary`, `playerId`, `itemInstId` of the slot) at the
//! gun's image point; every shot then kicks your secondary with `applyImpulseAtAngle(0.15, gun
//! angle)`, i.e. towards where Lina aims, and a contact listener damps it when it hits something.
//! Lina picks it back up by touching it (`update` L18683-18708: `isOverlapping`, `ammo++`,
//! destroy). Fruits are collected by Lina's physics contacts only (`update` L10010-10015:
//! `getPhysContactWith`, `tryTouchCoin`); once all are collected the portal opens and Lina wins by
//! overlapping it (`exitViaPortal`, L10000).
//!
//! What the mod does, from the core `tick` hook, in levels:
//!
//! - **Start of a level** (the first tick with Lina and the HUD slots, once per layout run): if no
//!   slot holds a Gun Gun, the slot `nr == 1` gets one (a new `{locked, type, wins}` record, so the
//!   run's tool list is untouched; ammo = its `baseAmmo`). Lina selects it and the mod calls the
//!   game's `shoot(playerId)` (a free shot: the slot keeps its ammo, at least 1): the vanilla code
//!   creates the gun, which the mod puts at Lina's spot
//!   and stops. Then Lina becomes a ghost: her physics body is switched off (`Physics.setEnabled
//!   0`: no collisions, no contacts, `syncPosWithSprite` leaves her alone) and her visible parts
//!   (`player`, `legs2`, `legs3`, `legs_bot`, `player_backflip`, the item icon on her head) are
//!   hidden every tick (`visible` false and opacity 0: the game shows some again later in the
//!   tick). Her held gun `img_gun` stays: vanilla draws it on the flying gun (`update` L10572).
//! - **Every tick after**: the ghost follows her gun (Lina's position = the gun's), so aiming, the
//!   reticle, other items and the portal work from the gun. Her switched-off body is moved along
//!   (`body_set_transform`, as `syncPosWithSprite` would): the portal test (`isOverlapping`) uses
//!   the body's shapes, so the level is won when the gun flies into the open portal. Fruits are
//!   collected as in vanilla when they leave the screen; with `fruits` the gun also collects the
//!   ones it touches (`tryTouchCoin`). When the gun is gone (out of the screen, destroyed) Lina is moved
//!   below the screen and the game's edge test ends the level as for any fall.
//! - The core `player_edge` hook keeps the ghost alive outside the screen while her gun exists (a
//!   gun that screen-wrap carries across the edge).
//! - Picking the gun back up (`update` L18686) is switched off for a ghost: the `isOverlapping`
//!   result becomes false.
//! - Ammo (`infinite_ammo`): the shot's `slot.ammo - 1` (the one `ammo - 1` site, as found by the
//!   infinite-ammo mod, which may have guarded it already: this mod runs after it) is skipped for
//!   Gun Gun slots.
//!
//! Left alone: the hub, title screens and tool selection; other items (they work from the gun);
//! how the gun flies.

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::asm::{FnBuilder, Label, Print};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefFun, Reg};
use openlina_sdk::{edit, hooks, world, Code, ModConfig};

const GUNGUN: &str = "gungun";
/// Lina's visible parts: `player`, legs and backflip sprite. Not her held gun (`img_gun`): while
/// the Gun Gun is out, the game draws it on the flying gun, turned to the aim (`update` L10572).
const PARTS: &[&str] = &["player", "legs2", "legs3", "legs_bot", "player_backflip"];
/// Where a ghost without a gun is sent so that the edge test ends the level.
const GONE_Y: f64 = 2000.0;

fn main() {
    openlina_sdk::run_mod(apply)
}

struct Opts {
    fruits: bool,
    trace: bool,
}

/// Functions the tick handler calls.
struct Fns {
    gun_of: RefFun,
    ghost: RefFun,
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let o = Opts { fruits: cfg.bool("fruits", false)?, trace: cfg.bool("trace", false)? };
    let infinite = cfg.bool("infinite_ammo", true)?;

    let fns = Fns { gun_of: build_gun_of(code)?, ghost: build_ghost(code)? };
    let init = build_init(code, &o, &fns)?;
    let follow = build_follow(code, &o, &fns)?;
    build_tick(code, init, follow)?;
    keep_ghost_alive(code, &fns)?;
    no_pickup(code, &fns)?;
    if infinite {
        keep_ammo(code, o.trace)?;
    }
    Ok(())
}

/// `gun_of(sheet, player) -> OClass_secondary`: the player's Gun Gun (not destroyed), or null.
fn build_gun_of(code: &mut Code) -> Result<RefFun> {
    let sheet_t = code.class("fish.game.evsheet.EvSheet_gameplay")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let sec_t = code.class("fish.game.oclass.OClass_secondary")?;
    let mut f = FnBuilder::new(code, "gungun-mode/gun_of", &[sheet_t, player_t], sec_t);
    let (sheet, player) = (f.arg(0), f.arg(1));
    let found = f.reg(sec_t);
    f.op(Opcode::Null { dst: found });
    let done = f.label();
    f.jnull(player, done);
    let id = f.get_new(player, "playerId")?;
    let picker = f.get_new(sheet, "secondary")?;
    f.jnull(picker, done);
    let insts = f.get_new(picker, "insts")?;
    f.jnull(insts, done);
    let n = f.array_len(insts)?;
    f.for_range(n, |f, k| {
        let next = f.label();
        let s = f.array_get(insts, k, sec_t)?;
        f.jnull(s, next);
        let sid = f.get_new(s, "playerId")?;
        f.jne(sid, id, next);
        let sprite = f.get_new(s, "sprite")?;
        f.jnull(sprite, next);
        let destroyed = f.get_new(sprite, "destroyed")?;
        f.jtrue(destroyed, next);
        f.mov(found, s);
        f.jmp(done);
        f.place(next);
        Ok(())
    })?;
    f.place(done);
    f.ret(found);
    f.finish()
}

/// `ghost(player) -> Bool`: is Lina a ghost (her physics body switched off by the mod)?
fn build_ghost(code: &mut Code) -> Result<RefFun> {
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let is_active = code.native("body_is_active")?;
    let bool_t = code.ty_bool();
    let mut f = FnBuilder::new(code, "gungun-mode/ghost", &[player_t], bool_t);
    let player = f.arg(0);
    let r = f.reg_bool();
    f.bool(r, false);
    let done = f.label();
    f.jnull(player, done);
    let phys = f.get_new(player, "physics")?;
    f.jnull(phys, done);
    let body = f.get_new(phys, "body")?;
    f.jnull(body, done);
    let active = f.call_new(is_active, &[body])?;
    f.jtrue(active, done);
    f.bool(r, true);
    f.place(done);
    f.ret(r);
    f.finish()
}

/// Jump to `not` unless `slot` holds a Gun Gun.
fn jump_unless_gungun(f: &mut FnBuilder, slot: Reg, not: Label) -> Result<()> {
    openlina_sdk::items::is_item(f, slot, GUNGUN, not)
}

/// `init(sheet) -> Bool`: the start of a level (see the module docs). False: not ready yet (no
/// Lina or no HUD slots), try again next tick.
fn build_init(code: &mut Code, o: &Opts, fns: &Fns) -> Result<RefFun> {
    let sheet_t = code.class("fish.game.evsheet.EvSheet_gameplay")?;
    let evsheet_t = code.class("fish.game.evsheet.EvSheet")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let slot_t = code.class("fish.game.oclass.OClass_b_item")?;
    let get_im = code.method("fish.game.evsheet.EvSheet", "get_itemManager")?;
    let find_item = code.method("fish.system.ItemManager", "findItem")?;
    let shoot = code.method("fish.game.evsheet.EvSheet_gameplay", "shoot")?;
    let set_v = code.method("fish.system.beh.Physics", "setVelocity")?;
    let set_enabled = code.method("fish.system.beh.Physics", "setEnabled")?;
    let item_rec_t = code.field_type(slot_t, code.field(slot_t, "item")?)?;
    let bool_t = code.ty_bool();

    let mut f = FnBuilder::new(code, "gungun-mode/init", &[sheet_t], bool_t);
    let sheet = f.arg(0);
    let (no, yes) = (f.label(), f.label());

    let pp = f.get_new(sheet, "player")?;
    f.jnull(pp, no);
    let players = f.get_new(pp, "insts")?;
    f.jnull(players, no);
    let np = f.array_len(players)?;
    let zero = f.const_i32(0);
    f.jle(np, zero, no);
    let sp = f.get_new(sheet, "b_item")?;
    f.jnull(sp, no);
    let slots = f.get_new(sp, "insts")?;
    f.jnull(slots, no);
    let ns = f.array_len(slots)?;
    f.jle(ns, zero, no);

    // A slot with a Gun Gun, else slot 1 gets one.
    let gslot = f.reg(slot_t);
    f.op(Opcode::Null { dst: gslot });
    f.for_range(ns, |f, k| {
        let next = f.label();
        let s = f.array_get(slots, k, slot_t)?;
        jump_unless_gungun(f, s, next)?;
        f.mov(gslot, s);
        f.place(next);
        Ok(())
    })?;
    let have = f.label();
    f.jnotnull(gslot, have);
    {
        let first = f.array_get(slots, zero, slot_t)?;
        f.mov(gslot, first);
        let one = f.const_f64(1.0);
        f.for_range(ns, |f, k| {
            let next = f.label();
            let s = f.array_get(slots, k, slot_t)?;
            f.jnull(s, next);
            let nr = f.get_new(s, "nr")?;
            f.jne(nr, one, next);
            f.mov(gslot, s);
            f.place(next);
            Ok(())
        })?;
        f.jnull(gslot, yes);
        let es = f.cast(sheet, evsheet_t);
        let im = f.call_new(get_im, &[es])?;
        f.jnull(im, yes);
        let name = f.string_obj(GUNGUN)?;
        let ty = f.call_new(find_item, &[im, name])?;
        let missing = f.label();
        f.jnull(ty, missing);
        let tn = f.get_new(ty, "name")?;
        let found = f.label();
        f.jstr_ne(tn, GUNGUN, missing)?;
        f.jmp(found);
        f.place(missing);
        if o.trace {
            f.print(&[Print::Str("[gungun-mode] the game has no Gun Gun in its item pool")])?;
        }
        f.jmp(yes);
        f.place(found);
        if o.trace {
            let old = f.get_new(gslot, "item")?;
            let old_ty = f.get_new(old, "type")?;
            let old_name = f.get_new(old_ty, "name")?;
            f.print(&[Print::Str("[gungun-mode] slot 1: "), Print::Val(old_name), Print::Str(" becomes the Gun Gun")])?;
        }
        let rec = f.reg(item_rec_t);
        f.op(Opcode::New { dst: rec });
        let (fl, wins) = (f.reg_bool(), f.const_i32(0));
        f.bool(fl, false);
        f.set(rec, "locked", fl)?;
        let type_t = {
            let c = f.code();
            c.field_type(item_rec_t, c.field(item_rec_t, "type")?)?
        };
        let ty = f.cast(ty, type_t);
        f.set(rec, "type", ty)?;
        f.set(rec, "wins", wins)?;
        f.set(gslot, "item", rec)?;
        let ammo = f.get_new(ty, "baseAmmo")?;
        f.set(gslot, "ammo", ammo)?;
    }
    f.place(have);

    // Each player: select it, fire it (the game creates the gun), put the gun at her spot, ghost.
    let nr = f.get_new(gslot, "nr")?;
    f.for_range(np, |f, k| {
        let next = f.label();
        let p = f.array_get(players, k, player_t)?;
        f.jnull(p, next);
        let g = f.call_new(fns.ghost, &[p])?;
        f.jtrue(g, next);
        let sprite = f.get_new(p, "sprite")?;
        let pos = f.get_new(sprite, "position")?;
        let (x, y) = (f.get_new(pos, "x")?, f.get_new(pos, "y")?);
        f.set(p, "item_selected", nr)?;
        let id = f.get_new(p, "playerId")?;
        // the start shot is free and always fires (a slot carried over from the previous layout
        // may be empty when `infinite_ammo` is off): at least 1 ammo, then the shot given back
        let ammo = f.get_new(gslot, "ammo")?;
        let one = f.const_i32(1);
        let enough = f.label();
        f.jge(ammo, one, enough);
        f.set(gslot, "ammo", one)?;
        f.place(enough);
        let before = f.get_new(gslot, "ammo")?;
        let _ = f.call_new(shoot, &[sheet, id])?;
        f.set(gslot, "ammo", before)?;
        let gun = f.call_new(fns.gun_of, &[sheet, p])?;
        let made = f.label();
        f.jnotnull(gun, made);
        if o.trace {
            f.print(&[Print::Str("[gungun-mode] firing made no Gun Gun, Lina stays")])?;
        }
        f.jmp(next);
        f.place(made);
        let gs = f.get_new(gun, "sprite")?;
        let gp = f.get_new(gs, "position")?;
        f.set(gp, "x", x)?;
        f.set(gp, "y", y)?;
        let gphys = f.get_new(gun, "physics")?;
        let still = f.label();
        f.jnull(gphys, still);
        let z = f.const_f64(0.0);
        let void = f.code().ty_void();
        let out = f.reg(void);
        f.call(out, set_v, &[gphys, z, z]);
        f.place(still);
        let phys = f.get_new(p, "physics")?;
        let off = f.const_i32(0);
        f.call(out, set_enabled, &[phys, off]);
        let fl = f.reg_bool();
        f.bool(fl, false);
        f.set(sprite, "visible", fl)?;
        if o.trace {
            f.print(&[
                Print::Str("[gungun-mode] Lina is gone, the Gun Gun is at ("),
                Print::Val(x),
                Print::Str(", "),
                Print::Val(y),
                Print::Str(")"),
            ])?;
        }
        f.place(next);
        Ok(())
    })?;
    f.jmp(yes);

    f.place(no);
    let r = f.reg_bool();
    f.bool(r, false);
    f.ret(r);
    f.place(yes);
    let r = f.reg_bool();
    f.bool(r, true);
    f.ret(r);
    f.finish()
}

/// `follow(sheet)`: every tick after the start (see the module docs).
fn build_follow(code: &mut Code, o: &Opts, fns: &Fns) -> Result<RefFun> {
    let sheet_t = code.class("fish.game.evsheet.EvSheet_gameplay")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let coin_t = code.class("fish.game.oclass.OClass_coin")?;
    let obj_t = code.class("fish.system.ObjectClass")?;
    let contact = code.method("fish.system.Sprite", "getPhysContactWith")?;
    let touch = code.method("fish.game.evsheet.EvSheet_gameplay", "tryTouchCoin")?;
    let contact_arg_t = code.func_type(contact)?.args[1];
    let set_transform = code.native("body_set_transform")?;
    let set_opacity = code.method("fish.system.Sprite", "setOpacity")?;
    let display_t = code.class("fish.game.oclass.OClass_icon_display")?;
    let map_t = code.class("haxe.ds.StringMap")?;
    let map_get = code.method("haxe.ds.StringMap", "get")?;
    let void = code.ty_void();

    let mut f = FnBuilder::new(code, "gungun-mode/follow", &[sheet_t], void);
    let sheet = f.arg(0);
    let end = f.label();
    let pp = f.get_new(sheet, "player")?;
    f.jnull(pp, end);
    let players = f.get_new(pp, "insts")?;
    f.jnull(players, end);
    let np = f.array_len(players)?;
    let any_ghost = f.reg_bool();
    f.bool(any_ghost, false);
    f.for_range(np, |f, k| {
        let next = f.label();
        let p = f.array_get(players, k, player_t)?;
        f.jnull(p, next);
        let g = f.call_new(fns.ghost, &[p])?;
        f.jfalse(g, next);
        f.bool(any_ghost, true);
        let pos = f.get_new(p, "sprite")?;
        let pos = f.get_new(pos, "position")?;
        let gun = f.call_new(fns.gun_of, &[sheet, p])?;
        let has = f.label();
        f.jnotnull(gun, has);
        // no gun any more: below the screen, the edge test ends the level
        let gone = f.const_f64(GONE_Y);
        f.set(pos, "y", gone)?;
        f.jmp(next);
        f.place(has);
        let gs = f.get_new(gun, "sprite")?;
        let gp = f.get_new(gs, "position")?;
        let (x, y) = (f.get_new(gp, "x")?, f.get_new(gp, "y")?);
        f.set(pos, "x", x)?;
        f.set(pos, "y", y)?;
        // her switched-off body too: `isOverlapping` (the open portal) tests the body's shapes,
        // and `syncPosWithSprite` skips inactive bodies (do what it does: layout units * worldScale)
        let phys = f.get_new(p, "physics")?;
        let body = f.get_new(phys, "body")?;
        let moved = f.label();
        f.jnull(body, moved);
        let lay = f.get_new(p, "layout")?;
        let ws = f.get_new(lay, "worldScale")?;
        let col_pos = f.get_new(phys, "colPos")?;
        f.set(col_pos, "x", x)?;
        f.set(col_pos, "y", y)?;
        let cache = f.get_new(phys, "colPosCache")?;
        let (bx, by) = (f.reg_f64(), f.reg_f64());
        f.mul(bx, x, ws);
        f.mul(by, y, ws);
        f.set(cache, "x", bx)?;
        f.set(cache, "y", by)?;
        let angle = f.get_new(phys, "colAngleCache")?;
        let out = f.reg(void);
        f.call(out, set_transform, &[body, cache, angle]);
        f.place(moved);
        if o.fruits {
            let cp = f.get_new(sheet, "coin")?;
            f.jnull(cp, next);
            let coins = f.get_new(cp, "insts")?;
            f.jnull(coins, next);
            let nc = f.array_len(coins)?;
            let zero = f.const_f64(0.0);
            f.for_range(nc, |f, j| {
                let skip = f.label();
                let c = f.array_get(coins, j, coin_t)?;
                f.jnull(c, skip);
                let cd = f.get_new(c, "cd")?;
                f.jne(cd, zero, skip);
                let state = f.get_new(c, "state")?;
                f.jne(state, zero, skip);
                let co = f.cast(c, obj_t);
                let cv = f.cast(co, contact_arg_t);
                let hit = f.call_new(contact, &[gs, cv])?;
                f.jnull(hit, skip);
                let out = f.reg(void);
                f.call(out, touch, &[sheet, c, p]);
                if o.trace {
                    f.print(&[Print::Str("[gungun-mode] the Gun Gun touched a fruit")])?;
                }
                f.place(skip);
                Ok(())
            })?;
        }
        f.place(next);
        Ok(())
    })?;
    // hide Lina's parts (the game shows some again later in the tick; it never sets opacity)
    f.jfalse(any_ghost, end);
    let hide = |f: &mut FnBuilder, o: Reg| -> Result<()> {
        let s = f.get_new(o, "sprite")?;
        let fl = f.reg_bool();
        f.bool(fl, false);
        f.set(s, "visible", fl)?;
        let zero = f.const_f64(0.0);
        let out = f.reg(void);
        f.call(out, set_opacity, &[s, zero]);
        Ok(())
    };
    for part in PARTS {
        let skip = f.label();
        let picker = f.get_new(sheet, part)?;
        f.jnull(picker, skip);
        let insts = f.get_new(picker, "insts")?;
        f.jnull(insts, skip);
        let n = f.array_len(insts)?;
        f.for_range(n, |f, k| {
            let next = f.label();
            let o = f.array_get(insts, k, obj_t)?;
            f.jnull(o, next);
            hide(f, o)?;
            f.place(next);
            Ok(())
        })?;
        f.place(skip);
    }
    // the item icon on her head (`icon_display` "head" and its `item_icon`, placed at her legs)
    let skip = f.label();
    let picker = f.get_new(sheet, "icon_display")?;
    f.jnull(picker, skip);
    let insts = f.get_new(picker, "insts")?;
    f.jnull(insts, skip);
    let n = f.array_len(insts)?;
    f.for_range(n, |f, k| {
        let next = f.label();
        let d = f.array_get(insts, k, display_t)?;
        f.jnull(d, next);
        let kind = f.get_new(d, "display_type")?;
        f.jnull(kind, next);
        f.jstr_ne(kind, "head", next)?;
        hide(f, d)?;
        let cont = f.get_new(d, "container")?;
        f.jnull(cont, next);
        let map = f.get_new(cont, "insts")?;
        let map = f.cast(map, map_t);
        f.jnull(map, next);
        let key = f.string_obj("item_icon")?;
        let icon = f.call_new(map_get, &[map, key])?;
        let icon = f.cast(icon, obj_t);
        f.jnull(icon, next);
        hide(f, icon)?;
        f.place(next);
        Ok(())
    })?;
    f.place(skip);
    f.place(end);
    f.ret_void();
    f.finish()
}

/// The `tick` handler: `init` once per layout run (until it succeeds), `follow` after.
fn build_tick(code: &mut Code, init: RefFun, follow: RefFun) -> Result<()> {
    let layout_t = code.class("fish.system.Layout")?;
    let i32_t = code.ty_i32();
    let (g_layout, g_tick, g_done) = (code.add_global(layout_t), code.add_global(i32_t), code.add_global(i32_t));

    let mut f = hooks::handler(code, "tick", "gungun-mode/tick")?;
    let (sheet, layout) = (f.arg(0), f.arg(1));
    let end = f.label();
    world::jump_unless_level(&mut f, layout, end)?;
    // a new layout run: another layout, or the tick went back (restart)
    let tick = f.get_new(layout, "currentTick")?;
    let last_layout = f.get_global(g_layout);
    let last_tick = f.get_global(g_tick);
    let (fresh, same) = (f.label(), f.label());
    f.jne(last_layout, layout, fresh);
    f.jlt(tick, last_tick, fresh);
    f.jmp(same);
    f.place(fresh);
    let zero = f.const_i32(0);
    f.set_global(g_done, zero);
    f.place(same);
    f.set_global(g_layout, layout);
    f.set_global(g_tick, tick);

    let done = f.get_global(g_done);
    let started = f.label();
    f.jne(done, zero, started);
    let ok = f.call_new(init, &[sheet])?;
    f.jfalse(ok, end);
    let one = f.const_i32(1);
    f.set_global(g_done, one);
    f.place(started);
    let void = f.code().ty_void();
    let out = f.reg(void);
    f.call(out, follow, &[sheet]);
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "tick", h)
}

/// `player_edge`: a ghost outside the screen stays alive while her gun exists.
fn keep_ghost_alive(code: &mut Code, fns: &Fns) -> Result<()> {
    let mut f = hooks::handler(code, "player_edge", "gungun-mode/player_edge")?;
    let (sheet, player) = (f.arg(2), f.arg(3));
    let r = f.reg_bool();
    f.bool(r, false);
    let done = f.label();
    let g = f.call_new(fns.ghost, &[player])?;
    f.jfalse(g, done);
    let gun = f.call_new(fns.gun_of, &[sheet, player])?;
    f.jnull(gun, done);
    f.bool(r, true);
    f.place(done);
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, "player_edge", h)
}

/// Lina picking her gun back up (`update` L18686: `isOverlapping(player.sprite, secondaries)`,
/// right before the slot loop that compares `"gungun"`): false for a ghost.
fn no_pickup(code: &mut Code, fns: &Fns) -> Result<()> {
    let update = code.method("fish.game.evsheet.EvSheet_gameplay", "update")?;
    let overlap = code.method("fish.system.Sprite", "isOverlapping")?;
    let player_t = code.class("fish.game.oclass.OClass_player")?;
    let bool_t = code.ty_bool();

    // may_pick(overlapping, player) -> Bool
    let mut f = FnBuilder::new(code, "gungun-mode/may_pick", &[bool_t, player_t], bool_t);
    let (over, player) = (f.arg(0), f.arg(1));
    let r = f.reg_bool();
    f.bool(r, false);
    let done = f.label();
    f.jfalse(over, done);
    let g = f.call_new(fns.ghost, &[player])?;
    f.jtrue(g, done);
    f.bool(r, true);
    f.place(done);
    f.ret(r);
    let may_pick = f.finish()?;

    let fun = code.func(update)?;
    let gungun_reads = edit::find(
        fun,
        |_, op| matches!(op, Opcode::GetGlobal { global, .. } if code.global_string(*global) == Some(GUNGUN)),
    );
    let first = *gungun_reads.first().context("no \"gungun\" in EvSheet_gameplay.update (game update?)")?;
    let call = edit::prev_match(fun, first, |op| matches!(edit::call_target(op), Some((t, _)) if t == overlap))
        .context("no isOverlapping before the Gun Gun pickup")?;
    ensure!(
        first - call < 40,
        "the Gun Gun pickup's isOverlapping is {} ops before \"gungun\" (game update?)",
        first - call
    );
    let (_, args) = edit::call_target(&fun.ops[call]).unwrap();
    let dst = match fun.ops[call] {
        Opcode::Call3 { dst, .. } => dst,
        _ => bail!("the Gun Gun pickup's isOverlapping is not a Call3"),
    };
    // player: `Field sprite = player.sprite` right before
    let field = edit::prev_match(fun, call, |op| matches!(op, Opcode::Field { dst, .. } if *dst == args[0]))
        .context("no player.sprite read before the pickup test")?;
    let Opcode::Field { obj: player, .. } = fun.ops[field] else { unreachable!() };
    ensure!(fun.regs[player.0 as usize] == player_t, "the pickup test isn't on a player (game update?)");
    let fun = code.func_mut(update)?;
    edit::insert_ops(fun, call + 1, vec![edit::call(dst, may_pick, &[dst, player])], edit::Incoming::ToOriginal);
    Ok(())
}

/// `infinite_ammo`: skip the shot's `slot.ammo = slot.ammo - 1` for Gun Gun slots.
fn keep_ammo(code: &mut Code, trace: bool) -> Result<()> {
    let slot_t = code.class("fish.game.oclass.OClass_b_item")?;
    let mut sites = Vec::new();
    for fun in &code.bc.functions {
        if code.func_location(fun).is_some_and(|l| l.starts_with("openlina/")) {
            continue;
        }
        for at in edit::find_field_access(code, fun, "ammo", true) {
            let Opcode::SetField { obj, src, .. } = fun.ops[at] else { continue };
            if fun.regs[obj.0 as usize] != slot_t {
                continue;
            }
            // r = slot.ammo; c = 1; r = r - c; [infinite-ammo's guard;] slot.ammo = r
            let Some(sub) = (at.saturating_sub(5)..at)
                .rev()
                .find(|&k| matches!(fun.ops[k], Opcode::Sub { dst, a, .. } if dst == src && a == src))
            else {
                continue;
            };
            let Opcode::Sub { b: c, .. } = fun.ops[sub] else { continue };
            if sub < 2 {
                continue;
            }
            let read = &fun.ops[sub - 2];
            let Opcode::Field { dst: r0, obj: o0, .. } = *read else { continue };
            if o0 != obj || r0 != src || !edit::is_field(code, fun, read, "ammo") {
                continue;
            }
            let Opcode::Int { dst, ptr } = fun.ops[sub - 1] else { continue };
            if dst == c && code.bc.ints[ptr.0] == 1 {
                sites.push((fun.findex, at, obj));
            }
        }
    }
    ensure!(sites.len() == 1, "expected one `slot.ammo - 1` (a shot), found {} (game update?)", sites.len());
    let (findex, at, slot) = sites[0];

    // keep(slot) -> Bool: true for a Gun Gun (the write is skipped)
    let bool_t = code.ty_bool();
    let mut f = FnBuilder::new(code, "gungun-mode/keep_ammo", &[slot_t], bool_t);
    let s = f.arg(0);
    let r = f.reg_bool();
    f.bool(r, false);
    let done = f.label();
    jump_unless_gungun(&mut f, s, done)?;
    f.bool(r, true);
    if trace {
        let ammo = f.get_new(s, "ammo")?;
        f.print(&[Print::Str("[gungun-mode] Gun Gun ammo stays "), Print::Val(ammo)])?;
    }
    f.place(done);
    f.ret(r);
    let keep = f.finish()?;

    let fun = code.func_mut(findex)?;
    let ok = edit::add_reg(fun, bool_t);
    edit::guard_op(fun, at, ok, keep, &[slot]);
    Ok(())
}
