//! `harness`: drive the game for automated tests and showcase recordings, without input.
//!
//! 1. **Skip the title screen** at title tick `start_tick`, like a key press does
//!    (`EvSheet_first_screen_ev.update`: `levelManager.refreshPool(Main.i.packManager);
//!    layout.goToLayout("main")`).
//! 2. **Load a chosen level** on gameplay tick 2 after the title (the run's first level), with the recipe the editor's
//!    Play button uses (`editor.buttons.Play.onClick`):
//!    `levelPool = [level]; rollRaw(new Rand(seed), …); [modifier]; rollItemsRaw(); [items];
//!    loadCurrentLevel(game)`. `level` is matched by name (and `level_n`) in the pool built by
//!    `refreshPool`; `list_levels` prints the pool.
//! 3. **Scripted inputs**: `PlayerInputs.updateSP` (the per-frame keyboard/gamepad read) is guarded:
//!    while a level runs and `inputs` is set, it calls `readBin(bits)` instead, like a replay.
//!    Bits: 0 up, 1 down, 2 left, 3 right, 4 jump, 5 shoot, 6 switch, 7 restart.
//! 4. **Frame capture**: renders the game to `Main.gifTarget` (600×338, as `Main.renderGifFrame`
//!    does), `capturePixels().toPNG()` → `<capture_dir>/frame_1NNNN.png`.
//! 5. **Exit** with code 0 at level tick `end_tick`.
//!
//! Everything it does is printed as `[harness] …` lines, which `lina test` parses.

use anyhow::{bail, ensure, Context, Result};
use openlina_sdk::asm::{FnBuilder, Print};
use openlina_sdk::edit::{add_reg, call, insert_ops, prepend_call, Incoming};
use openlina_sdk::hlbc::opcodes::Opcode;
use openlina_sdk::hlbc::types::{RefFun, RefGlobal, RefType, Reg};
use openlina_sdk::{hooks, Code, ModConfig};

fn main() {
    openlina_sdk::run_mod(apply)
}

struct Opts {
    start_tick: i32,
    level: String,
    level_n: i32,
    seed: i32,
    modifier: i32,
    items: Vec<String>,
    inputs: Vec<(i32, i32, i32)>,
    end_tick: i32,
    capture: Option<(i32, i32, i32)>,
    capture_dir: String,
    list_levels: bool,
}

fn list(cfg: &ModConfig, key: &str) -> Result<Vec<String>> {
    match cfg.table.get(key) {
        None => Ok(vec![]),
        Some(v) => v
            .as_array()
            .context("expected a list")?
            .iter()
            .map(|x| x.as_str().map(str::to_string).context("expected strings"))
            .collect(),
    }
}

/// `"60-90:right+jump"` (ticks 60..90) or `"60:shoot"` (tick 60 only) → (from, to, bits).
fn parse_input(s: &str) -> Result<(i32, i32, i32)> {
    let (range, actions) = s.split_once(':').with_context(|| format!("input `{s}`: expected `ticks:actions`"))?;
    let (from, to) = match range.split_once('-') {
        Some((a, b)) => (a.trim().parse()?, b.trim().parse()?),
        None => {
            let t: i32 = range.trim().parse()?;
            (t, t + 1)
        }
    };
    ensure!(from < to, "input `{s}`: empty tick range");
    let mut bits = 0;
    for a in actions.split('+').map(str::trim).filter(|a| !a.is_empty()) {
        bits |= 1 << match a {
            "up" => 0,
            "down" => 1,
            "left" => 2,
            "right" => 3,
            "jump" => 4,
            "shoot" => 5,
            "switch" => 6,
            "restart" => 7,
            other => bail!("input `{s}`: unknown action `{other}`"),
        };
    }
    Ok((from, to, bits))
}

/// `"60-300/2"`: ticks 60..=300, every 2nd tick.
fn parse_capture(s: &str) -> Result<Option<(i32, i32, i32)>> {
    if s.is_empty() {
        return Ok(None);
    }
    let (range, step) = s.split_once('/').unwrap_or((s, "1"));
    let (a, b) = range.split_once('-').with_context(|| format!("capture `{s}`: expected `from-to/step`"))?;
    Ok(Some((a.trim().parse()?, b.trim().parse()?, step.trim().parse()?)))
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let o = Opts {
        start_tick: cfg.i64("start_tick", 5)? as i32,
        level: cfg.str("level", "")?.to_string(),
        level_n: cfg.i64("level_n", -1)? as i32,
        seed: cfg.i64("seed", 1)? as i32,
        modifier: cfg.i64("modifier", -1)? as i32,
        items: list(cfg, "items")?,
        inputs: list(cfg, "inputs")?.iter().map(|s| parse_input(s)).collect::<Result<_>>()?,
        end_tick: cfg.i64("end_tick", 0)? as i32,
        capture: parse_capture(cfg.str("capture", "")?)?,
        capture_dir: cfg.str("capture_dir", "frames")?.to_string(),
        list_levels: cfg.bool("list_levels", false)?,
    };

    let i32_t = code.ty_i32();
    // 0 = title screen (which also runs gameplay ticks at startup: ignore those),
    // 1 = title skipped, 2 = the test level is running.
    let state = code.add_global(i32_t);

    skip_title(code, o.start_tick, state)?;
    let capture = if o.capture.is_some() { Some(build_capture(code, &o.capture_dir)?) } else { None };
    let tick = build_tick(code, &o, state, capture)?;
    hooks::subscribe(code, "tick", tick)?;
    if !o.inputs.is_empty() {
        script_inputs(code, &o, state)?;
    }
    Ok(())
}

/// At title tick `tick`: `levelManager.refreshPool(Main.i.packManager); layout.goToLayout("main")`.
fn skip_title(code: &mut Code, tick: i32, state: RefGlobal) -> Result<()> {
    let update = code.method("fish.game.evsheet.EvSheet_first_screen_ev", "update")?;
    let refresh = code.method("fish.system.LevelManager", "refreshPool")?;
    let goto = code.method("fish.system.Layout", "goToLayout")?;
    let layout_t = code.class("fish.system.Layout")?;
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, "harness/skip_title", &[layout_t], void);
    let layout = f.arg(0);
    let skip = f.label();
    let now = f.get_new(layout, "currentTick")?;
    let at = f.const_i32(tick);
    f.jne(now, at, skip);
    let main = main_instance(&mut f)?;
    let game = f.get_new(main, "game")?;
    let lm = f.get_new(game, "levelManager")?;
    let pm = f.get_new(main, "packManager")?;
    f.call_new(refresh, &[lm, pm])?;
    let name = f.string_obj("main")?;
    f.call_new(goto, &[layout, name])?;
    set_global(&mut f, state, 1);
    f.print(&[Print::Str("[harness] title skipped")])?;
    f.place(skip);
    f.ret_void();
    let start = f.finish()?;
    prepend_call(code, update, start, &[Reg(1)])
}

fn main_instance(f: &mut FnBuilder) -> Result<Reg> {
    let st = f.static_obj("fish.system.Main")?;
    f.get_new(st, "i")
}

fn get_global(f: &mut FnBuilder, g: RefGlobal) -> Reg {
    let t = f.code().bc.globals[g.0];
    let r = f.reg(t);
    f.op(Opcode::GetGlobal { dst: r, global: g });
    r
}

fn set_global(f: &mut FnBuilder, g: RefGlobal, v: i32) {
    let r = f.const_i32(v);
    f.op(Opcode::SetGlobal { global: g, src: r });
}

/// Type of `field` of the type of `field_of`'s field `outer` (e.g. the element type of a record).
fn nested_type(code: &Code, t: RefType, outer: &str, inner: &str) -> Result<RefType> {
    let ot = code.field_type(t, code.field(t, outer)?)?;
    code.field_type(ot, code.field(ot, inner)?)
}

fn build_tick(code: &mut Code, o: &Opts, state: RefGlobal, capture: Option<RefFun>) -> Result<RefFun> {
    let lm_t = code.class("fish.system.LevelManager")?;
    let level_t = nested_type(code, lm_t, "currentLevel", "type")?; // {bgColor, …, name, …}
    let b_item_t = code.class("fish.game.oclass.OClass_b_item")?;
    let pool_item_t = nested_type(code, b_item_t, "item", "type")?; // {aimType, baseAmmo, name, …}
    let slot_t = code.field_type(b_item_t, code.field(b_item_t, "item")?)?; // {locked, type, wins}
    let rand_init = code.method("hxd.Rand", "init")?;
    let roll = code.method("fish.system.LevelManager", "rollRaw")?;
    let roll_items = code.method("fish.game.evsheet.EvSheet_manager_ev", "rollItemsRaw")?;
    let load = code.method("fish.system.LevelManager", "loadCurrentLevel")?;
    let push = code.method("hl.types.ArrayObj", "push")?;
    let (bool_t, dyn_t) = (code.ty_bool(), code.ty_dyn());
    let reload = !o.level.is_empty() || o.modifier >= 0 || !o.items.is_empty() || o.list_levels;

    let mut f = hooks::handler(code, "tick", "harness/tick")?;
    let layout = f.arg(1);
    let (running, end) = (f.label(), f.label());
    let t = f.get_new(layout, "currentTick")?;
    let st = get_global(&mut f, state);
    let zero = f.const_i32(0);
    f.jeq(st, zero, end); // still on the title screen
    let one_ = f.const_i32(1);
    f.jne(st, one_, running);

    // ---- first gameplay ticks (the run's first level): set up the test level
    if !reload {
        set_global(&mut f, state, 2);
        f.print(&[Print::Str("[harness] level started (first level of the run)")])?;
        f.jmp(running);
    } else {
        let two = f.const_i32(2);
        f.jlt(t, two, end);
        set_global(&mut f, state, 2);
        let main = main_instance(&mut f)?;
        let game = f.get_new(main, "game")?;
        let lm = f.get_new(game, "levelManager")?;
        let pool = f.get_new(lm, "levelPool")?;
        let n = f.array_len(pool)?;

        if o.list_levels {
            f.for_range(n, |f, i| {
                let e = f.array_get(pool, i, level_t)?;
                let name = f.get_new(e, "name")?;
                let ln = f.get_new(e, "levelN")?;
                f.print(&[Print::Str("[harness] level "), Print::Val(name), Print::Str(" level_n="), Print::Val(ln)])
            })?;
            let im = f.get_new(game, "itemManager")?;
            let item_pool = f.get_new(im, "itemPool")?;
            let pn = f.array_len(item_pool)?;
            f.for_range(pn, |f, i| {
                let it = f.array_get(item_pool, i, pool_item_t)?;
                let nm = f.get_new(it, "name")?;
                let ammo = f.get_new(it, "baseAmmo")?;
                let aim = f.get_new(it, "aimType")?;
                f.print(&[Print::Str("[harness] item "), Print::Val(nm), Print::Str(" ammo="), Print::Val(ammo), Print::Str(" aim="), Print::Val(aim)])
            })?;
            f.print(&[Print::Str("[harness] end (list_levels)")])?;
            f.exit(0)?;
        }

        if !o.level.is_empty() {
            let found = f.reg(dyn_t);
            f.op(Opcode::Null { dst: found });
            f.for_range(n, |f, i| {
                let next = f.label();
                let native = f.get_new(pool, "array")?;
                let raw = f.reg(dyn_t);
                f.op(Opcode::GetArray { dst: raw, array: native, index: i });
                let e = f.cast(raw, level_t);
                let name = f.get_new(e, "name")?;
                f.jstr_ne(name, &o.level, next)?;
                if o.level_n >= 0 {
                    let ln = f.get_new(e, "levelN")?;
                    let want = f.const_i32(o.level_n);
                    f.jne(ln, want, next);
                }
                f.mov(found, raw);
                f.place(next);
                Ok(())
            })?;
            let ok = f.label();
            f.jnotnull(found, ok);
            f.print(&[Print::Str(&format!("[harness] ERROR level `{}` not found (set list_levels = true)", o.level))])?;
            f.exit(3)?;
            f.place(ok);
            // levelPool = [found]
            f.set(pool, "length", zero)?;
            f.call_new(push, &[pool, found])?;
            let no = f.reg(bool_t);
            f.bool(no, false);
            f.set(lm, "shouldRefreshPool", no)?;
        }

        // Like Main.renderLevelPreview: mark the tutorial done, or the run forces `jeppetutorial`.
        let inst_ev = f.get_new(game, "ev_instancing_ev")?;
        let picker = f.get_new(inst_ev, "manager")?;
        let first = f.code().method("fish.system.Picker", "first")?;
        let m_dyn = f.call_new(first, &[picker])?;
        let manager_t = f.code().class("fish.game.oclass.OClass_manager")?;
        let manager = f.cast(m_dyn, manager_t);
        let yes = f.reg(bool_t);
        f.bool(yes, true);
        f.set(manager, "tutorial_done", yes)?;

        let rand = f.new_obj("hxd.Rand")?;
        let seed = f.const_i32(o.seed);
        f.call_new(rand_init, &[rand, seed])?;
        let no = f.reg(bool_t);
        f.bool(no, false);
        f.call_new(roll, &[lm, rand, no, no, no])?;
        if o.modifier >= 0 {
            let cur = f.get_new(lm, "currentLevel")?;
            let m = f.const_i32(o.modifier);
            f.set(cur, "modifier", m)?;
        }
        let mgr = f.get_new(game, "ev_manager_ev")?;
        f.call_new(roll_items, &[mgr])?;

        if !o.items.is_empty() {
            let im = f.get_new(game, "itemManager")?;
            let item_pool = f.get_new(im, "itemPool")?;
            let current = f.get_new(im, "currentItems")?;
            let picker = f.get_new(mgr, "b_item")?;
            let slots = f.get_new(picker, "insts")?;
            for (k, name) in o.items.iter().enumerate() {
                let found = f.reg(pool_item_t);
                f.op(Opcode::Null { dst: found });
                let pn = f.array_len(item_pool)?;
                f.for_range(pn, |f, i| {
                    let next = f.label();
                    let it = f.array_get(item_pool, i, pool_item_t)?;
                    let nm = f.get_new(it, "name")?;
                    f.jstr_ne(nm, name, next)?;
                    f.mov(found, it);
                    f.place(next);
                    Ok(())
                })?;
                let ok = f.label();
                f.jnotnull(found, ok);
                f.print(&[Print::Str(&format!("[harness] ERROR item `{name}` not in the item pool"))])?;
                f.exit(3)?;
                f.place(ok);
                // currentItems[k].type = found (the record is shared with slot k's `item`)
                let kr = f.const_i32(k as i32);
                let done = f.label();
                let cn = f.array_len(current)?;
                f.jge(kr, cn, done);
                let rec = f.array_get(current, kr, slot_t)?;
                f.set(rec, "type", found)?;
                // slot k's ammo, if the slot objects exist yet
                let sn = f.array_len(slots)?;
                f.jge(kr, sn, done);
                let slot = f.array_get(slots, kr, b_item_t)?;
                let ammo = f.get_new(found, "baseAmmo")?;
                f.set(slot, "ammo", ammo)?;
                f.place(done);
            }
        }
        f.call_new(load, &[lm, game])?;
        f.print(&[Print::Str(&format!(
            "[harness] loading level `{}` seed {} modifier {} items {:?}",
            o.level, o.seed, o.modifier, o.items
        ))])?;
        f.jmp(end);
    }

    // ---- the test level is running
    f.place(running);
    let one = f.const_i32(1);
    let not_first = f.label();
    f.jne(t, one, not_first);
    {
        let main = main_instance(&mut f)?;
        let game = f.get_new(main, "game")?;
        let lm = f.get_new(game, "levelManager")?;
        let cur = f.get_new(lm, "currentLevel")?;
        let ty = f.get_new(cur, "type")?;
        let name = f.get_new(ty, "name")?;
        let m = f.get_new(cur, "modifier")?;
        let st = f.static_obj("fish.system.Main")?;
        let dt = f.get_new(st, "frameTime")?;
        f.print(&[
            Print::Str("[harness] level tick 1: "), Print::Val(name), Print::Str(" modifier "), Print::Val(m),
            Print::Str(" frameTime "), Print::Val(dt),
        ])?;
    }
    f.place(not_first);
    if let (Some((a, b, step)), Some(cap)) = (o.capture, capture) {
        let skip = f.label();
        let (ar, br, sr) = (f.const_i32(a), f.const_i32(b), f.const_i32(step));
        f.jlt(t, ar, skip);
        f.jgt(t, br, skip);
        let d = f.reg(f.reg_type(t));
        f.sub(d, t, ar);
        let rem = f.reg(f.reg_type(t));
        f.op(Opcode::SMod { dst: rem, a: d, b: sr });
        f.jne(rem, zero, skip);
        let idx = f.reg(f.reg_type(t));
        f.op(Opcode::SDiv { dst: idx, a: d, b: sr });
        let void = f.code().ty_void();
        let r = f.reg(void);
        f.call(r, cap, &[idx]);
        f.place(skip);
    }
    if o.end_tick > 0 {
        let e = f.const_i32(o.end_tick);
        f.jlt(t, e, end);
        f.print(&[Print::Str("[harness] end at tick "), Print::Val(t)])?;
        f.exit(0)?;
    }
    f.place(end);
    f.ret_void();
    f.finish()
}

/// `capture(n)`: render the game at 600×338 into `Main.gifTarget` and save it as a PNG.
fn build_capture(code: &mut Code, dir: &str) -> Result<RefFun> {
    let resize = code.method("h3d.Engine", "resize")?;
    let on_resize = code.method("fish.system.Game", "onResize")?;
    let push_target = code.method("h3d.Engine", "pushTarget")?;
    let pop_target = code.method("h3d.Engine", "popTarget")?;
    let render = code.method("fish.system.Game", "render")?;
    let capture = code.method("h3d.mat.Texture", "capturePixels")?;
    let to_png = code.method("hxd.Pixels", "toPNG")?;
    let save = code.method("sys.io.File", "saveBytes")?;
    let push_args = code.func_type(push_target)?.args.clone();
    let cap_args = code.func_type(capture)?.args.clone();
    let png_args = code.func_type(to_png)?.args.clone();
    let (i32_t, f64_t, bool_t, void) = (code.ty_i32(), code.ty_f64(), code.ty_bool(), code.ty_void());

    let mut f = FnBuilder::new(code, "harness/capture", &[i32_t], void);
    let n = f.arg(0);
    let main = main_instance(&mut f)?;
    let engine = f.get_new(main, "engine")?;
    let game = f.get_new(main, "game")?;
    let target = f.get_new(main, "gifTarget")?;
    let have = f.label();
    f.jnotnull(target, have);
    f.print(&[Print::Str("[harness] WARNING no gifTarget, frame not captured")])?;
    f.ret_void();
    f.place(have);
    let (w, h) = (f.get_new(engine, "width")?, f.get_new(engine, "height")?);
    let (cw, ch) = (f.const_i32(600), f.const_i32(338));
    let (cwf, chf) = (f.const_f64(600.0), f.const_f64(338.0));
    f.call_new(resize, &[engine, cw, ch])?;
    f.call_new(on_resize, &[game, cwf, chf])?;
    let nulls: Vec<Reg> = push_args[2..].iter().map(|t| {
        let r = f.reg(*t);
        f.op(Opcode::Null { dst: r });
        r
    }).collect();
    f.call_new(push_target, &[engine, target, nulls[0], nulls[1], nulls[2]])?;
    let no = f.reg(bool_t);
    f.bool(no, false);
    f.call_new(render, &[game, engine, no])?;
    f.call_new(pop_target, &[engine])?;
    let cnulls: Vec<Reg> = cap_args[1..].iter().map(|t| {
        let r = f.reg(*t);
        f.op(Opcode::Null { dst: r });
        r
    }).collect();
    let pixels = f.call_new(capture, &[target, cnulls[0], cnulls[1], cnulls[2]])?;
    let (wf, hf) = (f.reg(f64_t), f.reg(f64_t));
    f.op(Opcode::ToSFloat { dst: wf, src: w });
    f.op(Opcode::ToSFloat { dst: hf, src: h });
    f.call_new(resize, &[engine, w, h])?;
    f.call_new(on_resize, &[game, wf, hf])?;
    let level = f.reg(png_args[1]);
    f.op(Opcode::Null { dst: level });
    let png = f.call_new(to_png, &[pixels, level])?;
    let big = f.const_i32(10000);
    let num = f.reg(i32_t);
    f.add(num, n, big);
    let path = f.string_of(&[Print::Str(dir), Print::Str("/frame_"), Print::Val(num), Print::Str(".png")])?;
    f.call_new(save, &[path, png])?;
    f.ret_void();
    f.finish()
}

/// Guard `PlayerInputs.updateSP`: while the test level runs, feed the scripted bits with
/// `readBin` (the replay path) instead of reading the keyboard.
fn script_inputs(code: &mut Code, o: &Opts, state: RefGlobal) -> Result<()> {
    let update_sp = code.method("fish.system.PlayerInputs", "updateSP")?;
    let read_bin = code.method("fish.system.PlayerInputs", "readBin")?;
    let pi_t = code.class("fish.system.PlayerInputs")?;
    let (bool_t, void) = (code.ty_bool(), code.ty_void());

    let mut f = FnBuilder::new(code, "harness/inputs", &[pi_t], bool_t);
    let pi = f.arg(0);
    let no = f.label();
    let st = get_global(&mut f, state);
    let two = f.const_i32(2);
    f.jne(st, two, no);
    let main = main_instance(&mut f)?;
    let game = f.get_new(main, "game")?;
    let layouts = f.get_new(game, "layouts")?;
    let layout = f.get_new(layouts, "mainLayout")?;
    let t = f.get_new(layout, "currentTick")?;
    let bits = f.const_i32(0);
    for &(from, to, b) in &o.inputs {
        let skip = f.label();
        let (a, z) = (f.const_i32(from), f.const_i32(to));
        f.jlt(t, a, skip);
        f.jge(t, z, skip);
        let br = f.const_i32(b);
        f.op(Opcode::Or { dst: bits, a: bits, b: br });
        f.place(skip);
    }
    f.call_new(read_bin, &[pi, bits])?;
    let yes = f.reg(bool_t);
    f.bool(yes, true);
    f.ret(yes);
    f.place(no);
    let r = f.reg(bool_t);
    f.bool(r, false);
    f.ret(r);
    let handler = f.finish()?;

    // updateSP(this): if (harness_inputs(this)) return;
    let fun = code.func_mut(update_sp)?;
    let ok = add_reg(fun, bool_t);
    let v = add_reg(fun, void);
    insert_ops(
        fun,
        0,
        vec![call(ok, handler, &[Reg(0)]), Opcode::JFalse { cond: ok, offset: 1 }, Opcode::Ret { ret: v }],
        Incoming::ToOriginal,
    );
    Ok(())
}
