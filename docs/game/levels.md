# Runs, levels and level packs

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

## Starting runs and levels

- Title screen: `EvSheet_first_screen_ev.update`; any key runs `levelManager.refreshPool(Main.i.packManager);
  layout.goToLayout("main")`. The title layout also runs `EvSheet_gameplay.update` for its first 2 ticks.
- A run starts with the tutorial `jeppetutorial` unless `manager.tutorial_done` is set
  (`game.ev_instancing_ev.manager.first()`, an `OClass_manager`, which also holds the run's RNG seeds).
- Loading a level (editor Play button, `Main.renderLevelPreview`): `levelManager.levelPool = [instance]`,
  `rollRaw(rand, false, directors_cut, coop)`, `ev_manager_ev.rollItemsRaw()`, `loadCurrentLevel(game)`.
- The pool after `refreshPool` holds the base game's 361 levels, named e.g. `greendemo 1`, `bluedemo 12`,
  `reddemo 3` (the harness `list_levels` option prints them all).
- Title screen → hub (layout `help`) → walking off its right edge starts a run: the MANAGER screen (layout
  `manager`) rolls the tools (see items.md, "Run start"), then the first level loads. Level layouts are named after
  the level.

## Level data

`mld.Level`: `bgColor`, `fruitType`, `disableModifiers`, `longMode`, `disabled`, `toolOverrides`, `objects`
(`mld.Object`: `type`, `families`, `x`, `y`, `rotation`, `width`, `height`, `customData`). Custom level packs
live in `userdata/levelpacks/`.

Level packs (`fish.system.PackManager`, constructor fn@4346): `loadLocalLevels` (`userdata/levelpacks/<dir>/data.mld`,
`LocalPack`) and `loadDownloadedLevels` (`userdata/downloaded/<dir>/data.mld`, `DownloadedPack`) fill
`packManager.packs`. A `Pack` has `type`, `enabled` (true by default), `hash` (identity: `LocalPack.calcHash(bytes)`,
used by pack states and to find a level's pack), `data: mld.LevelPack` (`name`, `creator`, `code`, `levels`) and
`instances`, built by `initInstances()`: one `{bgColor, dotColor, isLong, levelN, name: "<pack name> <n>", source,
sourceHash: pack.hash}` per enabled level. `LevelManager.refreshPool` collects the instances of enabled packs;
loading an instance finds its pack by `sourceHash` and calls `data.levels[levelN].createLayout(game)`.

`mld.Object.getFromType(type)` creates the right `mld.objects.*` subclass and applies the editor defaults
(`setInitParams`); `rotation` is in radians, `createInstance` turns it into `sprite.angle` degrees. Only
`Resizable` objects (zones) take `width`/`height`. The editor's new-level template is a coin at (300,100), a
portal (player entry) at (300,150) and a `tile_L` at (300,200). Tile sizes (units): `tile_long` 64×16,
`tile_medium` ~50×16, `tile_short` 32×16, `tile_tiny` 16×16, `tile_box` 32×32. Fruits (`coin`) float by default.
