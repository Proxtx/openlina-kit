# Testing mods

Everything runs headless (SDL offscreen driver): no window, no human. The harness runs 16 extra game steps per
rendered frame (`turbo`), and `lina test` runs half the cores' worth of scenarios at once: the whole suite takes
about a minute and a half. Steps keep their fixed length, so results are the same as in real time.

## Scenarios

A scenario is a scripted game run with expectations on its log, in `mods/<id>/tests/*.toml` (scenarios across
several mods: `tests/*.toml`). `lina new` creates `tests/smoke.toml`; the format is documented in
`tools/lina/src/scenario.rs`.

```toml
name = "screen-wrap: a falling box wraps from bottom to top"
mods = ["screen-wrap"]            # mods under test; core, harness and requirements are added
fixtures = ["debug-spawn"]        # dev mods that set the scene or print things
timeout = 60                      # wall-clock seconds

[options.screen-wrap]
trace = true

[options.debug-spawn]
object = "box"
tick = 30

[harness]
level = "greendemo 1"
modifier_key = "screen-wrap"
end_tick = 600

[[expect]]
contains = "[screen-wrap] tick"
```

- `[harness]` (all options: `docs/mods.md`, "harness"): `level` (names: run a scenario with `list_levels = true`),
  `seed`, `items` (a temporary pool of just these items, rolled by the game's own code), `modifier` /
  `modifier_key`, `inputs` (`"60-90:right+jump"`: up down left right jump shoot switch restart), `end_tick`,
  `capture`.
  - `inputs` and `end_tick` count ticks from the start of each layout, and inputs replay on every retry of a
    level. `restart` only restarts the editor's level preview (both `restartLayout` calls in
    `EvSheet_gameplay.update` check `isPreview`); in a run, a retry goes through a death and the tool selection
    (`manager`), where level ticks stop: a scenario that dies before `end_tick` times out with that reason. Use
    `end_total` (game steps on any screen) for those.
  - `new_run = true` plays the game's own run start instead of loading a level: the hub, then the MANAGER tool
    selection (`inputs = ["30-700:right", "1000-1010:jump"]` walk off the hub and confirm). `new_run_items`
    forces items into its roll. See `mods/swap/tests/run-start-selection.toml`.
  - `chaos = true`: seeded random input on every screen but the title (when no scripted input is active);
    `end_total = N` ends after N game steps on any screen. For soak runs, see `tests/all-mods-soak.toml`.
  - `roll_until_modifier = "<key>"`: the game's own roll, repeated until it draws the modifier (a test that it is in
    the pool, independent of which seed draws it; that changes with every modifier mod added).
  - `slots = ["box", "swap:2"]`: exactly these tools (optional ammo) at level tick 1; replays use it.
- Fixtures: `debug-spawn` (objects at a tick: `object`/`tick`/`x`/`y`, or `spawns = ["box@60:300,60", …]`),
  `trace-positions` (`[pos] tick T <type> x y` for `types` at `ticks`, a string like `"40-1200/40"` or `"11,300"`), `trace-calls` (calls to any function,
  with their arguments), `inspect` (any part of the game's state at a moment, see `lina probe`), or a dev mod of
  your own subscribing to `tick`.
- `[[expect]]`: `contains` (+ `min`/`max`), `not_contains`, `position = { tick, type, x, y, within, away }` on
  `trace-positions` output; bounds instead of (or with) x/y: `position = { tick = 200, type = "box", y_lt = 110 }`
  ("still above 110", y grows downwards; also `y_gt`, `x_lt`, `x_gt`). Measure vanilla once and bound against it.
  `away = true` inverts: with x/y, no object near that point; with only bounds, no object inside them.
  `{version:<id>}` in `contains`/`not_contains` stands for that mod's version (mod-menu lists other mods'
  versions without breaking when they are bumped).
  An object shows up in `trace-positions` from the tick after it was created.
- Every run also fails on a crash, a logged Haxe exception (`Null access`, `Called from …`), a `[harness] ERROR`,
  a non-zero exit or the timeout.
- Aiming and jumping: `docs/game/input.md` (hold the direction on the shoot tick; jump fires on the press, so
  press it again once Lina stands).
- A scenario with `level` first starts the run's first level (the tutorial; its gameplay ticks run too, so a mod's
  tick-1 trace can appear for `jeppetutorial`), then loads the chosen level on its tick 2.

Rules:

- **Make every test able to fail.** Assert that the situation happened (e.g. `trace-calls` shows vanilla's
  function ran), not just that nothing bad was printed; test options in both directions. Check that a new test
  fails without your fix.
- Runs are deterministic: the harness seeds every RNG from `seed` (the game's `Rand` rolls, and `Math.random` /
  `Std.random`, which mods may use), so the same build and scenario give the same items, ticks and coordinates. It prints `[harness] slot k: <item> ammo <n>` at level tick 1 and
  `[harness] layout <name> tick <t>` when the screen changes and every 1200 ticks (the tick of these lines varies
  a little between runs; everything else doesn't).
- Scenario files can live anywhere (`lina test path/to/x.toml`), e.g. throwaway probes. `lina probe` writes one
  (`work/probe.toml`); `--harness key=value` and `--set mod.key=value` set any option there.
- Recordings: `lina run --record` (you play) or the fixture `record` in any run, then `lina recordings <log>`:
  one replay scenario per level attempt in `work/recordings/`.

## Running

- `lina test` (all), `--mod <id>`, `-k <text>` (name or file contains), `--failed` (the failures of the last
  run), `--wasm` (what players run; do it before publishing), `-j <n>` (parallel runs).
- Logs: `work/test/<n>/log.txt`, build lines first. A failure report names the reason (a timeout names the last
  screen seen), shows the log's last lines and keeps the patched game in `work/test/<n>/game/`
  (`lina fn … --input work/test/<n>/game/hlboot.dat`).
- Captured frames: `work/test/<n>/frames/`, plus one contact sheet `work/test/<n>/frames.png` to look at.

## Showcase gifs

Add a `[gif]` section (`capture = "from-to/step"` in level ticks, 120 per second; `out = "media/x.gif"`) and run
`lina gif mods/<id>/tests/<scenario>.toml` (`--out <file>` writes elsewhere). The scenario's expectations are
checked too. Frames come from the game's own renderer at 600×338; look at them before publishing
(`work/gif/frames/*.png`). `[mod] showcase = ["best.gif", …]` orders the gifs on the website.

## Graphics

Pixel art as text grids in `mods/<id>/art/*.toml`: `lina sprite mods/<id>/art/x.toml --out
mods/<id>/assets/images/x.png` (game sprites; paths relative to where you run it) or `--out
mods/<id>/media/icon.png` (website icon). `lina sprite --palette` lists the game palette; `--scale 16` writes a
preview, `--scale 2` a large item icon.

## The player's side

`lina pack <id>` writes `dist/<id>-<version>.zip`. To try the player flow, bundle it (requirements are added) and
install into a throwaway data dir:
`lina pack <id> --bundle try && OPENLINA_HOME=$PWD/work/home ./openlina install dist/try.zip`.

## Unit tests

`cargo test --release` (relocation, validator, manifest, caps), `cargo clippy --release`, `cargo fmt`
(`rustfmt.toml`: 120 columns).
