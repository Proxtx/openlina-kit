# Writing Mosa Lina mods: playbook

For AI agents and humans. Mods are Rust programs that patch the game's HashLink bytecode; `lina` builds, tests and
shares them, `openlina` installs them for players. This file is the workflow and the rules. Details:

| read | when |
|---|---|
| [docs/testing.md](docs/testing.md) | writing scenarios, gifs, sprites; running tests |
| [docs/sdk.md](docs/sdk.md) | using the SDK: library map, registry checklists, HashLink pitfalls |
| [docs/debugging.md](docs/debugging.md) | something hangs, crashes or behaves wrong |
| [docs/game-internals.md](docs/game-internals.md) | what is known about the game's code: an index of `docs/game/*.md` by topic |
| [docs/mods.md](docs/mods.md) | the existing mods and dev fixtures, with their options |
| [docs/PLAN.md](docs/PLAN.md) | where the project stands |
| [docs/blind-test.md](docs/blind-test.md) | after big changes: let a fresh agent build a mod and fix what it stumbles on |

## Setup (once per machine and game version)

```bash
nix develop                       # optional: Rust + wasm32-wasip1, imagemagick, gifsicle. Without nix: rustup +
                                  # `rustup target add wasm32-wasip1` + imagemagick (ask the user before installing)
lina() { ./lina "$@"; }           # the wrapper builds the tools when needed
lina doctor                       # checks all of the above and the game; fix what it reports
lina setup && lina dump && lina check   # pristine bytecode, searchable dump, 0 validator problems
```

With nix, `nix develop --no-warn-dirty -c <command>` runs one command in the shell without the "Git tree is dirty"
warning on every call.

`work/` and `dist/` are git-ignored. They hold game-derived files; never commit them.

## 1. Find out how the game does it

- **Look at it running** first: `lina probe` prints any part of the game's state at a moment you pick (~7 s):
  `lina probe --mod swap --level "greendemo 1" --at tick:30 game.itemManager.itemPool.length "@item[].NAME"`.
  `--new-run` plays the run start, `--at layout:manager@200` works on screens without gameplay, `--input`,
  `--capture`. It writes `work/probe.toml`, a scenario you can grow into a test.
- **Search the code**: `lina refs <field|string>` (who reads, writes or uses it), `lina class <Class>`,
  `lina fn <Class.method|findex> [--hx] [--ops a..b]`, `lina callers <fn>`, `lina hooks`. The package can be left
  out of names.
- **The dump** (`work/dump/`): `hx/` is readable pseudo-Haxe but lossy (loops, closures; ~200 functions fail,
  including `EvSheet_gameplay.update`); `asm/` is exact, with `fn@<findex>`, source `file:line` and per-op `L1234`.
  Closures live in `asm/_global.asm`. Game logic: `fish.game.evsheet.EvSheet_*` (per-tick `update`); objects:
  `fish.game.oclass.OClass_*`; engine: `fish.system.*`.

## 2. Plan the patch

Prefer, in order:

1. **A registry or hook of the SDK** (`items`, `modifiers`, `levels`, `text`, `anims`; `hooks::handler` +
   `hooks::subscribe` on the `core` hooks, `lina hooks`). Several mods can use the same hook without knowing about
   each other. A registry must cover every place the game lists that kind of thing: see the checklists in
   docs/sdk.md.
2. **A new `core` hook** when the game point will interest other mods too (`hooks::define` in `mods/core`, document
   it in `CORE_HOOKS`).
3. **A direct patch** for mod-specific points: guard a call (`edit::guard_op`), hook an entry
   (`edit::prepend_call`), replace an op (`edit::replace_op`), insert ops (`edit::insert_ops*`, jumps are
   relocated).

Write new logic as new functions (`asm::FnBuilder`, or `hooks::handler`, which has the hook's signature) instead of
long inline op sequences: easier to read, validate and trace (`openlina/<name>:<op>` in stack traces).

## 3. Write the mod

`lina new <section> <id>` creates `mods/<id>/` from a template for the section (`items` and `modifiers` come with
the registry call, placeholder icons and a smoke test that runs; `levels|general|dev` with a tick hook). Look at
the showcase mod of the section first: `portal-gun`/`swap`, `screen-wrap`/`solid-edges`, `tumble`, `mod-menu`.

- `mod.toml` (format: `openlina_sdk::manifest`): id (= directory), name, version, section, description,
  `requires = ["core"]`, options (`bool|int|float|string|list`, default, description), `[stats]` for the website,
  `showcase` gif order.
- **Load order**: a mod runs after its `requires` (must be present) and `after` (if present); `core` first;
  otherwise by id. `conflicts = [...]` refuses to build with the listed mods: only for combinations that can
  never work. Mods that can't meet don't conflict (a level has one modifier); a conflict that depends on options is
  checked in code (`runner::pack_info`, see `solid-edges`). Website packs may hold conflicting mods; `lina pull`
  turns them into tasks.
- `src/main.rs`: `openlina_sdk::run_mod(|code, cfg| { … })`; options via `cfg.bool/i64/f64/str/list` (every
  declared option is passed, unknown ones are rejected). Its module doc comment is the design document: what
  vanilla does, what the mod changes, what it leaves alone.
- `assets/` overlays `fish/game/res/`; `media/` holds the website icon and gifs; `art/*.toml` pixel art.
- `modpack.toml` is the development pack for `lina build` / `lina run` without `--mod`.

Rules:

- **Anchors by meaning, not by index**: find ops through field names (`edit::find_field_access`), calls
  (`edit::find_calls`) and neighbors (`next_match`, `prev_match`); pin each with `edit::expect_one` / `ensure!`, so
  a game update fails the build instead of patching the wrong op. Take registers from the anchors and check their
  types. Patch several places in one function from the last index to the first.
- Every tunable is an option; add a `trace` option that prints what the mod does (`FnBuilder::print`).
- Add the mod to `docs/mods.md` (option tables are generated: `lina docs` after changing options) and what you
  learned about the game to the topic file in `docs/game/`.

## 4. Verify

1. `lina build --mod <id>` (`--wasm` for what players run): each mod validates what it touched and the result must
   parse. `lina fn <fn> --input work/hlboot.modded.dat` shows a patched or injected function (`swap/use`).
2. Scenarios in `mods/<id>/tests/*.toml` that **can fail** (docs/testing.md); `lina test --mod <id>`, then
   `lina test` (all, <1 min) and `lina test --wasm`. `-k <text>`, `--failed`.
3. A showcase gif (`lina gif`); look at the frames before keeping it.
4. `cargo test --release`, `cargo clippy --release`, `cargo fmt`, `lina docs --check`.

When something hangs, crashes or misbehaves: docs/debugging.md (reproduce as a scenario, `lina probe`,
`trace-calls` with arguments, `lina refs`, fix, keep the test). A player's problem arrives as an `openlina report`
zip (installed mods, versions, options, game version, the last runs' output, the game's crash report); your own
play becomes replay scenarios with `lina run --record`.

## 5. Share (OpenLina website)

- `lina login <site>`: the user logs in with a token from the site's maintainer; there is no default site, so ask.
  Saved in `~/.config/openlina/lina.toml` (600) or `$OPENLINA_CONFIG`. Never print, pass on the command line or
  commit a token.
- `lina pull <pack link>`: mods you don't have into `mods/<id>/`, and `work/pull/<pack>/` with `modpack.toml` and
  `REQUESTS.md` (the change requests as a to-do list).
- `lina publish <id>`: wasm scenarios, package with source, dry-run summary; uploads only with `--yes`, **after the
  user agreed**. The skill `.claude/skills/openlina-modding` has the full workflow.
- **Safety**: after every mod, `lina` and the player's `openlina` compare the bytecode (`openlina_sdk::caps`) and
  report new uses of files, programs, network, Steam, reflection, new natives, changed constants or types. `lina`
  warns; `openlina` refuses the mod until the player runs `openlina allow <id>`. Pulled mods (`.openlina-pulled`)
  build and run only as wasm and are refused when they reach outside the game; sources with build scripts, hidden
  files or foreign dependencies are quarantined.
