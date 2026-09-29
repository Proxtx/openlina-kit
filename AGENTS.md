# Writing Mosa Lina mods: playbook

For AI agents and humans. Read [README.md](README.md) for the overview, [docs/game-internals.md](docs/game-internals.md)
for what's known about the game, and [docs/PLAN.md](docs/PLAN.md) for where the project stands.

## Setup (once per game version)

```bash
nix develop                       # Rust + wasm32-wasip1, imagemagick, gifsicle
cargo build --release
lina() { target/nix/release/lina "$@"; }
lina setup                        # work/hlboot.orig.dat (+ version check)
lina dump                         # work/dump/: hx/ (pseudo-Haxe), asm/ (exact), classes.tsv, functions.tsv
lina check                        # must report 0 validator problems
```

`work/` and `dist/` are git-ignored. They hold game-derived files; never commit them.

## 1. Find the code

- **Grep the dump.** `work/dump/hx/<pkg>/<Class>.hx` is readable but lossy: the decompiler gets loops, closures
  and some control flow wrong, and prints `// decompilation failed` for about 200 functions (including
  `EvSheet_gameplay.update`, the main game loop). `work/dump/asm/<pkg>/<Class>.asm` is exact. Every function
  header shows `fn@<findex>` and its source `file:line`, and every op its source line (`L1234`).
- Game logic lives in `fish.game.evsheet.EvSheet_*` (event sheets, per-tick `update`), object types in
  `fish.game.oclass.OClass_*`, engine code in `fish.system.*`. See docs/game-internals.md.
- Queries: `lina fn <Class.method|findex> [--hx] [--ops a..b]`, `lina callers <fn>`, `lina strings <text>`,
  `grep -rn "\.fieldName$" work/dump/asm` (readers of a field), `classes.tsv` / `functions.tsv`.
- Many callbacks are anonymous closures in `asm/_global.asm`, attributed by their source line.

## 2. Plan the patch

Prefer, in order:

1. **Subscribe to a hook** (`hooks::handler` + `hooks::subscribe`). Hooks are provided by the `core` mod, listed
   in `openlina_sdk::hooks::CORE_HOOKS`. Several mods can use the same hook without knowing about each other.
2. **Add a hook to `core`** when the game point you need will interest other mods too: define it in
   `mods/core` (`hooks::define` + wiring into the game code) and document it in `CORE_HOOKS`.
3. Patch game code directly, only for mod-specific points:
   - **guard an existing call** (`edit::guard_op`): `if (myFn(args)) skip op;`, vanilla stays as fallback
   - **hook a function's entry** (`edit::prepend_call`)
   - **replace a single op** (`edit::replace_op`)
   - **insert ops** (`edit::insert_ops`, `insert_ops_with_exits`); every jump is relocated for you

Write new logic as new functions with `asm::FnBuilder` (or `hooks::handler`, which gives you one with the hook's
signature) instead of long inline op sequences: easier to read, validate, and trace (`openlina/<name>:<op>` in
stack traces).

## 3. Write the mod

```bash
lina new items portal-gun         # mods/portal-gun/: Cargo.toml, mod.toml, src/main.rs (tick hook example)
```

- `mod.toml`: id (= directory name), name, version, section (`items|modifiers|levels|general|dev`),
  description, `requires = ["core"]`, options with type/default/description. Format: `openlina_sdk::manifest`.
- `src/main.rs`: `openlina_sdk::run_mod(|code, cfg| { ... })`. Read options with `cfg.bool/i64/f64/str`; the host
  always passes every declared option (defaults filled in), and rejects unknown or mistyped ones.
- `assets/` (optional): files overlaid onto `fish/game/res/` (e.g. `assets/images/my-sheet.png`).
- `media/` (optional): icon and showcase gifs for the website.
- Add it to `modpack.toml` to include it in `lina build`.

Rules:

- **Find anchors by meaning, not by index.** Look ops up through field names (`edit::find_field_access`), calls
  (`edit::find_calls`, `call_target`) and nearby ops (`next_match`, `prev_match`). Pin each anchor down with
  `edit::expect_one` / `ensure!`, so a game update fails the build instead of patching the wrong op.
- Registers: take them from the anchors (`Field { dst, obj, .. }`) and check their types before relying on them.
- When patching several places in one function, go from the **last index to the first**: inserting ops shifts
  later indices.
- Expose tunables as options, and add a `trace` option that prints what the mod does (`FnBuilder::print`).
- The module doc comment of `src/main.rs` is the mod's design document: what vanilla does, what the mod changes,
  what it leaves alone. Add the mod to `docs/mods.md` and new findings to `docs/game-internals.md`.

## 4. Verify

1. `lina build --mod <id>` (natively; `--wasm` for exactly what players run). Each mod validates the functions
   it touched (register bounds, jumps, call arity and value kinds, fields, returns) before writing its output,
   and the result must parse.
2. `lina fn <patched fn> --input work/hlboot.modded.dat --ops a..b` to eyeball the patch.
3. **Run it headless**, no window and no human needed:
   `lina run --headless --timeout 30 > log 2>&1`. SDL's offscreen driver renders through EGL. Look for `SIGNAL`,
   `Uncaught` or a stack trace; injected code appears as `openlina/<name>:<line>`.
4. **Runtime checks without input.** Synthetic input (`xdotool`) doesn't reach the game. Put test fixtures in a
   test modpack (e.g. `work/test.toml`, `lina build --pack work/test.toml`):
   - `autostart` skips the title screen, so the run reaches the first level
   - `debug-spawn` (or a fixture mod of your own, subscribing to `tick`) sets up a situation in a level
   - `trace-calls` (`options = { functions = [...] }`) and your mod's `trace` option print what happens; grep the
     log. The physics is deterministic: the same build and fixtures give the same ticks and coordinates, so
     assertions on trace lines are reliable.
   - screenshots of a windowed run: `import -window $(xdotool search --name '^Mosa Lina$' | head -1) shot.png`
5. `cargo test --release` (relocation, validator and manifest tests; the relocation tests use the real bytecode)
   and `cargo clippy --release`.
6. `lina pack <id>` to produce the package; test the player flow with a throwaway data dir:
   `OPENLINA_HOME=work/home dist/…/openlina install dist/<id>-<version>.zip`.

## HashLink pitfalls (learned the hard way)

- The **`String` opcode yields raw UTF-16 `hl.Bytes`**, not a `String` object. Use `FnBuilder::string_obj`.
- **String pool index 0 is `"String"`**, but hlbc's `Resolve` prints index 0 as `<none>`. Use `Code::str`.
- Jumps are relative: `target = pos + 1 + offset`. Backward jump targets must be `Label` ops. `Switch.end` marks
  the end of the switch block and may equal `ops.len()`; the default case falls through.
- Free functions have no name in the bytecode. Injected ones are recognized by their debug file
  (`openlina/<name>`); hooks by `openlina/hook/<name>`.
- A call's `dst` register must match the callee's return kind, or be `Void` to discard the result.
- hlbc's private findex tables are only rebuilt on load. After adding functions, use `Code::func`/`func_type`
  (linear search), not hlbc's `code.get(RefFun)`.
- Only `Mosa Lina_jit` runs bytecode. It needs its resources relative to the working directory and the game dir on
  `LD_LIBRARY_PATH` (`lina run` / `openlina run` do this).
- Setting `sprite.position` teleports physics objects (`Physics.syncPosWithSprite`).
- The game parks objects off-screen and deletes objects placed off-screen in the first ticks of a layout.
  Anything that changes edge or destroy behavior needs to account for that.
- Mods run as separate processes on the whole bytecode: a mod cannot see another mod's Rust state, only what the
  previous mods left in the bytecode (hooks, debug file names).

## Library map (`tools/sdk`, crate `openlina_sdk`)

| module | purpose |
|---|---|
| `Code` (lib.rs) | load/save, `class`, `field`, `field_type`, `method`, `native`, `func(_mut)`, `func_type`, `func_name`, `op_location`, interning (`string`, `float`, `int`, `intern_type`, `ty_*`), `add_global` |
| `asm::FnBuilder` | new functions: registers, labels, jumps, constants, `get`/`set` fields, `call`, `string_obj`, `print` |
| `edit` | `find*`, `expect_one`, `next_match`/`prev_match`, `replace_op`, `insert_ops`, `insert_ops_with_exits`, `guard_op`, `prepend_call`, `remove_ops`, `add_reg` |
| `hooks` | `CORE_HOOKS`, `find`, `signature`, `handler`, `subscribe`, `define` |
| `runner` | `run_mod`: the `main` of every mod |
| `manifest` | `ModManifest` (mod.toml), `ModPack` (modpack.toml), `resolve_order` |
| `validate` | `check_function`, `check_touched`, `kind` |
