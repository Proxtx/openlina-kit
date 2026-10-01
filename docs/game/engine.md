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

