# Mosa Lina internals

What we know about the game code, gathered while writing the first mods. Written against Steam build
22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe sources listed in the
debug info; findexes (`fn@N`) refer to this build.

## Builds and runtime

- Haxe + Heaps + HashLink. `hlboot.dat` is HashLink bytecode version 4: 16388 functions, 15613 types,
  866 natives, with debug info (file/line per op).
- `Mosa Lina_jit <file>` runs any bytecode file. It must run with the game directory as the working directory
  (`fish/game/res`, `userdata/`) and with the game directory on `LD_LIBRARY_PATH` (`libturbojpeg.so.0`).
- The game's stdout carries `trace()` output and, on a crash, a HashLink stack trace with file:line. Injected
  functions show up as `mosa/<name>:<op index + 1>`.
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

## Gameplay

- The main per-tick logic is `EvSheet_gameplay.update` (fn@3767, ~22k ops, source `EvSheet_gameplay.hx`
  L9734–16300). hlbc's decompiler can't handle it. Read it with `mosa fn 3767 --ops a..b` or
  `work/dump/asm/fish/game/evsheet/EvSheet_gameplay.asm`.
- On the title screen the gameplay sheet only runs for its first ticks (startup cleanup). It runs every
  tick in levels.
- **Coins are the fruits.** Pushing fruits out of the screen is how you win a level (`coinedgecheck`, fn@3751,
  collects them), and `coin.glitched` coins respawn at `spawnPos` until every player has touched them.
- The player loses at the screen edge (`player_death`, fn@3746). Items come from `ItemManager` and
  `EvSheet_gameplay.shoot` (fn@3739).

### Screen and edges

The play field is **600 × 338** layout units, with a black border of **25 units** (`margin`) around it. At 1080p
the scale is 3.2 (80 px of border).

The edge test in `update` (L16195–16268):

```haxe
var margin = 25;
for (p in player.insts)                                  // player_death if outside [25,575]x[25,313]
for (o in physics_obj.insts) {                           // L16219
    if (o.sprite.destroyed || (isStatic(o.physics.body) && layout.currentTick != 0)) continue;
    var pos = o.sprite.position, ew = o.edgewith;        // per-object slack
    if (pos.y > 338 - margin + ew || pos.y < margin - ew
        || (!bossMode && (pos.x < margin - ew || pos.x > 600 - margin + ew)))
        switch (o.type) {
            case "coin":   coinedgecheck(o);             // win condition
            case "frog":   frogland_count++; destroy;
            case "player": // handled above
            default:       o.sprite.destroy();
        }
}
for (o in secondary_physics.insts)                       // same test, no edgewith, always destroy
```

- `bossMode = currentLevel.type.isLong` when a level starts (closure fn@9188). It is also set by `prepare_boss`.
  It's true for boss arenas and for custom levels with `mld.Level.longMode`: levels wider than the screen that
  scroll horizontally. Those levels skip the left/right test.
- The game **parks objects far off-screen** (e.g. at −1000, −1000) and places objects just below the screen in
  level data, relying on the edge test to delete them (first ticks of a layout). Mods that change edge
  behavior need to leave those alone.

## Input

Default keyboard map (`KeymapManager.defaultKeymap`, closure fn@17238; SDL scancodes → actions):

| keys | action |
|---|---|
| arrows / WASD | move, aim |
| Z, Y, J, Space | jump |
| X, E, K | shoot (use item) |
| C, Q, L, Shift (+ codes 256/257, probably mouse buttons) | switch item |
| R | retry |
| Return / Esc | confirm / back |

`xdotool` with `--window` (XSendEvent) does not reach the game, so input can't be automated that way. Test
from inside the game instead (see `debug-spawn` and `trace-calls`).

## Level data

`mld.Level`: `bgColor`, `fruitType`, `disableModifiers`, `longMode`, `disabled`, `toolOverrides`, `objects`
(`mld.Object`: `type`, `families`, `x`, `y`, `rotation`, `width`, `height`, `customData`). Custom level packs
live in `userdata/levelpacks/`.
