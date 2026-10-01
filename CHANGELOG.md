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

- `openlina install` records the installed version of reinstalled mods in its state (it kept the old one).

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
