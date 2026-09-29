# OpenLina plan

Multi-stage plan agreed on 2026-09-29. Update the checkboxes and "Findings" as stages complete, so a new session
can continue from here.

## Layout

```
mosa-mod/                 (plain folder)
  openlina-kit/           git repo: SDK, CLIs, core patch, harness, showcase mods, docs, agent skill
  openlina-web/           git repo: the OpenLina website (mod hub)
```

## Decisions

- **Distribution: client-side patching.** Zips never contain a patched `hlboot.dat` (it's the developer's
  copyrighted game, it breaks on updates, and combinations explode). A zip holds mod packages + the `openlina`
  helper, which patches the user's own local bytecode.
- **Mod package** = `mod.toml` (id, category, version, compatible game builds, deps/order, options schema,
  item stats) + `patch.wasm` (the mod's Rust `apply()` compiled to wasm32-wasip1; bytecode in → bytecode out,
  sandboxed: no fs/net) + `assets/` + `media/` (icon, gifs).
- **OpenLina core** patch is applied first in every pack: named hook points (level start, tick, object left
  screen, shoot, …) + item and modifier registries, declared load order, conflict detection.
- **Website:** Rust (axum) + SQLite, single binary. Review on localhost; production on rhost, VPS if too slow.
  Four sections: Items, Modifiers, Level mods, General. Cart with per-mod/per-section change requests. Export
  zip (helper + packages + `modpack.toml`) or JSON (ids, versions, URLs, change requests) for agents.
  Voting: one up/down per mod per salted IP hash (no raw IPs). REST API + OpenAPI. Agents use `lina pull/publish`
  (CLI + skill); MCP optional later. Uploads with per-user tokens into a review queue.
- **Testing without synthetic input** (xdotool doesn't reach the game): a harness patch boots straight into a
  level, gives items, picks modifiers, replays scripted inputs through `PlayerInputs`, emits assertion trace
  lines, captures frames via Heaps → `lina gif`. Run under Xvfb where possible.
- **Tools installed via nix** (flake devShells per repo).
- **Windows:** the Windows build also ships the JIT + `hlboot.dat`; test via Proton later.

## Showcase mods

| Section | Mod |
|---|---|
| Items | Portal gun: two portals on surfaces, objects/player pass through with momentum |
| Modifiers | Screen wrap as a real modifier id in the game's modifier pool, with display/icon |
| General | Mod menu: "OpenLina" entry in the pause menu listing active mods and options |
| Level | "Tumble": gravity rotates 90° every 10 s, camera rotates along, geometry the editor can't place |

## Stages

- [x] 0. Spikes: asset overlay + frame data, direct level loading, input injection, frame capture, item/`shoot`/
      modifier plumbing
- [x] 1. Website design prototype (Claude Design, Mosa Lina style, placeholder mods), approved by the user
- [x] 2. Kit foundation: wasm package format, `openlina` helper, core hooks/registries, screen-wrap ported
- [x] 3. Harness + `lina test` / `lina gif` / `lina sprite`
- [ ] 4. Showcase mods: modifier → general → item → level, each with icon, gifs, tests
- [ ] 5. Website implementation: API, voting, zip/JSON export, upload tokens, review queue
- [ ] 6. Agent integration: `lina pull/publish`, skill, change-request workflow, docs polish

## Findings

(Filled in as stages complete. Details in game-internals.md.)

- Design prototype: https://claude.ai/artifact/SeqU5XWxttwHCJwjgDh6Uy (copy in openlina-web/design/), approved.

- Items are created in code (`EvSheet_instancing_ev`, ~L100–278) as `OClass_item` with `NAME`, `ammo`,
  `aim_type`, `unlocked`, `secondLayer`, then `ItemManager.initBaseItems`. Behavior is dispatched in
  `EvSheet_gameplay.shoot` (fn@3739).
- Modifiers are integer ids drawn per level from hardcoded pools in `LevelManager` (`[1..7]`, `[1,2,9]` when
  `dx`) into `currentLevel.modifier` / `coopModifier`.
- **Assets:** sprite frames are defined in code: each `OClass_*` builds
  `FrameData("images/<sheet>.png", x, y, w, h, rotated, duration, originX, originY, …)`. New sprites = PNG +
  injected `FrameData`. The resource root is the string `"./fish/game/res"` (cwd-relative), and userdata is
  `Main.dir()` = cwd + `/userdata` (except on Mac). So the helper runs the JIT from an **overlay directory**:
  symlinks to the install, merged `fish/game/res` with mod assets, `userdata` symlinked (shared saves). The
  install is never modified.
- **Level boot** (from `editor.buttons.Play`): `levelManager.levelPool = [instance]`,
  `levelManager.rollRaw(seed, false, directors_cut, coop)`, `ev_manager_ev.rollItemsRaw()`,
  `levelManager.loadCurrentLevel(game)`. `instance = {bgColor, dotColor, isLong, levelN, name, source, sourceHash}`.
- **Inputs:** `PlayerInputs` holds 9 action bits per frame (`toBin`/`readBin`; negative = command). The game
  has a random-input mode (`Main.randomInputs` → `updateRandom`).
- **Replays:** `fish.system.Replay` records and plays per-frame inputs (`record`, `play`, `save`, `load`,
  `writeBin`/`readBin`); `StateSerializer` saves/restores level state.
- **GIFs:** `Main.saveGif` re-simulates `Main.gifState {loadLayout, replay, state}` and renders every frame at
  600×338 into a GIF (native `gif_create` in fish.hdll, own thread), hiding score/HUD. `renderGifFrame`
  shows how to render one frame to a texture (`capturePixels`). `Main.renderLevelPreview(pack, n)` → `hxd.Pixels`
  (`.toPNG()`) for level thumbnails.
- **Modifier display:** a sprite whose `animFrame` = modifier id (id 9 → frame 7), in
  `EvSheet_edge_ev.update` ~L471. A new modifier needs an extra frame in that animation. Effects are checked
  in `EvSheet_gameplay.update` (e.g. L16093 `modifier == 1`) and on level load; object types `mod_*`
  (`mod_random_block`, `mod_no_portal`, `mod_below_protec`, …) implement some modifiers.
- **Stage 2 (done):** mods are separate crates (`mods/<id>`), run as stdin/stdout programs natively or as
  `wasm32-wasip1` in wasmtime (~0.3 s per mod including compilation). `core` provides the `tick` and `edge_exit`
  hooks; screen-wrap is an `edge_exit` subscriber. `openlina` installs packs into `~/.local/share/openlina` and
  runs the game from an overlay dir. Verified end to end: pack zip → `openlina install` → build (wasm) → game runs.
- **Headless runs work without Xvfb:** `SDL_VIDEODRIVER=offscreen` (EGL via the system's Mesa), with `DISPLAY`
  and `WAYLAND_DISPLAY` unset. nixpkgs' Xvfb had no GLX visuals, so Xvfb is not used.
- **Deterministic physics:** the same build and fixtures reproduce identical ticks/coordinates (box spawned by
  debug-spawn first wraps at tick 418 from y=345.99 in every run), so tests can assert on trace output.
- **Title screen:** a key press runs `levelManager.refreshPool(Main.i.packManager); layout.goToLayout("main")`
  in `EvSheet_first_screen_ev.update` (the `harness` mod does this without input).
- **Stage 3 (done):** `harness` mod (title skip, level/seed/items/modifier selection, scripted inputs through
  `readBin`, frame capture via the game renderer, exit at a tick), scenario files `mods/<id>/tests/*.toml`,
  `lina test` (headless, expectations on the log), `lina gif` (ImageMagick + gifsicle from the nix shell),
  `lina sprite` (text grids → PNG, game palette). screen-wrap has 3 tests (box wraps; fruits vanilla by default;
  fruits wrap with `coins = true`) and `media/box-wraps.gif`, `media/icon.png`. A scenario run takes 7-10 s.
