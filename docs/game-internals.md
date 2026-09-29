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

## Gameplay

- The main per-tick logic is `EvSheet_gameplay.update` (fn@3767, ~22k ops, source `EvSheet_gameplay.hx`
  L9734–16300). hlbc's decompiler can't handle it. Read it with `lina fn 3767 --ops a..b` or
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

## Timing

The game steps at a fixed `Main.frameTime` = 1/120 s: **120 ticks per second**. `Layout.currentTick` counts ticks
since the layout started. Runs are deterministic: the same bytecode, level, seed and inputs give the same ticks and
coordinates.

## Starting levels and items

- Title screen: `EvSheet_first_screen_ev.update`; any key runs `levelManager.refreshPool(Main.i.packManager);
  layout.goToLayout("main")`. The title layout also runs `EvSheet_gameplay.update` for its first 2 ticks.
- A run starts with the tutorial `jeppetutorial` unless `manager.tutorial_done` is set
  (`game.ev_instancing_ev.manager.first()`, an `OClass_manager`, which also holds the run's RNG seeds).
- Loading a level (editor Play button, `Main.renderLevelPreview`): `levelManager.levelPool = [instance]`,
  `rollRaw(rand, false, directors_cut, coop)`, `ev_manager_ev.rollItemsRaw()`, `loadCurrentLevel(game)`.
- The pool after `refreshPool` holds the base game's 361 levels, named e.g. `greendemo 1`, `bluedemo 12`,
  `reddemo 3` (the harness `list_levels` option prints them all).
- Items: `itemManager.itemPool` holds 48 item types `{aimType, baseAmmo, name, secondLayer}` (`box`, `bomb`,
  `phaser`, `rocket`, …); `itemManager.currentItems[k]` is `{locked, type, wins}` and is shared with the HUD slot
  `ev_manager_ev.b_item.insts[k].item` (`OClass_b_item`, which also has `ammo`).
- **Ammo** (found by the ammo-boost docs test): the game copies ammo with the two ops
  `Field r = type.baseAmmo; SetField holder.ammo = r` in 11 places: `EvSheet_manager_ev.manage` and `rollItemsRaw`
  (HUD slots), 3 `EvSheet_instancing_ev` closures, item blocks (`EvSheet_gameplay` closure ~L6147,
  `mld.objects.ItemBlock.createInstance`) and `StateSerializer.fromBin`. Shooting does `ammo - 1`, returning an item
  `ammo + 1`; taking an item block swaps ammo between the slot and the block (closure ~L17390).
  `OClass_test_item` objects copy their own ammo into slot 0 (title screen), and the `main` layout's preset slots
  get theirs from layout data. The HUD shows ammo as `ammoSprite.animFrame = ammo + 1`.
- **Firing** (`EvSheet_gameplay.shoot(playerId)`) runs a closure over the item slots (`foreach(b_item, …)`, source
  L1140-3720): it finds the player's selected slot, does `if (ammo > 0) ammo--`, then dispatches on the item name
  (`if (slot.item.type.name == "unbox") … else if …`, comparing against the game's string constants). Its
  environment enum holds the sheet, the player picker and the player's aim pickers (crosshair_point, gun, …,
  max_aim_point, mid_aim_point, short_aim_point). The crosshair is the reticle just in front of Lina.
- Item types come from `OClass_item` objects (`NAME`, `ammo`, `aim_type`) via `ItemManager.initBaseItems`; aim
  types: shoot 0, short 1, short2 2, mid 3, mid2 4, long 5, long2 6, remote 7. The HUD icon is the `item_icon`
  object's animation named after the item; the HUD label is the text `TOOL_<NAME>` (from `loc.dat`, an xlsx).
- The roll draws 3 items and the 4th slot copies the 2nd (`rollItemsRaw` crashes if the pool has fewer than 2).
- Randomness: the run's RNGs are `hxd.Rand` fields of the manager (`mainSeed`, `levelSeed`, `toolSeed`, …), seeded
  randomly at startup; the harness seeds them all from its `seed` option.
- `Picker.insts` is typed `hl.types.ArrayDyn` but holds an `hl.types.ArrayObj`; cast before indexing (the SDK's
  `array_get`/`array_len` do).

## Rendering a frame

`Main.renderGifFrame` shows how: `engine.resize(600, 338); game.onResize(600, 338);
engine.pushTarget(Main.i.gifTarget); game.render(engine, false); engine.popTarget()`, then
`gifTarget.capturePixels().toPNG()`, and resize back. Works headless (SDL offscreen driver). The harness does
this for `lina gif`.

## Modifiers

- `LevelManager.rollRaw(rng, hasModifier, dx, coop)` (the decompiler shows the parameter names shifted by one) and
  `reroll` draw `currentLevel.modifier` from `[1,2,3,4,5,6,9]`, or `[1,2,9]` in "dx" runs; co-op draws
  `coopModifier` from `[1,2]`. Single levels (editor Play, level previews) pass `hasModifier = false` (modifier 0);
  in runs, `reroll` gives a modifier to a quarter of the levels.
- HUD: `OClass_optionthingos` icons (music, sound, record, "cut", "mod"); `EvSheet_edge_ev.update` sets the "mod"
  icon's `animFrame` to the modifier (9 → frame 7). Animations live in the static
  `$OClass_optionthingos._animData` (`StringMap` name → `Anim`), frames are `FrameData` with an image url.
- `openlina_sdk::modifiers` adds modifiers through the core hooks `modifier_pool` / `modifier_icon`, with ids
  `100 + fnv1a(key) % 9000` and their own icon animation.

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

Per frame, `PlayerInputs.updateSP` (called from `Main.mainLoop`) reads keyboard/gamepad into 9 action slots:
`inputs[i] = frame` while action `i` is held (`isDown(i)` = `inputs[i] == frame`). Bits for `toBin`/`readBin`
(replays): 0 up, 1 down, 2 left, 3 right, 4 jump, 5 shoot, 6 switch, 7 restart, 8 pause. `readBin(bits)` advances
the frame and sets the held actions, which is how the harness plays scripted inputs.

`xdotool` (XSendEvent) does not reach the game; use the harness.

## Level data

`mld.Level`: `bgColor`, `fruitType`, `disableModifiers`, `longMode`, `disabled`, `toolOverrides`, `objects`
(`mld.Object`: `type`, `families`, `x`, `y`, `rotation`, `width`, `height`, `customData`). Custom level packs
live in `userdata/levelpacks/`.
