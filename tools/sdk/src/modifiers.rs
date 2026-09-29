//! Modifiers: the per-level rule changes the game rolls (the icon in the top-left HUD).
//!
//! The game draws `currentLevel.modifier` from fixed pools in `LevelManager.rollRaw`/`reroll`
//! (`[1,2,3,4,5,6,9]`, or `[1,2,9]` in "dx" runs) and shows it as a frame of the `"mod"` animation
//! of `OClass_optionthingos`. [`register`] adds a new modifier through the core hooks
//! `modifier_pool` and `modifier_icon`, so independent modifier mods combine:
//!
//! ```ignore
//! let id = modifiers::register(code, &Modifier { key: "screen-wrap", icon: "images/openlina/screen-wrap.png", size: (16.0, 16.0), in_dx: true })?;
//! // in a hook handler: only act while this level rolled our modifier
//! let active = modifiers::is_active(&mut f, id)?;
//! ```

use anyhow::{bail, Result};
use hlbc::types::Reg;

use crate::asm::FnBuilder;
use crate::hooks;
use crate::Code;

/// Modifier ids the base game uses.
pub const VANILLA_IDS: &[i32] = &[1, 2, 3, 4, 5, 6, 9];

pub struct Modifier<'a> {
    /// Stable name, usually the mod id. Determines the modifier id (see [`id_of`]).
    pub key: &'a str,
    /// HUD icon: an image under `fish/game/res/` (ship it in the mod's `assets/`), e.g.
    /// `images/openlina/screen-wrap.png`. The vanilla icons are 16×16.
    pub icon: &'a str,
    pub size: (f64, f64),
    /// Also roll it in "dx" runs (the game's smaller pool).
    pub in_dx: bool,
}

/// The modifier id for a key: `100 + fnv1a(key) % 9000`. Stable across packs and builds, so
/// scenarios and saved runs can refer to it (the harness option `modifier_key` uses this).
pub fn id_of(key: &str) -> i32 {
    let mut h: u32 = 0x811c9dc5;
    for b in key.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    100 + (h % 9000) as i32
}

/// Register a modifier: it becomes possible in the game's rolls and shows its own HUD icon.
/// Returns its id. Requires the `core` mod.
pub fn register(code: &mut Code, m: &Modifier) -> Result<i32> {
    let id = id_of(m.key);
    let marker = format!("modifier/{id}");
    if code.bc.functions.iter().any(|f| code.func_location(f).is_some_and(|l| l.starts_with(&format!("openlina/{marker}:")))) {
        bail!("modifier id {id} (key `{}`) is already registered by another mod", m.key);
    }
    // A marker function records the registration in the bytecode.
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, &marker, &[], void);
    f.ret_void();
    f.finish()?;

    // modifier_pool(pool, dx): pool.push(id)
    let push = code.method("hl.types.ArrayBytes_Int", "push")?;
    let mut f = hooks::handler(code, "modifier_pool", &format!("{}/pool", m.key))?;
    let (pool, dx) = (f.arg(0), f.arg(1));
    let skip = f.label();
    if !m.in_dx {
        f.jtrue(dx, skip);
    }
    let v = f.const_i32(id);
    f.call_new(push, &[pool, v])?;
    f.place(skip);
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "modifier_pool", h)?;

    // modifier_icon(icon): if the level's modifier is ours, show our own animation.
    let set_anim = code.method("fish.system.Sprite", "set_anim")?;
    let bool_t = code.ty_bool();
    let mut f = hooks::handler(code, "modifier_icon", &format!("{}/icon", m.key))?;
    let icon = f.arg(0);
    let no = f.label();
    let cur_mod = current_modifier(&mut f)?;
    let want = f.const_i32(id);
    f.jne(cur_mod, want, no);
    let name = crate::anims::ensure(&mut f, "fish.game.oclass.OClass_optionthingos", &format!("openlina_mod_{id}"), m.icon, m.size, no)?;
    let sprite = f.get_new(icon, "sprite")?;
    f.call_new(set_anim, &[sprite, name])?;
    let zero = f.const_i32(0);
    f.set(sprite, "animFrame", zero)?;
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.ret(yes);
    f.place(no);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    let h = f.finish()?;
    hooks::subscribe(code, "modifier_icon", h)?;
    Ok(id)
}

/// `Main.i.game.levelManager.currentLevel.modifier` (0 if there is no current level).
pub fn current_modifier(f: &mut FnBuilder) -> Result<Reg> {
    let i32_t = f.code().ty_i32();
    let out = f.reg(i32_t);
    f.int(out, 0);
    let done = f.label();
    let st = f.static_obj("fish.system.Main")?;
    let main = f.get_new(st, "i")?;
    let game = f.get_new(main, "game")?;
    let lm = f.get_new(game, "levelManager")?;
    let cur = f.get_new(lm, "currentLevel")?;
    f.jnull(cur, done);
    f.get(out, cur, "modifier")?;
    f.place(done);
    Ok(out)
}

/// New Bool register: is modifier `id` active in the current level?
pub fn is_active(f: &mut FnBuilder, id: i32) -> Result<Reg> {
    let bool_t = f.code().ty_bool();
    let m = current_modifier(f)?;
    let want = f.const_i32(id);
    let r = f.reg(bool_t);
    let (yes, done) = (f.label(), f.label());
    f.jeq(m, want, yes);
    f.bool(r, false);
    f.jmp(done);
    f.place(yes);
    f.bool(r, true);
    f.place(done);
    Ok(r)
}
