//! Joined objects: step ladders, bamboo, unicycles, tentacles, chains… are several physics bodies
//! held together by Box2D joints. Moving one piece alone tears it from the others (the joint pulls
//! it back, or the pieces fly apart), so mods that move objects (wrap, swap, teleport) move the
//! whole group by the same offset.
//!
//! Box2D has no "move this body and everything joined to it", and the game's containers
//! (`ObjectClass.container`) only group objects created together (a cannon and its barrel, a fruit
//! and its `spr_coin`), not joint chains. [`collect`] walks the joints instead
//! (`body_get_joint_list`, `joint_get_body_a/b`) breadth-first from one piece and finds each body's
//! object among the `physics_obj` and `secondary_physics` pickers.
//!
//! A group held by something that can't move with it is not a group to move: a static or
//! kinematic body (a vine hanging from the ceiling, a bridge between tiles), Lina, a body whose
//! object isn't found, or more than [`MAX_GROUP`] pieces. [`collect`] jumps to `held` then.
//!
//! ```ignore
//! let held = f.label();
//! let g = joints::collect(&mut f, sheet, obj, held)?;   // obj alone: g.n == 1
//! let (cx, cy) = joints::centre(&mut f, &g)?;           // the mean of the pieces' positions
//! joints::shift(&mut f, &g, dx, dy)?;                    // every piece by (dx, dy)
//! ```
//!
//! Moving `sprite.position` is enough: `Physics.syncPosWithSprite` teleports each Box2D body and
//! keeps its velocity, so the joints stay as they were.

use anyhow::Result;
use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use crate::asm::{FnBuilder, Label};

/// The most pieces a group may have (bigger ones count as held).
pub const MAX_GROUP: i32 = 64;
/// `b2BodyType`: only dynamic bodies move.
const DYNAMIC_BODY: i32 = 2;

/// A joined group: `members[0..n]` (`ObjectClass`), `members[0]` the piece it was collected from.
pub struct Group {
    /// A native array of `fish.system.ObjectClass` (read with `GetArray`, see [`shift`]).
    pub members: Reg,
    /// I32: how many pieces (1: the object has no joints).
    pub n: Reg,
}

/// The group `obj` (an `ObjectClass`) belongs to: `obj` and every dynamic body joined to it,
/// directly or through others. An object without physics or joints is a group of one. Jumps to
/// `held` when the group is held by something that can't move with it (see the module docs).
/// `sheet` is the gameplay sheet (`EvSheet_gameplay`).
pub fn collect(f: &mut FnBuilder, sheet: Reg, obj: Reg, held: Label) -> Result<Group> {
    let code = f.code();
    let joint_list = code.native("body_get_joint_list")?;
    let body_a = code.native("joint_get_body_a")?;
    let body_b = code.native("joint_get_body_b")?;
    let body_type = code.native("body_get_type")?;
    let alloc_array = code.native("alloc_array")?;
    let obj_t = code.class("fish.system.ObjectClass")?;
    let joint_list_t = code.func_type(joint_list)?.ret;

    let obj = f.cast(obj, obj_t);
    let ty = f.type_value(obj_t);
    let cap = f.const_i32(MAX_GROUP);
    let members = f.call_new(alloc_array, &[ty, cap])?;
    let (n, i, zero) = (f.reg_i32(), f.reg_i32(), f.const_i32(0));
    f.op(Opcode::SetArray { array: members, index: zero, src: obj });
    f.int(n, 1);
    f.int(i, 0);

    // Breadth-first over the joints: members[i] for i < n, appending new pieces.
    let (outer, collected) = (f.label(), f.label());
    let m = f.reg(obj_t);
    let edge = f.reg(joint_list_t);
    f.place(outer);
    f.jge(i, n, collected);
    f.op(Opcode::GetArray { dst: m, array: members, index: i });
    f.op(Opcode::Incr { dst: i });
    let mphys = f.get_new(m, "physics")?;
    f.jnull(mphys, outer);
    let mbody = f.get_new(mphys, "body")?;
    f.jnull(mbody, outer);
    f.call(edge, joint_list, &[mbody]);
    let inner = f.label();
    f.place(inner);
    f.jnull(edge, outer);
    let joint = f.get_new(edge, "joint")?;
    let next = f.get_new(edge, "next")?;
    f.mov(edge, next);
    f.jnull(joint, inner);
    for end in [body_a, body_b] {
        let skip = f.label();
        let other = f.call_new(end, &[joint])?;
        f.jnull(other, skip);
        let t = f.call_new(body_type, &[other])?;
        let dynamic = f.const_i32(DYNAMIC_BODY);
        f.jne(t, dynamic, held);
        let o = owner_of(f, sheet, other, held)?;
        let otype = f.get_new(o, "type")?;
        let not_player = f.label();
        f.jstr_ne(otype, "player", not_player)?;
        f.jmp(held);
        f.place(not_player);
        // already a member?
        let (k, scan, add) = (f.reg_i32(), f.label(), f.label());
        let km = f.reg(obj_t);
        f.int(k, 0);
        f.place(scan);
        f.jge(k, n, add);
        f.op(Opcode::GetArray { dst: km, array: members, index: k });
        f.op(Opcode::Incr { dst: k });
        f.jeq(km, o, skip);
        f.jmp(scan);
        f.place(add);
        f.jge(n, cap, held);
        f.op(Opcode::SetArray { array: members, index: n, src: o });
        f.op(Opcode::Incr { dst: n });
        f.place(skip);
    }
    f.jmp(inner);
    f.place(collected);
    Ok(Group { members, n })
}

/// Move every piece of `g` by (`dx`, `dy`) (F64 registers).
pub fn shift(f: &mut FnBuilder, g: &Group, dx: Reg, dy: Reg) -> Result<()> {
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    f.for_range(g.n, |f, k| {
        let o = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: o, array: g.members, index: k });
        let p = f.get_new(o, "sprite")?;
        let p = f.get_new(p, "position")?;
        let (x, y) = (f.get_new(p, "x")?, f.get_new(p, "y")?);
        f.add(x, x, dx);
        f.add(y, y, dy);
        f.set(p, "x", x)?;
        f.set(p, "y", y)?;
        Ok(())
    })
}

/// The centre of `g`: the mean of its pieces' positions (new F64 registers).
pub fn centre(f: &mut FnBuilder, g: &Group) -> Result<(Reg, Reg)> {
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    let (cx, cy, nf) = (f.reg_f64(), f.reg_f64(), f.reg_f64());
    f.float(cx, 0.0);
    f.float(cy, 0.0);
    f.for_range(g.n, |f, k| {
        let o = f.reg(obj_t);
        f.op(Opcode::GetArray { dst: o, array: g.members, index: k });
        let p = f.get_new(o, "sprite")?;
        let p = f.get_new(p, "position")?;
        let (x, y) = (f.get_new(p, "x")?, f.get_new(p, "y")?);
        f.add(cx, cx, x);
        f.add(cy, cy, y);
        Ok(())
    })?;
    f.op(Opcode::ToSFloat { dst: nf, src: g.n });
    f.op(Opcode::SDiv { dst: cx, a: cx, b: nf });
    f.op(Opcode::SDiv { dst: cy, a: cy, b: nf });
    Ok((cx, cy))
}

/// Piece `k` (an I32 register) of `g`, as an `ObjectClass`.
pub fn member(f: &mut FnBuilder, g: &Group, k: Reg) -> Result<Reg> {
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    let o = f.reg(obj_t);
    f.op(Opcode::GetArray { dst: o, array: g.members, index: k });
    Ok(o)
}

/// The object whose physics body is `body`, among `physics_obj` and `secondary_physics`; jumps to
/// `none` when there is none.
pub fn owner_of(f: &mut FnBuilder, sheet: Reg, body: Reg, none: Label) -> Result<Reg> {
    let obj_t = f.code().class("fish.system.ObjectClass")?;
    let found = f.reg(obj_t);
    let done = f.label();
    for picker in ["physics_obj", "secondary_physics"] {
        let next_picker = f.label();
        let p = f.get_new(sheet, picker)?;
        f.jnull(p, next_picker);
        let insts = f.get_new(p, "insts")?;
        f.jnull(insts, next_picker);
        let len = f.array_len(insts)?;
        f.for_range(len, |f, k| {
            let miss = f.label();
            let o = f.array_get(insts, k, obj_t)?;
            f.jnull(o, miss);
            let ph = f.get_new(o, "physics")?;
            f.jnull(ph, miss);
            let b = f.get_new(ph, "body")?;
            f.jne(b, body, miss);
            f.mov(found, o);
            f.jmp(done);
            f.place(miss);
            Ok(())
        })?;
        f.place(next_picker);
    }
    f.jmp(none);
    f.place(done);
    Ok(found)
}
