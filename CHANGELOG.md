# Changelog

openlina-kit has one version for the SDK, `lina`, `openlina` and the `core` mod: the workspace version in
`Cargo.toml` (`openlina_sdk::kit::KIT_VERSION`). Every mod's `mod.toml` names the kit it was made with (`kit`;
`lina new` writes it, `lina pack` raises it to the kit that built the package; none means 0.1.0).

**Versioning.** `major.minor.patch`; a *line* is `0.minor` (later: `major`).

- **patch** (0.1.0 → 0.1.1): additions mods may use (a helper, a hook, a harness option) and fixes. Mods made
  with an older kit of the line keep working; mods made with the newer one need a kit (or `openlina`) at least
  that new, so `lina` asks for `git pull` and `openlina` for an update.
- **minor** (0.1.x → 0.2.0): something mods rely on changed (SDK API, core hook signatures, the runner's
  interface, conventions between mods). Mods made for an older line are refused by `openlina` and `lina` until an
  agent ports them; the website marks them NEEDS AN AGENT and refuses players' zips of packs that hold them.
  Each such release lists below, under **Porting**, what to change.

Maintainers: bump the version in the commit that changes something mods can notice, add an entry here, tag
`v<version>` and push the tag; the release workflow builds `openlina` and the skill zip. Then update the website
(rebuild it against the new kit, replace its helpers) before uploading mods made with the new kit.

## Unreleased

- **Mods**:
  - infinite-ammo 0.1.0 (general, new): items never run out (the game's ammo use and refunds are skipped).
  - solid-edges 0.4.0: `player` makes the border solid for Lina too (in levels; the hub's right edge stays open);
    frogs always bounce now (they were left to vanilla and lost at the border).
  - mod-menu 0.2.0: everything stays on screen: each mod opens a page with its options, at most 10 lines a page
    (MORE opens the next), long lines cut, characters the menu font lacks become spaces.
- **Harness**: `menu_open = "A > B"` presses nested menu items one after the other.

## 0.1.3 (2026-10-02)

- **SDK**: `joints` (joined objects, bodies held by Box2D joints: `collect` the whole group from one piece, or jump
  to `held` when a static body or Lina holds it; `shift`, `centre`, `member`, `owner_of`).
- **Mods**:
  - ray-crosshair 0.3.0 extends the game's own reticle instead of drawing a marker: for the items that ask, it sits
    on the first tile, wall or object along the aim within the item's range (vanilla: level geometry within about
    100 units), or at the end of the range, kept below the HUD bar. In real play the game showed its hidden reticle
    again, so two crosshairs appeared; now there is only the game's. `hide_reticle` is gone; the hook and
    `aim::show_crosshair` are unchanged.
  - swap 0.5.0 and portal-gun 0.5.0 move joined objects (step ladders, bamboo, unicycles…) as a whole: Swap trades
    places with the whole group (its centre to Lina's spot), a portal sends the whole group through, placed in front
    of the exit.
  - screen-wrap 0.5.1: joined objects wrap once the whole group is off the screen and slide in at the opposite edge
    leading end first (they wrapped by their centre and reappeared well inside the screen; a long bamboo popped up
    whole); uses `joints`. A fruit collected by wrapping looks and floats like a touched one (the darker model,
    `_collected`).
  - solid-edges 0.3.0: frictionless along the border again (`friction` default 1; 0.9 since 0.2.0).
  - cannons 0.2.0: the cannons drop in one after another (`stagger`, 45 ticks) and start hidden behind the HUD bar
    (`y` 4), falling into view instead of popping up in the play field.
- `openlina install a.zip b.zip`: a mod that requires one from a later zip of the same command installs (it was
  refused unless the required mod came first).
- **Docs**: where the game puts its reticle (an aim point and a ray from the `gun`, docs/game/input.md).

## 0.1.2 (2026-10-01)

- **SDK**: `sound` (`play`: the game's sound effects by name, or a mod's own from `assets/media/`; unknown names are
  skipped instead of crashing the game).
- **Core 0.6.0**: `edge_exit` kind 3, frogs (vanilla raised `frogland_count` already).
- **Mods**: screen-wrap 0.5.0 wraps fruits by default (a fruit pushed off the screen is still collected, then
  wraps), frogs, and joined objects (step ladders, bamboo, tentacles…) as a whole group; swap 0.4.0 swaps with level
  geometry too (`walls = "swap" | "block" | "pass"` replaces `walls_block`) and plays sounds; portal-gun 0.4.0 plays
  the game's gun sounds; ray-crosshair 0.2.0 hides the game's reticle while its marker shows (`hide_reticle`);
  solid-edges 0.2.2 (corrected fruit description); moon-gravity 0.1.1. Icons redrawn in the vanilla style (solid
  shapes, 2-3 tones, every size drawn natively).
- **Fixtures**: debug-spawn `pushes` (give objects of a type a velocity at a tick).
- **Docs**: how fruits are collected (touching, or deleting: pushing off the screen, the delete tool), sounds, the
  icon style.

## 0.1.1 (2026-10-01)

- **SDK**: `aim` (`AimRay`: the ray from Lina through the reticle to the next surface, with the hit point and
  `before`; `shooter`; `show_crosshair`), number hooks (`F64`, negative = not mine), `RayCast::fraction`,
  `world::spawn_on` (a layer).
- **Mods**: ray-crosshair 0.1.0 (general, a utility: the marker where the aim meets the next surface, for the items
  that ask); swap 0.3.0 and portal-gun 0.3.0 use it. Portals now land on the next surface along the aim (range 400,
  only level geometry; `mid_air` for the old placement at the end of the range).
- Scenarios may set options for mods that come in as requirements; portal-gun's and ray-crosshair's traces print
  `[pos]` lines for `position` expectations.
- `openlina install` records the installed version of reinstalled mods in its state (it kept the old one).
- **Images the game can't load** (1-, 2- or 4-bit PNGs in `assets/`: the game freezes on a black screen while
  starting): `openlina_sdk::assets` has the game's rule; `lina pack` and the website refuse such files with the fix,
  `lina build` and players' `openlina` convert them to 8-bit RGBA in the overlay (packages already on a site keep
  working).

## 0.1.0 (2026-10-01)

The first versioned release. Mods without `kit` in `mod.toml` (everything published before) count as 0.1.0 and
work with it.

What the kit can do: patch Mosa Lina's HashLink bytecode from Rust mods that run natively during development and
sandboxed as wasm for players; core hooks (`tick`, `edge_exit`, `player_edge`, `modifier_pool`, `modifier_icon`,
`item_pool`, `item_use`, `loc`, `packs`) and registries for items, modifiers, level packs and texts; headless
scenario tests in the real game (deterministic, parallel), showcase gifs from the game's renderer, pixel art from
text grids; `lina probe`, `refs`, `class`, `fn`, `callers`, the decompiled dump, recordings that replay as tests;
the players' `openlina` (install packs, Steam launch wrapper, capability checks, `report`); `lina pull` /
`publish` for the OpenLina website.

New in this release:

- **Kit versions**: `kit` in `mod.toml`, `openlina_sdk::kit` (versions, lines, `issue`/`issues`), checks in
  `lina build`/`test`/`pack`/`pull` (newer kit: `git pull`; older line: port it), in `openlina install`/`build`
  (newer: update openlina; older line: give the pack to an agent) and on the website. `lina --version`,
  `openlina --version`; `lina doctor` says whether the checkout is behind GitHub.
- **Declared option conflicts**: `[[conflict]]` in `mod.toml` (`with`, `options`, `with_options`, `reason`),
  checked by every build, `openlina set`, the website and `lina pull` (a task in REQUESTS.md). solid-edges 0.2.1
  declares its clash with screen-wrap there instead of in code.
- **Core 0.5.0**: the `player_edge(pos, margin, sheet, player) -> Bool` hook (Lina leaving the screen; return true
  to keep her alive). screen-wrap 0.4.0 uses it for its new `player` option.
- **SDK**: `world::{is_level, jump_unless_level, player_pos, spawn, count, NOT_LEVELS}`, `FnBuilder::random/abs`,
  `edit::is_field/is_set_field`; `Code::find_fn` finds static methods without the `$` (`Math.random`).
- **Harness**: `Math.random`/`Std.random` are seeded from `seed`, so scenarios with random placement replay.
  A scenario whose level ends before `end_tick` now says so instead of a bare timeout.
- **Tests**: `{version:<id>}` in expectations (a test no longer breaks when another mod is bumped); option type
  errors name the default as an example.
- **lina pull**: `--force` replaces only mods whose version differs, `--replace <id>` one mod even at the same
  version; same version with different files is reported.
- **openlina install**: prints what is installed afterwards and marks mods that aren't in the pack;
  `--replace` makes the installed mods exactly the pack's.
- **./lina** runs itself inside `nix develop` when cargo or the wasm target is missing and nix is there.
- **Mods**: cannons 0.1.0 (general: vanilla cannons in every level), screen-wrap 0.4.0 (`player`).
- Releases on GitHub: the players' `openlina` helper (Linux x86_64) and the skill as a zip.
