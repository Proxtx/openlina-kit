# Modifiers

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

- `LevelManager.rollRaw(rng, hasModifier, dx, coop)` (the decompiler shows the parameter names shifted by one) and
  `reroll` draw `currentLevel.modifier` from `[1,2,3,4,5,6,9]`, or `[1,2,9]` in "dx" runs; co-op draws
  `coopModifier` from `[1,2]`. Single levels (editor Play, level previews) pass `hasModifier = false` (modifier 0);
  in runs, `reroll` gives a modifier to a quarter of the levels.
- HUD: `OClass_optionthingos` icons (music, sound, record, "cut", "mod"); `EvSheet_edge_ev.update` sets the "mod"
  icon's `animFrame` to the modifier (9 → frame 7). Animations live in the static
  `$OClass_optionthingos._animData` (`StringMap` name → `Anim`), frames are `FrameData` with an image url.
- `openlina_sdk::modifiers` adds modifiers through the core hooks `modifier_pool` / `modifier_icon`, with ids
  `100 + fnv1a(key) % 9000` and their own icon animation.
