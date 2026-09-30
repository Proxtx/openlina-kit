//! Physics helpers on top of the game's Box2D world (`layout.world`).
//!
//! The game works in layout units; Box2D in physics units (layout units × `layout.worldScale`).
//! Bodies belong to objects: a fixture's user data is its `ObjectClass`.
//!
//! [`RayCast`]: the nearest object on a line, with the native `world_ray_cast(world, callback,
//! from, to)` the game's line-of-sight behavior (`fish.system.beh.LOS`) uses. Install it once
//! (it adds a callback and three globals), then cast from any function you build:
//!
//! ```ignore
//! let ray = physics::RayCast::install(code, "swap/ray", true)?;
//! let mut f = hooks::handler(code, "item_use", "swap/use")?;
//! …
//! let hit = ray.cast(&mut f, layout, (px, py), (tx, ty), Some(player))?; // ObjectClass or null
//! ```

use anyhow::{ensure, Context, Result};
use hlbc::opcodes::Opcode;
use hlbc::types::{RefFun, RefGlobal, RefType, Reg};

use crate::asm::FnBuilder;
use crate::Code;

pub struct RayCast {
    callback: RefFun,
    callback_t: RefType,
    ignore: RefGlobal,
    best: RefGlobal,
    frac: RefGlobal,
}

impl RayCast {
    /// Add the ray-cast callback `name`. With `statics`, static bodies (tiles) are hits too, so
    /// walls stop the ray (check the hit's `physics.immovable`); without, the ray passes through
    /// them and only movable objects count.
    pub fn install(code: &mut Code, name: &str, statics: bool) -> Result<Self> {
        let cast = code.native("world_ray_cast")?;
        let callback_t = code.func_type(cast)?.args[1];
        let cb = callback_t.as_fun(&code.bc).cloned().context("world_ray_cast: argument 2 is not a function type")?;
        // (b2Fixture, Vector2Default point, Vector2Default normal, F64 fraction) -> F64
        ensure!(
            cb.args.len() == 4 && cb.ret == code.ty_f64(),
            "world_ray_cast callback: unexpected signature {}",
            code.type_name(callback_t)
        );
        let user_data = code.native("fixture_get_user_data")?;
        let obj_t = code.class("fish.system.ObjectClass")?;
        let f64_t = code.ty_f64();
        let (ignore, best, frac) = (code.add_global(obj_t), code.add_global(obj_t), code.add_global(f64_t));

        // Box2D: return -1 to skip a fixture, its fraction to clip the ray there. Fixtures come
        // in any order; clipping leaves the nearest one last.
        let mut f = FnBuilder::new(code, name, &cb.args, cb.ret);
        let (fixture, fraction) = (f.arg(0), f.arg(3));
        let skip = f.label();
        let ud = f.call_new(user_data, &[fixture])?;
        let obj = f.cast(ud, obj_t);
        f.jnull(obj, skip);
        let me = f.get_global(ignore);
        f.jeq(obj, me, skip);
        let sprite = f.get_new(obj, "sprite")?;
        f.jnull(sprite, skip);
        let destroyed = f.get_new(sprite, "destroyed")?;
        f.jtrue(destroyed, skip);
        if !statics {
            let physics = f.get_new(obj, "physics")?;
            f.jnull(physics, skip);
            let immovable = f.get_new(physics, "immovable")?;
            f.jtrue(immovable, skip);
        }
        f.set_global(best, obj);
        f.set_global(frac, fraction);
        f.ret(fraction);
        f.place(skip);
        let minus = f.const_f64(-1.0);
        f.ret(minus);
        let callback = f.finish()?;
        Ok(Self { callback, callback_t, ignore, best, frac })
    }

    /// Cast a ray in `layout`'s world from `from` to `to` (layout units). Returns a new register
    /// holding the nearest `ObjectClass` hit, or null (nothing hit, or no physics world).
    /// `ignore`: an object the ray passes through, e.g. the one casting it.
    pub fn cast(
        &self,
        f: &mut FnBuilder,
        layout: Reg,
        from: (Reg, Reg),
        to: (Reg, Reg),
        ignore: Option<Reg>,
    ) -> Result<Reg> {
        let cast = f.code().native("world_ray_cast")?;
        let v2_new = f.code().method("hxmath.math.Vector2Default", "__constructor__")?;
        let v2_t = f.code().func_type(v2_new)?.args[0];
        let f64_t = f.code().ty_f64();
        let done = f.label();
        f.clear_global(self.best);
        match ignore {
            Some(r) => f.set_global(self.ignore, r),
            None => f.clear_global(self.ignore),
        }
        let one = f.const_f64(1.0);
        f.set_global(self.frac, one);
        let world = f.get_new(layout, "world")?;
        f.jnull(world, done);
        let scale = f.get_new(layout, "worldScale")?;
        let vec = |f: &mut FnBuilder, (x, y): (Reg, Reg)| -> Result<Reg> {
            let v = f.reg(v2_t);
            f.op(Opcode::New { dst: v });
            let (sx, sy) = (f.reg(f64_t), f.reg(f64_t));
            f.mul(sx, x, scale);
            f.mul(sy, y, scale);
            f.call_new(v2_new, &[v, sx, sy])?;
            Ok(v)
        };
        let (a, b) = (vec(f, from)?, vec(f, to)?);
        let cb = f.static_closure(self.callback, Some(self.callback_t))?;
        f.call_new(cast, &[world, cb, a, b])?;
        f.place(done);
        f.clear_global(self.ignore);
        Ok(f.get_global(self.best))
    }
}
