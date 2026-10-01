# Mosa Lina internals

What we know about the game code, gathered while writing mods, by topic. Read the one you need; add what you find
to it (with how you found it, e.g. "found by the swap mod").

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build.

| file | what's in it |
|---|---|
| [game/engine.md](game/engine.md) | builds and runtime, the Construct-style engine (layouts, object types, pickers, event sheets, behaviors), creating objects, layers and draw order, the main loop and timing, randomness, rendering a frame, image files (which PNGs load), sounds (`sfx`, the list of effects) |
| [game/world.md](game/world.md) | the main gameplay tick, fruits and winning, the 600×338 play field and the edge test (also Lina's), gravity (the game's own force, not Box2D's), cannons and how they wake, ray casts, static geometry |
| [game/items.md](game/items.md) | the item pool, slots and ammo, firing, aim types, item objects and icons (tool selection, editor), the run start's tool roll, raw random |
| [game/levels.md](game/levels.md) | title → hub → run, loading levels, the level pool, level data (`mld.Level`/`mld.Object`), level packs, tile sizes |
| [game/modifiers.md](game/modifiers.md) | the modifier rolls and pools (dx, co-op), the HUD icon |
| [game/input.md](game/input.md) | the keyboard map, `PlayerInputs` bits (what the harness feeds), jump on press, aiming directions |
