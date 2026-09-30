//! Sprite animations from mod images.
//!
//! Each object class keeps its animations in a static `StringMap` (`pkg.$OClass_x._animData`,
//! name → `fish.system.Anim`), built when the first instance is constructed. [`ensure`] adds a
//! one-frame animation showing an image shipped in the mod's `assets/` (e.g.
//! `images/openlina/portal.png`), once the map exists; `sprite.set_anim(name)` then shows it.

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use crate::asm::{FnBuilder, Label};

/// Emit: if `class`'s animation map exists and has no `name` yet, add a one-frame animation of
/// `url` (`w`×`h` pixels at 0,0 of the image). Jumps to `missing` if the map doesn't exist yet
/// (no instance was created). Returns the register holding `name` (a `String`).
pub fn ensure(f: &mut FnBuilder, class: &str, name: &str, url: &str, size: (f64, f64), missing: Label) -> Result<Reg> {
    ensure_frames(f, class, name, &[(url, size)], missing)
}

/// Like [`ensure`], with one frame per image (`(url, (w, h))`, each the whole image from 0,0).
/// Some objects show a fixed frame: item icons show frame 0 in the HUD and frame 1 (large) in the
/// tool selection and the editor.
pub fn ensure_frames(
    f: &mut FnBuilder,
    class: &str,
    name: &str,
    images: &[(&str, (f64, f64))],
    missing: Label,
) -> Result<Reg> {
    let map_get = f.code().method("haxe.ds.StringMap", "get")?;
    let map_set = f.code().method("haxe.ds.StringMap", "set")?;
    let frame_ctor = f.code().method("fish.system.FrameData", "__constructor__")?;
    let anim_ctor = f.code().method("fish.system.Anim", "__constructor__")?;
    let frame_t = f.code().class("fish.system.FrameData")?;
    let multi_t = f.code().func_type(frame_ctor)?.args[12];
    let (bool_t, dyn_t) = (f.code().ty_bool(), f.code().ty_dyn());

    let statics = f.static_obj(class)?;
    let map = f.get_new(statics, "_animData")?;
    f.jnull(map, missing);
    let name_r = f.string_obj(name)?;
    let existing = f.call_new(map_get, &[map, name_r])?;
    let have = f.label();
    f.jnotnull(existing, have);
    let no = f.reg(bool_t);
    f.bool(no, false);
    let mut fds = Vec::new();
    for &(url, size) in images {
        let fd = f.reg(frame_t);
        f.op(Opcode::New { dst: fd });
        let url_r = f.string_obj(url)?;
        let (x, y, w, h) = (f.const_f64(0.0), f.const_f64(0.0), f.const_f64(size.0), f.const_f64(size.1));
        let (dur, ox, oy) = (f.const_f64(1.0), f.const_f64(0.5), f.const_f64(0.5));
        // imagePoints (empty; element type irrelevant), polygon points (empty), no multi-polygons
        let points = f.new_array_obj(dyn_t, &[])?;
        let poly = f.empty_f64_array()?;
        let multi = f.reg(multi_t);
        f.op(Opcode::Null { dst: multi });
        f.call_new(frame_ctor, &[fd, url_r, x, y, w, h, no, dur, ox, oy, points, poly, multi])?;
        fds.push(fd);
    }
    let frames = f.new_array_obj(frame_t, &fds)?;
    let anim = f.new_obj("fish.system.Anim")?;
    let (speed, zero) = (f.const_f64(0.0), f.const_i32(0));
    f.call_new(anim_ctor, &[anim, speed, no, zero, zero, no, frames])?;
    f.call_new(map_set, &[map, name_r, anim])?;
    f.place(have);
    Ok(name_r)
}
