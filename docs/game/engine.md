# Engine, runtime and timing

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

## Builds and runtime

- Haxe + Heaps + HashLink. `hlboot.dat` is HashLink bytecode version 4: 16388 functions, 15613 types,
  866 natives, with debug info (file/line per op).
- `Mosa Lina_jit <file>` runs any bytecode file. It must run with the game directory as the working directory
  (`fish/game/res`, `userdata/`) and with the game directory on `LD_LIBRARY_PATH` (`libturbojpeg.so.0`).
- The game's stdout carries `trace()` output and, on a crash, a HashLink stack trace with file:line. Injected
  functions show up as `openlina/<name>:<op index + 1>`.
- Saves and settings are in `<game>/userdata/`, shared between the native and the JIT builds.

## Engine architecture ("fish")

The game was made in Construct and ported to Haxe by a converter, so the code mirrors Construct's concepts:

| Construct | Haxe |
|---|---|
| layout (scene) | `fish.system.Layout`: `width`, `height`, `currentTick`, `worldScale`, Box2D `world`, `insts` |
| object type | `fish.game.oclass.OClass_<name>` (~1000 of them), base `fish.system.ObjectClass` (`sprite`, `physics`, `layout`, …) |
| family / object picking | `fish.system.Picker` (`insts`, `pick`, `first`, `call`) |
| event sheet | `fish.game.evsheet.EvSheet_<name>`, with `setupEvents(layout)` and a per-tick `update(layout)` |
| behavior | `fish.system.beh.*`: `Physics` (Box2D), `Rotate`, `Sin`, `Pin`, … `Bwrap` (Wrap) exists but is an empty stub |
| instance variables | fields on the `OClass_*` (e.g. `edgewith`, `glitched`, `type`) |

- Objects are created with `Layout.createObject(layout, typeName, layer, x, y, Bool, String, callback)` (fn@2973)
  and destroyed with `Sprite.destroy(sprite, null)` (fn@3075).
- `Physics.syncPosWithSprite` (fn@3029) pushes `sprite.position`/`angle` into the Box2D body whenever they
  differ from the cached values. **Setting `sprite.position.x/y` is enough to teleport a physics object**, and
  its velocity is kept.
- Positions are `hxmath.math.Vector2Default` (`x`, `y`: F64) in layout units.

## Main loop and timing

The game steps at a fixed `Main.frameTime` = 1/120 s: **120 ticks per second**. `Layout.currentTick` counts ticks
since the layout started. Runs are deterministic: the same bytecode, level, seed and inputs give the same ticks and
coordinates.

- `Main.mainLoop` runs once per rendered frame on every screen: it adds the real time since the last frame to
  `accum` and runs one game step per `Main.frameTime` in it, then renders once (so the game runs in real time,
  headless too; the harness `turbo` adds steps). `fish.system.Game.update` runs once per step everywhere;
  `EvSheet_gameplay.update` (core `tick` hook) only in gameplay layouts; `EvSheet_manager_ev.update` not in the hub.
- Randomness: the run's RNGs are `hxd.Rand` fields of the manager (`mainSeed`, `levelSeed`, `toolSeed`, …), seeded
  randomly at startup; the harness seeds them all from its `seed` option.
- `Picker.insts` is typed `hl.types.ArrayDyn` but holds an `hl.types.ArrayObj`; cast before indexing (the SDK's
  `array_get`/`array_len` do).
- The game keeps most string literals in globals initialized with the constant (`GetGlobal … // "text"`), not
  `String` ops: `lina refs` / `lina strings` find both.

## Rendering a frame

`Main.renderGifFrame` shows how: `engine.resize(600, 338); game.onResize(600, 338);
engine.pushTarget(Main.i.gifTarget); game.render(engine, false); engine.popTarget()`, then
`gifTarget.capturePixels().toPNG()`, and resize back. Works headless (SDL offscreen driver). The harness does
this for `lina gif`.

## Layers and draw order

Level layouts have the layers 0 "Layer 0", 1 "bg", 2 "display" (the level's objects) and 3 "debug"
(`layout.layers[].name`). `Layout.createObject(type, layer, …)` sets `sprite.layer`, and `addInstance` adds the
sprite's graphic to `layout.scene` (an `h2d.Layers`) at that index. Still, a sprite created by a mod on layer 3 or
10 was drawn under the vine it overlapped (found by the ray-crosshair mod): don't rely on layers to draw over level
objects; place markers next to them instead.


## Image files

Images are read by Heaps' `hxd.res.Image`. `getInfo` (Image.hx:126-134) reads the PNG header and accepts bit depth 8
(every color type, palettes included) and 16 (grayscale, RGB, gray+alpha, RGBA); everything else throws
`Unsupported png format <bits>/<color type>(<path>)`. At startup `editor.Editor.loadTiles` → `prepareClassTiles`
(Editor.hx:590, 795, from `Main.init` Main.hx:350) reads the pixels of every object image, mod images in
`fish/game/res/images/` included, so one 4-bit PNG stops `Main.init`: the window stays black with a grey bar, the
process keeps running (found with the item mod vector-piranha). `openlina_sdk::assets::png_problem` is this rule;
see docs/testing.md "Graphics" for what the tools do about it.

## Sounds

`EvSheet.sfx(name, loop: Ref<I32>, volume: Ref<F64>, pitch: Ref<F64>, object)` (fn@3432, EvSheet.hx:1927) plays a sound
effect: `name` is a file of `res/media/` without its extension, `volume` an offset in dB, `pitch` a random pitch spread.
The game's gun: `sfx("gungun_shot", 0, -3, 0.2, null)` (closure fn@8545, L1660). A name already played this frame is
skipped (`sfxThisFrame`). `PAudio` (the sound player behavior) lists `res/media/` (WAV and OGG) and `res/media/music`
once when it is created at startup (`dig`, closure fn@6161), keyed by file name without extension, so files a mod adds
to `assets/media/` are found like the game's. `openlina_sdk::sound` wraps this.

The game's effects (`res/media/`, 150 WAV, 5 OGG): `achievement_popup`, `achievement_popup1`, `achievement_popup2`, `achievement_popup3`, `achievement_popup4`, `achievement_popup5`, `achievement_popup6`, `activ_telebox`, `ball_inflate`, `bamboosnake`, `bamboosnake2`, `bamboosnake3`, `bird_death`, `bird_spawn`, `bomb_waitandexplode`, `boot_spawn`, `bottle_use`, `bouncer_bounce_1`, `bouncer_bounce_2`, `bouncer_bounce_3`, `bouncer_create`, `box_create`, `boxhit_1`, `boxhit_2`, `boxhit_3`, `bubble_pop`, `bubble_popspawn`, `bubble_spawn`, `cable_create`, `cannon_prep`, `cannon_shoot`, `cell_connect`, `cell_stick_ground`, `clock_destabilise`, `clock_spawn`, `clock_stabilise`, `clock_ticking`, `cocoon_ground_enter`, `cocoon_respawn`, `cocoon_spawn`, `curse_scream`, `death`, `elevator_attach`, `elevator_break`, `elevator_detach`, `elevator_hitbottom`, `elevator_loop`, `elevator_spawn`, `fanbox_power`, `fanbox_spawn`, `fanbox_windup`, `fanbox_windup_edit`, `final_level`, `fish_death`, `fish_jump`, `fish_spawn`, `floor_crack`, `fly_flap`, `fly_spawn`, `frog_death`, `frog_eat_coin`, `frog_hit_ground`, `frog_jump`, `frog_spawn`, `gear_loop`, `gear_spawn`, `general_collision_1`, `general_collision_2`, `general_collision_3`, `general_collision_4`, `general_collision_5`, `general_jump`, `general_new_weapons`, `general_shot`, `glove_fistbump`, `glove_impact`, `glove_spawn`, `grenade_explode`, `gungun_collect`, `gungun_port`, `gungun_shot`, `hammer_spawn`, `honey_spawn`, `honey_stick`, `hook_connect`, `hook_pull`, `hook_snap`, `immoveable_spawn`, `impulse_pull`, `item_suck`, `jelly_death`, `jelly_spawn`, `jelly_swim`, `ladder_grow`, `ladder_spawn`, `leg_attach`, `leg_jump`, `leg_spawn`, `level_finish`, `level_start`, `lightbox_spawn`, `menu_confirm`, `menu_item_selection_finish`, `menu_item_selection_loop`, `menu_move`, `mobilise_effect`, `mosa_lina_theme`, `nograv_create`, `phaser_passing`, `phaser_port`, `point_collect`, `portal_open_loop`, `portal_open_win`, `portbox_port`, `railway_end`, `railway_loop`, `reed_digging`, `reed_grow`, `rope_destroy`, `rune_port`, `rune_spawn`, `shifer_loop`, `shifter_spawn`, `shoot_telebox`, `spear_impact`, `springbox_button`, `springbox_reset`, `springbox_spawnjump`, `stabiliser_freeze`, `starfish_hitwall`, `starfish_port`, `stepladder_grow`, `stickbomb_armed`, `stickbomb_explode`, `stickbomb_stick`, `sunflower_grow`, `swap_item`, `tele_telebox`, `tentacle_spawn`, `turret_shot`, `turtle_rip`, `turtle_rip_loop`, `unboxer_unbox`, `unicycle_spawn`, `unicycle_wheel_moves`, `upgrav_create`, `vaporites_create_pull_freeze`, `vaporites_create_pull_freeze alternative`, `vaporites_create_pull_freeze alternative 2`, `vaporites_create_pull_freeze alternative 3`, `vaporites_create_pull_freeze alternative 4`, `vaporites_create_pull_freeze alternative 5`, `vlc_clone`, `vlc_unclone`, `weapon_switch`.
