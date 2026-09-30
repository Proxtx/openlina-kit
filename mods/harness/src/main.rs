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
    roll_modifier: bool,
    pause_tick: i32,
    dump_menu: bool,
    capture_ui: bool,
    menu_open: String,
    new_run: bool,
    new_run_items: Vec<String>,
    turbo: i32,
    heartbeat: i32,
    chaos: bool,
    end_total: i32,
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
        bits |= 1
            << match a {
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
        modifier: match cfg.str("modifier_key", "")? {
            "" => cfg.i64("modifier", -1)? as i32,
            key => openlina_sdk::modifiers::id_of(key),
        },
        items: list(cfg, "items")?,
        inputs: list(cfg, "inputs")?.iter().map(|s| parse_input(s)).collect::<Result<_>>()?,
        end_tick: cfg.i64("end_tick", 0)? as i32,
        capture: parse_capture(cfg.str("capture", "")?)?,
        capture_dir: cfg.str("capture_dir", "frames")?.to_string(),
        list_levels: cfg.bool("list_levels", false)?,
        roll_modifier: cfg.bool("roll_modifier", false)?,
        pause_tick: cfg.i64("pause_tick", 0)? as i32,
        dump_menu: cfg.bool("dump_menu", false)?,
        capture_ui: cfg.bool("capture_ui", false)?,
        menu_open: cfg.str("menu_open", "")?.to_string(),
        new_run: cfg.bool("new_run", false)?,
        new_run_items: list(cfg, "new_run_items")?,
        turbo: cfg.i64("turbo", 16)? as i32,
        heartbeat: cfg.i64("heartbeat", 1200)? as i32,
        chaos: cfg.bool("chaos", false)?,
        end_total: cfg.i64("end_total", 0)? as i32,
    };

    let i32_t = code.ty_i32();
    // 0 = title screen (which also runs gameplay ticks at startup: ignore those),
    // 1 = title skipped, 2 = the test level is running.
    let state = code.add_global(i32_t);

    skip_title(code, o.start_tick, o.new_run.then_some(o.seed), state)?;
    if o.turbo > 0 {
        turbo(code, o.turbo)?;
    }
    if o.heartbeat > 0 {
        heartbeat(code, o.heartbeat)?;
    }
    let capture = if o.capture.is_some() { Some(build_capture(code, &o.capture_dir, o.capture_ui)?) } else { None };
    if o.new_run {
        run_start_clock(code, &o, capture)?;
        if !o.new_run_items.is_empty() {
            force_rerolled_items(code, &o.new_run_items)?;
        }
    }
    let tick = build_tick(code, &o, state, capture)?;
    hooks::subscribe(code, "tick", tick)?;
    if !o.inputs.is_empty() || o.chaos {
        script_inputs(code, &o, state)?;
    }
    if o.end_total > 0 {
        end_total(code, o.end_total)?;
    }
    Ok(())
}

/// At title tick `tick`: `levelManager.refreshPool(Main.i.packManager); layout.goToLayout("main")`.
fn skip_title(code: &mut Code, tick: i32, new_run: Option<i32>, state: RefGlobal) -> Result<()> {
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
    if let Some(seed) = new_run {
        // A returning player's run: no tutorial, the game's own run start (tool selection) follows.
        let inst_ev = f.get_new(game, "ev_instancing_ev")?;
        let picker = f.get_new(inst_ev, "manager")?;
        let first = f.code().method("fish.system.Picker", "first")?;
        let m_dyn = f.call_new(first, &[picker])?;
        let manager_t = f.code().class("fish.game.oclass.OClass_manager")?;
        let manager = f.cast(m_dyn, manager_t);
        let yes = f.reg_bool();
        f.bool(yes, true);
        f.set(manager, "tutorial_done", yes)?;
        seed_run(&mut f, manager, seed)?;
    }
    let name = f.string_obj("main")?;
    f.call_new(goto, &[layout, name])?;
    set_global(&mut f, state, 1);
    f.print(&[Print::Str("[harness] title skipped")])?;
    f.place(skip);
    f.ret_void();
    let start = f.finish()?;
    prepend_call(code, update, start, &[Reg(1)])
}

/// Seed every RNG of the run (items come from `toolSeed`, which is random at startup).
fn seed_run(f: &mut FnBuilder, manager: Reg, seed: i32) -> Result<()> {
    let rand_init = f.code().method("hxd.Rand", "init")?;
    for (k, field) in [
        "mainSeed",
        "levelSeed",
        "toolSeed",
        "toolBlockSeed",
        "modifierSeed",
        "colorSeed",
        "musicSeed",
        "sfxSeed",
        "bossSeed",
        "endwormSeed",
    ]
    .iter()
    .enumerate()
    {
        let skip = f.label();
        let r = f.get_new(manager, field)?;
        f.jnull(r, skip);
        let s = f.const_i32(seed.wrapping_mul(31).wrapping_add(k as i32));
        f.call_new(rand_init, &[r, s])?;
        f.place(skip);
    }
    Ok(())
}

/// `Main.mainLoop` adds the real time since the last frame to `accum` and runs one game step per
/// `Main.frameTime` (1/120 s) in it, then renders once: the game runs in real time even headless.
/// With `turbo`, every frame also gets `turbo` extra steps. Steps keep their fixed length, so runs
/// stay deterministic; only the wall-clock time shrinks.
fn turbo(code: &mut Code, steps: i32) -> Result<()> {
    let main_loop = code.method("fish.system.Main", "mainLoop")?;
    let main_t = code.class("fish.system.Main")?;
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, "harness/turbo", &[main_t], void);
    let main = f.arg(0);
    let st = f.static_obj("fish.system.Main")?;
    let ft = f.get_new(st, "frameTime")?;
    let n = f.const_f64(steps as f64);
    let extra = f.reg_f64();
    f.mul(extra, ft, n);
    let acc = f.get_new(main, "accum")?;
    f.add(acc, acc, extra);
    f.set(main, "accum", acc)?;
    f.ret_void();
    let h = f.finish()?;
    prepend_call(code, main_loop, h, &[Reg(0)])
}

/// `chaos`: from tick 60 of every layout but the title screen, when no scripted input is active,
/// random input bits (up down left right jump shoot switch; never restart), new every 15 ticks.
/// In levels that is random play; in the hub it wanders off (a new run), on the tool selection it
/// confirms. The generator is seeded once, so a scenario replays the same way.
fn chaos_bits(f: &mut FnBuilder, layout: Reg, t: Reg, bits: Reg) -> Result<()> {
    let i32_t = f.code().ty_i32();
    let state = f.code().add_global(i32_t);
    let current = f.code().add_global(i32_t);
    let skip = f.label();
    let zero = f.const_i32(0);
    f.jne(bits, zero, skip); // scripted input wins
    let name = f.get_new(layout, "name")?;
    let title = f.string_obj("first_screen")?;
    f.jeq(name, title, skip);
    let sixty = f.const_i32(60);
    f.jlt(t, sixty, skip);
    // every 15 ticks: state = state * 1103515245 + 12345; current = (state >> 16) & 0x7f
    let (fifteen, m) = (f.const_i32(15), f.reg_i32());
    f.op(Opcode::SMod { dst: m, a: t, b: fifteen });
    let keep = f.label();
    f.jne(m, zero, keep);
    let st = f.get_global(state);
    let (mul, add) = (f.const_i32(1103515245), f.const_i32(12345));
    f.op(Opcode::Mul { dst: st, a: st, b: mul });
    f.op(Opcode::Add { dst: st, a: st, b: add });
    f.set_global(state, st);
    let (sh, mask, v) = (f.const_i32(16), f.const_i32(0x7f), f.reg_i32());
    f.op(Opcode::UShr { dst: v, a: st, b: sh });
    f.op(Opcode::And { dst: v, a: v, b: mask });
    f.set_global(current, v);
    f.place(keep);
    let v = f.get_global(current);
    f.mov(bits, v);
    f.place(skip);
    Ok(())
}

/// `end_total`: exit (code 0) after this many game steps in total, whatever the screen
/// (`fish.system.Game.update` runs once per step everywhere): for soak runs that die, restart
/// and roll new runs.
fn end_total(code: &mut Code, steps: i32) -> Result<()> {
    let update = code.method("fish.system.Game", "update")?;
    let args = code.func_type(update)?.args.clone();
    let (void, i32_t) = (code.ty_void(), code.ty_i32());
    let count = code.add_global(i32_t);
    let mut f = FnBuilder::new(code, "harness/end_total", &args[..1], void);
    let end = f.label();
    let n = f.get_global(count);
    f.op(Opcode::Incr { dst: n });
    f.set_global(count, n);
    let lim = f.const_i32(steps);
    f.jlt(n, lim, end);
    f.print(&[Print::Str("[harness] end after "), Print::Val(n), Print::Str(" steps")])?;
    f.exit(0)?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    prepend_call(code, update, h, &[Reg(0)])
}

/// A sign of life from `Main.mainLoop`, which runs on every screen: `[harness] layout <name>
/// tick <t>` whenever the main layout changes and every `every` ticks of it. When a run hangs or
/// waits, the log's last lines say where (`lina test` shows them).
fn heartbeat(code: &mut Code, every: i32) -> Result<()> {
    let main_loop = code.method("fish.system.Main", "mainLoop")?;
    let main_t = code.class("fish.system.Main")?;
    let layout_t = code.class("fish.system.Layout")?;
    let (void, i32_t) = (code.ty_void(), code.ty_i32());
    let last_layout = code.add_global(layout_t);
    let last_bucket = code.add_global(i32_t);
    let mut f = FnBuilder::new(code, "harness/heartbeat", &[main_t], void);
    let main = f.arg(0);
    let end = f.label();
    let game = f.get_new(main, "game")?;
    f.jnull(game, end);
    let layouts = f.get_new(game, "layouts")?;
    f.jnull(layouts, end);
    let layout = f.get_new(layouts, "mainLayout")?;
    f.jnull(layout, end);
    let t = f.get_new(layout, "currentTick")?;
    let n = f.const_i32(every);
    let bucket = f.reg_i32();
    f.op(Opcode::SDiv { dst: bucket, a: t, b: n });
    let (report, same_layout) = (f.label(), f.label());
    let prev = f.get_global(last_layout);
    f.jeq(prev, layout, same_layout);
    f.set_global(last_layout, layout);
    f.jmp(report);
    f.place(same_layout);
    let pb = f.get_global(last_bucket);
    f.jeq(pb, bucket, end);
    f.place(report);
    f.set_global(last_bucket, bucket);
    let name = f.get_new(layout, "name")?;
    f.print(&[Print::Str("[harness] layout "), Print::Val(name), Print::Str(" tick "), Print::Val(t)])?;
    f.place(end);
    f.ret_void();
    let h = f.finish()?;
    prepend_call(code, main_loop, h, &[Reg(0)])
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
    let rand_init = code.method("hxd.Rand", "init")?;
    let roll = code.method("fish.system.LevelManager", "rollRaw")?;
    let roll_items = code.method("fish.game.evsheet.EvSheet_manager_ev", "rollItemsRaw")?;
    let load = code.method("fish.system.LevelManager", "loadCurrentLevel")?;
    let push = code.method("hl.types.ArrayObj", "push")?;
    let (bool_t, dyn_t) = (code.ty_bool(), code.ty_dyn());
    let reload = !o.level.is_empty() || o.modifier >= 0 || !o.items.is_empty() || o.list_levels || o.roll_modifier;

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
                f.print(&[
                    Print::Str("[harness] item "),
                    Print::Val(nm),
                    Print::Str(" ammo="),
                    Print::Val(ammo),
                    Print::Str(" aim="),
                    Print::Val(aim),
                ])
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
        seed_run(&mut f, manager, o.seed)?;

        let rand = f.new_obj("hxd.Rand")?;
        let seed = f.const_i32(o.seed);
        f.call_new(rand_init, &[rand, seed])?;
        // rollRaw(rng, hasModifier, dx, coop): with hasModifier the game draws a modifier from its
        // pool (the decompiler shows these parameter names shifted by one).
        let no = f.reg(bool_t);
        f.bool(no, false);
        let has_mod = f.reg(bool_t);
        f.bool(has_mod, o.roll_modifier);
        f.call_new(roll, &[lm, rand, has_mod, no, no])?;
        if o.modifier >= 0 {
            let cur = f.get_new(lm, "currentLevel")?;
            let m = f.const_i32(o.modifier);
            f.set(cur, "modifier", m)?;
        }
        let mgr = f.get_new(game, "ev_manager_ev")?;
        if o.items.is_empty() {
            f.call_new(roll_items, &[mgr])?;
        } else {
            // Roll from a temporary pool holding just the requested items, so the game's own code
            // assigns them (and their ammo, including changes made by other mods). The slot order
            // follows the seeded roll.
            let im = f.get_new(game, "itemManager")?;
            let item_pool = f.get_new(im, "itemPool")?;
            let mut chosen = Vec::new();
            for name in &o.items {
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
                chosen.push(found);
            }
            // The roll draws 3 items and the 4th slot copies the 2nd: repeat the requested items
            // so the pool never runs short.
            let padded: Vec<Reg> = chosen.iter().cycle().take(chosen.len().max(3)).copied().collect();
            let temp = f.new_array_obj(pool_item_t, &padded)?;
            f.set(im, "itemPool", temp)?;
            f.call_new(roll_items, &[mgr])?;
            f.set(im, "itemPool", item_pool)?;
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
            Print::Str("[harness] level tick 1: "),
            Print::Val(name),
            Print::Str(" modifier "),
            Print::Val(m),
            Print::Str(" frameTime "),
            Print::Val(dt),
        ])?;
        // [harness] slot k: <item> ammo <n>
        let mgr = f.get_new(game, "ev_manager_ev")?;
        let picker = f.get_new(mgr, "b_item")?;
        let slots = f.get_new(picker, "insts")?;
        let n = f.array_len(slots)?;
        f.for_range(n, |f, k| {
            let slot = f.array_get(slots, k, b_item_t)?;
            let skip = f.label();
            let rec = f.get_new(slot, "item")?;
            f.jnull(rec, skip);
            let ty = f.get_new(rec, "type")?;
            f.jnull(ty, skip);
            let nm = f.get_new(ty, "name")?;
            let ammo = f.get_new(slot, "ammo")?;
            f.print(&[
                Print::Str("[harness] slot "),
                Print::Val(k),
                Print::Str(": "),
                Print::Val(nm),
                Print::Str(" ammo "),
                Print::Val(ammo),
            ])?;
            f.place(skip);
            Ok(())
        })?;
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
    if o.pause_tick > 0 {
        // Open the pause menu (Main.render shows it while `localInputs.paused`).
        let skip = f.label();
        let p = f.const_i32(o.pause_tick);
        f.jne(t, p, skip);
        let main = main_instance(&mut f)?;
        let inputs = f.get_new(main, "localInputs")?;
        let yes = f.reg_bool();
        f.bool(yes, true);
        f.set(inputs, "paused", yes)?;
        f.print(&[Print::Str("[harness] paused")])?;
        if o.dump_menu {
            let menu = f.get_new(main, "menu")?;
            let items = f.get_new(menu, "items")?;
            let item_t = f.code().class("bib.MenuItem")?;
            let n = f.array_len(items)?;
            f.for_range(n, |f, i| {
                let it = f.array_get(items, i, item_t)?;
                let text = f.get_new(it, "text")?;
                f.print(&[Print::Str("[harness] menu: "), Print::Val(text)])
            })?;
        }
        f.place(skip);
    }
    if o.pause_tick > 0 && !o.menu_open.is_empty() {
        // One tick after pausing: select the item with this text and press it (opens a submenu).
        let skip = f.label();
        let p = f.const_i32(o.pause_tick + 2);
        f.jne(t, p, skip);
        let main = main_instance(&mut f)?;
        let menu = f.get_new(main, "menu")?;
        let items = f.get_new(menu, "items")?;
        let item_t = f.code().class("bib.MenuItem")?;
        let select = f.code().method("bib.Menu", "selectItem")?;
        let exec = f.code().method("fish.system.PauseMenu", "exec")?;
        let n = f.array_len(items)?;
        let target = o.menu_open.clone();
        f.for_range(n, |f, i| {
            let next = f.label();
            let it = f.array_get(items, i, item_t)?;
            let text = f.get_new(it, "text")?;
            f.jstr_ne(text, &target, next)?;
            f.call_new(select, &[menu, i])?;
            f.call_new(exec, &[menu])?;
            f.print(&[Print::Str(&format!("[harness] opened menu item `{target}`"))])?;
            f.place(next);
            Ok(())
        })?;
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

/// With `new_run`: the run start (tool selection) runs no gameplay ticks, so capture and
/// `end_tick` also follow `mainLayout.currentTick` from `EvSheet_manager_ev.update`, which runs
/// every frame. Scripted inputs already use that clock.
fn run_start_clock(code: &mut Code, o: &Opts, capture: Option<RefFun>) -> Result<()> {
    let update = code.method("fish.game.evsheet.EvSheet_manager_ev", "update")?;
    let sheet_t = code.class("fish.game.evsheet.EvSheet_manager_ev")?;
    let void = code.ty_void();
    let mut f = FnBuilder::new(code, "harness/run_start_clock", &[sheet_t], void);
    let end = f.label();
    let main = main_instance(&mut f)?;
    let game = f.get_new(main, "game")?;
    let layouts = f.get_new(game, "layouts")?;
    let layout = f.get_new(layouts, "mainLayout")?;
    let t = f.get_new(layout, "currentTick")?;
    if let (Some((from, to, step)), Some(cap)) = (o.capture, capture) {
        let skip = f.label();
        let (a, z, s) = (f.const_i32(from), f.const_i32(to), f.const_i32(step.max(1)));
        f.jlt(t, a, skip);
        f.jgt(t, z, skip);
        let i32_t = f.code().ty_i32();
        let (d, m) = (f.reg(i32_t), f.reg(i32_t));
        f.sub(d, t, a);
        f.op(Opcode::SMod { dst: m, a: d, b: s });
        let zero = f.const_i32(0);
        f.jne(m, zero, skip);
        f.call_new(cap, &[t])?;
        f.place(skip);
    }
    if o.end_tick > 0 {
        let e = f.const_i32(o.end_tick);
        f.jlt(t, e, end);
        f.print(&[Print::Str("[harness] end at layout tick "), Print::Val(t)])?;
        f.exit(0)?;
    }
    f.place(end);
    f.ret_void();
    let clock = f.finish()?;
    prepend_call(code, update, clock, &[Reg(0)])
}

/// `new_run_items`: the run start's tool roll (`ItemManager.reroll`, called by
/// `EvSheet_manager_ev.manage`) puts these items into the first slots of `pickedItems`, so a test
/// can see how the tool selection handles them.
fn force_rerolled_items(code: &mut Code, names: &[String]) -> Result<()> {
    let reroll = code.method("fish.system.ItemManager", "reroll")?;
    let find_item = code.method("fish.system.ItemManager", "findItem")?;
    let manage = code.method("fish.game.evsheet.EvSheet_manager_ev", "manage")?;
    let ft = code.func_type(reroll)?.clone();
    let slot_t = code.func_type(code.method("fish.system.ItemManager", "findPicked")?)?.ret;
    let mut f = FnBuilder::new(code, "harness/reroll", &ft.args, ft.ret);
    let args: Vec<Reg> = (0..ft.args.len()).map(|i| f.arg(i)).collect();
    f.call_new(reroll, &args)?;
    let im = args[0];
    let picked = f.get_new(im, "pickedItems")?;
    let n = f.array_len(picked)?;
    for (k, name) in names.iter().enumerate() {
        let skip = f.label();
        let kr = f.const_i32(k as i32);
        f.jge(kr, n, skip);
        let slot = f.array_get(picked, kr, slot_t)?;
        let nm = f.string_obj(name)?;
        let ty = f.call_new(find_item, &[im, nm])?;
        f.set(slot, "type", ty)?;
        f.print(&[Print::Str(&format!("[harness] run start: tool {k} forced to `{name}`"))])?;
        f.place(skip);
    }
    f.ret_void();
    let wrapper = f.finish()?;
    let fun = code.func_mut(manage)?;
    let at = openlina_sdk::edit::expect_one(openlina_sdk::edit::find_calls(fun, reroll), "reroll call in manage")?;
    let (_, call_args) = openlina_sdk::edit::call_target(&fun.ops[at]).context("not a call")?;
    let dst = match fun.ops[at] {
        Opcode::Call4 { dst, .. } => dst,
        _ => bail!("reroll call in manage is not a Call4"),
    };
    openlina_sdk::edit::replace_op(fun, at, call(dst, wrapper, &call_args));
    Ok(())
}

/// `capture(n)`: render the game at 600×338 into `Main.gifTarget` and save it as a PNG.
fn build_capture(code: &mut Code, dir: &str, ui: bool) -> Result<RefFun> {
    let scene_render = code.method("h2d.Scene", "render")?;
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
    let nulls: Vec<Reg> = push_args[2..]
        .iter()
        .map(|t| {
            let r = f.reg(*t);
            f.op(Opcode::Null { dst: r });
            r
        })
        .collect();
    f.call_new(push_target, &[engine, target, nulls[0], nulls[1], nulls[2]])?;
    let no = f.reg(bool_t);
    f.bool(no, false);
    f.call_new(render, &[game, engine, no])?;
    if ui {
        // The UI layer (pause menu, …) is Main's own 2D scene, drawn after the game.
        let s2d = f.get_new(main, "s2d")?;
        f.call_new(scene_render, &[s2d, engine])?;
    }
    f.call_new(pop_target, &[engine])?;
    let cnulls: Vec<Reg> = cap_args[1..]
        .iter()
        .map(|t| {
            let r = f.reg(*t);
            f.op(Opcode::Null { dst: r });
            r
        })
        .collect();
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
    if o.chaos {
        chaos_bits(&mut f, layout, t, bits)?;
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
