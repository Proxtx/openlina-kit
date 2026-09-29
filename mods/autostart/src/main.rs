//! `autostart`: skip the title screen without input.
//!
//! Synthetic input doesn't reach the game, so automated tests couldn't get past "press any
//! button". On any key, `EvSheet_first_screen_ev.update` (source L123-) runs:
//!
//! ```haxe
//! this.game.levelManager.refreshPool(Main.i.packManager);
//! layout.goToLayout("main");
//! ```
//!
//! This mod does the same at title-screen tick `tick`, from a hook at the start of that function.

use anyhow::Result;
use openlina_sdk::asm::FnBuilder;
use openlina_sdk::edit::prepend_call;
use openlina_sdk::hlbc::types::Reg;
use openlina_sdk::{Code, ModConfig};

const TITLE: &str = "fish.game.evsheet.EvSheet_first_screen_ev";

fn main() {
    openlina_sdk::run_mod(apply)
}

fn apply(code: &mut Code, cfg: &ModConfig) -> Result<()> {
    let tick = cfg.i64("tick", 30)? as i32;
    let update = code.method(TITLE, "update")?;
    let refresh = code.method("fish.system.LevelManager", "refreshPool")?;
    let goto = code.method("fish.system.Layout", "goToLayout")?;
    let layout_t = code.class("fish.system.Layout")?;
    let void = code.ty_void();

    // start(layout): if (layout.currentTick == tick) { refreshPool(...); layout.goToLayout("main"); }
    let mut f = FnBuilder::new(code, "autostart/start", &[layout_t], void);
    let layout = f.arg(0);
    let skip = f.label();
    let now = f.get_new(layout, "currentTick")?;
    let at = f.const_i32(tick);
    f.jne(now, at, skip);
    let main_static = f.static_obj("fish.system.Main")?;
    let main = f.get_new(main_static, "i")?;
    let game = f.get_new(main, "game")?;
    let lm = f.get_new(game, "levelManager")?;
    let pm = f.get_new(main, "packManager")?;
    f.call_new(refresh, &[lm, pm])?;
    let name = f.string_obj("main")?;
    f.call_new(goto, &[layout, name])?;
    f.place(skip);
    f.ret_void();
    let start = f.finish()?;

    // update(this, layout)
    prepend_call(code, update, start, &[Reg(1)])
}
