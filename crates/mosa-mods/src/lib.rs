//! All mods known to the `mosa` CLI. To add a mod, create a module implementing
//! [`mosa_bc::Mod`] and register it in [`all`].

use mosa_bc::Mod;

pub mod debug_spawn;
pub mod screen_wrap;
pub mod trace_calls;

/// Every available mod, in the order they are applied.
pub fn all() -> Vec<Box<dyn Mod>> {
    vec![
        Box::new(screen_wrap::ScreenWrap),
        Box::new(debug_spawn::DebugSpawn),
        Box::new(trace_calls::TraceCalls),
    ]
}
