//! Levels: level packs shipped by mods.
//!
//! The game keeps its custom levels as level packs (`fish.system.Pack` holding an
//! `mld.LevelPack` of `mld.Level`s of `mld.Object`s), loaded by the `PackManager` from
//! `userdata/levelpacks` (editor) and `userdata/downloaded` (workshop). Each enabled pack
//! contributes level instances named `"<pack name> <n>"` to the level pool.
//!
//! [`register`] builds a pack in bytecode (no files, nothing written to userdata) and adds it at
//! the core `packs` hook, right after the game loaded its own packs. Objects are created with the
//! game's own `mld.Object.getFromType(type)` (the editor's defaults), then positioned; any value
//! is allowed, including positions and angles the editor's grid would not produce.
//!
//! Describe the pack in TOML (e.g. `levels/<name>.toml`, embedded with `include_str!`):
//!
//! ```toml
//! name = "Tumble"
//! creator = "openlina-kit"
//!
//! [[level]]
//! fruit = "blue"                # blue | green | red
//! bg = [105, 105, 105]          # background colour
//! objects = [
//!   { type = "portal", x = 300, y = 150 },                 # where the player enters
//!   { type = "coin", x = 300, y = 100 },                   # the fruit
//!   { type = "tile_long", x = 300, y = 200, angle = 15 },  # degrees
//!   { type = "z_nograv", x = 100, y = 100, w = 64, h = 32 }, # resizable zones only
//! ]
//! ```
//!
//! Object types are the editor's (`mld.Object.idToType`); `lina strings tile_` lists candidates.

use anyhow::{ensure, Context, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;
use serde::Deserialize;

use crate::asm::FnBuilder;
use crate::{hooks, Code};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelPack {
    /// Pack name; level instances are named `"<name> <n>"` (1-based), e.g. `"Tumble 1"`.
    pub name: String,
    #[serde(default)]
    pub creator: String,
    #[serde(rename = "level")]
    pub levels: Vec<Level>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    #[serde(default = "blue")]
    pub fruit: String,
    #[serde(default = "grey")]
    pub bg: [u8; 3],
    /// Horizontally scrolling long level.
    #[serde(default)]
    pub long: bool,
    #[serde(default)]
    pub disable_modifiers: bool,
    pub objects: Vec<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    #[serde(rename = "type")]
    pub kind: String,
    pub x: f64,
    pub y: f64,
    /// Degrees, clockwise. Omitted: the editor's default for the type.
    pub angle: Option<f64>,
    /// Size, for resizable objects (zones).
    pub w: Option<f64>,
    pub h: Option<f64>,
}

fn blue() -> String {
    "blue".into()
}
fn grey() -> [u8; 3] {
    [105, 105, 105]
}

impl LevelPack {
    pub fn from_toml(s: &str) -> Result<Self> {
        let p: Self = toml::from_str(s).context("parsing the level pack")?;
        ensure!(!p.name.is_empty(), "the level pack needs a name");
        ensure!(!p.levels.is_empty(), "the level pack has no [[level]]");
        for (i, l) in p.levels.iter().enumerate() {
            ensure!(["blue", "green", "red"].contains(&l.fruit.as_str()), "level {}: unknown fruit `{}`", i + 1, l.fruit);
            ensure!(l.objects.iter().any(|o| o.kind == "portal"), "level {}: no `portal` (the player's entry)", i + 1);
        }
        Ok(p)
    }
}

/// Add a level pack to the game. Requires the `core` mod.
pub fn register(code: &mut Code, pack: &LevelPack) -> Result<()> {
    let object_t = code.class("mld.Object")?;
    let level_t = code.class("mld.Level")?;
    let from_type = code.method("mld.Object", "getFromType")?;
    let level_ctor = code.method("mld.Level", "__constructor__")?;
    let lp_ctor = code.method("mld.LevelPack", "__constructor__")?;
    let pack_ctor = code.method("fish.system.DownloadedPack", "__constructor__")?;
    let calc_hash = code.method("fish.system.LocalPack", "calcHash")?;
    let init_instances = code.method("fish.system.Pack", "initInstances")?;
    let of_string = code.method("haxe.io.Bytes", "ofString")?;
    let push = code.method("hl.types.ArrayObj", "push")?;
    let bool_t = code.ty_bool();
    for o in pack.levels.iter().flat_map(|l| &l.objects) {
        ensure!(code.bc.strings.iter().any(|s| s.as_str() == o.kind), "unknown object type `{}`", o.kind);
    }

    let mut f = hooks::handler(code, "packs", &format!("levels/{}", pack.name))?;
    let pm = f.arg(0);

    let levels = f.new_array_obj(level_t, &[])?;
    for l in &pack.levels {
        let lv = f.new_obj("mld.Level")?;
        f.call_new(level_ctor, &[lv])?;
        let flag = f.reg(bool_t);
        f.bool(flag, false);
        f.set(lv, "disabled", flag)?;
        f.bool(flag, l.long);
        f.set(lv, "longMode", flag)?;
        f.bool(flag, l.disable_modifiers);
        f.set(lv, "disableModifiers", flag)?;
        let fruit = f.string_obj(&l.fruit)?;
        f.set(lv, "fruitType", fruit)?;
        // bgColor: an ArrayBytes_Int of 3 components, allocated by the constructor.
        let bg = f.get_new(lv, "bgColor")?;
        let bytes = f.get_new(bg, "bytes")?;
        for (i, c) in l.bg.iter().enumerate() {
            let (idx, v) = (f.const_i32(i as i32 * 4), f.const_i32(*c as i32));
            f.op(Opcode::SetMem { bytes, index: idx, src: v });
        }

        let objects = f.new_array_obj(object_t, &[])?;
        for o in &l.objects {
            let ty = f.string_obj(&o.kind)?;
            let obj = f.call_new(from_type, &[ty])?;
            set_f64(&mut f, obj, "x", o.x)?;
            set_f64(&mut f, obj, "y", o.y)?;
            if let Some(a) = o.angle {
                set_f64(&mut f, obj, "rotation", a.to_radians())?;
            }
            if let Some(w) = o.w {
                set_f64(&mut f, obj, "width", w)?;
            }
            if let Some(h) = o.h {
                set_f64(&mut f, obj, "height", h)?;
            }
            f.call_new(push, &[objects, obj])?;
        }
        f.set(lv, "objects", objects)?;
        f.call_new(push, &[levels, lv])?;
    }

    let lp = f.new_obj("mld.LevelPack")?;
    f.call_new(lp_ctor, &[lp])?;
    let name = f.string_obj(&pack.name)?;
    f.set(lp, "name", name)?;
    let creator = f.string_obj(&pack.creator)?;
    f.set(lp, "creator", creator)?;
    f.set(lp, "levels", levels)?;

    let p = f.new_obj("fish.system.DownloadedPack")?;
    f.call_new(pack_ctor, &[p])?;
    let path = f.string_obj(&format!("openlina/{}", pack.name))?;
    f.set(p, "path", path)?;
    f.set(p, "data", lp)?;
    // The hash identifies the pack (level instances point back to it, pack states are saved by
    // it); derive it from the name so it is stable across runs.
    let seed = f.string_obj(&format!("openlina level pack {}", pack.name))?;
    let encoding_t = f.code().func_type(of_string)?.args[1];
    let enc = f.reg(encoding_t);
    f.op(Opcode::Null { dst: enc });
    let seed_bytes = f.call_new(of_string, &[seed, enc])?;
    f.call_new(calc_hash, &[p, seed_bytes])?;
    f.call_new(init_instances, &[p])?;
    let packs = f.get_new(pm, "packs")?;
    f.call_new(push, &[packs, p])?;
    f.ret_void();
    let h = f.finish()?;
    hooks::subscribe(code, "packs", h)
}

fn set_f64(f: &mut FnBuilder, obj: Reg, field: &str, v: f64) -> Result<()> {
    let r = f.const_f64(v);
    f.set(obj, field, r)
}

/// The name of the level being played (`levelManager.currentLevel.type.name`, e.g. `"Tumble 1"`),
/// or null.
pub fn current_level_name(f: &mut FnBuilder) -> Result<Reg> {
    let st = f.static_obj("fish.system.Main")?;
    let main = f.get_new(st, "i")?;
    let game = f.get_new(main, "game")?;
    let lm = f.get_new(game, "levelManager")?;
    let cur = f.get_new(lm, "currentLevel")?;
    let ty = f.get_new(cur, "type")?;
    f.get_new(ty, "name")
}

/// Jump to `not_mine` unless the level being played is one of `pack`'s. Use it at the top of
/// `tick` handlers that give a level its own behavior.
pub fn jump_unless_in_pack(f: &mut FnBuilder, pack: &LevelPack, not_mine: crate::asm::Label) -> Result<()> {
    let name = current_level_name(f)?;
    let mine = f.label();
    for i in 1..=pack.levels.len() {
        let other = f.label();
        f.jstr_ne(name, &format!("{} {i}", pack.name), other)?;
        f.jmp(mine);
        f.place(other);
    }
    f.jmp(not_mine);
    f.place(mine);
    Ok(())
}
