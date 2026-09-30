# Debugging a mod: hangs, crashes, wrong behavior

The loop: **reproduce → look → explain → fix → keep a test**. Each step has a tool; none needs a mod of your own.

## 1. Reproduce as a scenario

Turn the report into a scenario that fails, before reading any code. It pins the bug down, and it becomes the
regression test.

- Something in a level: `level`, `items`, `debug-spawn`, `inputs` (see `docs/testing.md`).
- Something in real play (run start, tool selection, menus, several levels): `new_run = true` with `inputs`, and
  `new_run_items` to force the item into the roll. For "sometimes": `chaos = true` + `end_total` and a few seeds.
- The whole pack: `tests/all-mods-soak.toml` plays new runs with all showcase mods and random input.
- From a player: ask for `openlina report` (a zip: installed mods with versions and options, game version, the
  last modded runs' output, the game's `crash_stackdump.txt`). Build the same pack and look for the crash there.
- From your own play: `lina run --record` writes one replay scenario per level attempt (`work/recordings/`: level,
  modifier, tool slots, inputs). Replays are exact for harness runs; a human session replays as long as the level
  has no random elements.

Read the failure report: a crash names the exception and stack (`Called from … (openlina/<fn> line <op>)` points
into injected code: `lina fn <fn> --input work/test/<n>/game/hlboot.dat --ops <op-5>..<op+5>`). A timeout names
the last screen seen (`last seen: layout manager tick 1800`): the game is stuck or waiting there. Compare with the
same scenario without the mod (`mods = []`): "waiting for input" is normal on some screens.

## 2. Look at the running game

- `lina probe`: any value at a moment, e.g. what the tool selection rolled:
  `lina probe --mod swap --new-run --input 30-700:right --at layout:manager@200 "game.itemManager.pickedItems[].type.name"`.
  Add `--capture 600-1000/20` for a contact sheet of the screen.
- `trace-calls` (fixture): which functions run, how often and **with which arguments**:
  `functions = ["EvSheet_manager_ev.final_ani_block"]` printed `(…, unlocked_item, swap, _)`, the call that
  never finished.
- `lina class <Class>`: fields and methods of what you are looking at.

## 3. Explain it in the code

- `lina refs <field|string>`: every function that reads/writes the field or uses the string (the game keeps its
  strings in globals; `lina strings` misses those). E.g. `lina refs unlocked_item`.
- `lina fn <Class.method> [--hx]`, `lina callers <fn>`; closures are `fn@N <anonymous>` with their source line.
- Look for the place the game **lists** something (a loop over a picker or pool, a lookup by name). Mods usually
  break there: the thing they add is missing from one of those lists. `docs/sdk.md` keeps a checklist per
  registry; add what you found.

## 4. Fix, preferably in the SDK

If the cause is a missing piece of a registry (items, modifiers, levels), fix the registry, not the one mod: every
mod using it gets the fix. Bump the versions of the mods that change.

## 5. Keep the test

The scenario from step 1 goes into `mods/<id>/tests/` (or `tests/` for several mods). Check it fails without the
fix. Run `lina test` (all) and `lina test --wasm` before committing. Write what you learned about the game into
the topic file in `docs/game/` (index: `docs/game-internals.md`).

## Case study: the frozen tool selection

Report: "starting a new run freezes on the tool selection screen; tools can't be picked in the editor". Items
rolled into levels worked.

1. Reproduce: `new_run` + walking off the hub reached the MANAGER screen; `new_run_items = ["swap"]` made it hang
   every time (without it, only when the roll drew swap).
2. Look: `trace-calls` on `final_ani_block` showed an "unlocked_item" animation block for `swap`; frames showed the
   grid never lit up. The same screen without mods continued.
3. Explain: `lina refs unlocked_item` → the block's handler looks up the `OClass_item` object whose `NAME` is the
   tool; `lina probe "@item[].NAME"` showed 48 objects, none for mod items. The editor's tool list
   (`editor.ItemOverride.init`) is built from the same objects. A probe of the icon animation showed vanilla icons
   have two frames and the grid shows frame 1.
4. Fix in `items::register`: a global item object per mod item (a fifth grid row) and a large icon frame.
5. Test: `mods/swap/tests/run-start-selection.toml`; the soak test covers all items together.
