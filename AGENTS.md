# Writing Mosa Lina mods: playbook

For AI agents and humans. Read [README.md](README.md) for the overview and
[docs/game-internals.md](docs/game-internals.md) for what's already known about the game.

## Setup (once per game version)

```bash
cargo build --release
./target/release/mosa setup   # work/hlboot.orig.dat (+ version check)
./target/release/mosa dump    # work/dump/: hx/ (pseudo-Haxe), asm/ (exact), classes.tsv, functions.tsv
./target/release/mosa check   # must report 0 validator problems
```

`work/` is git-ignored. It holds game-derived files, so never commit it.

## 1. Find the code

- **Grep the dump.** `work/dump/hx/<pkg>/<Class>.hx` is readable but lossy: the decompiler gets loops, closures
  and some control flow wrong, and prints `// decompilation failed` for about 200 functions (including
  `EvSheet_gameplay.update`, the main game loop). `work/dump/asm/<pkg>/<Class>.asm` is exact. Every function
  header shows `fn@<findex>` and its source `file:line`, and every op shows its source line (`L1234`), so you
  can follow the original code line by line.
- Game logic lives in `fish.game.evsheet.EvSheet_*` (event sheets, per-tick `update`), object types in
  `fish.game.oclass.OClass_*`, engine code in `fish.system.*`.
- Useful queries:
  - `mosa fn <Class.method|findex> [--hx] [--ops a..b]`
  - `mosa callers <fn>`: who calls it, or makes a closure of it
  - `mosa strings <text>`: string constants, and which functions use them
  - `grep -n "\.fieldName$" work/dump/asm -r`: readers of a field
  - `classes.tsv` / `functions.tsv` for names, fields and signatures
- Many callbacks are anonymous closures in `asm/_global.asm`, attributed by their source line.

## 2. Plan the patch

Prefer, in order:

1. **Guard an existing call** (`edit::guard_op`): `if (myFn(args)) skip op;`. The vanilla code stays as the
   fallback. That's the screen-wrap approach.
2. **Hook a function's entry** (`edit::prepend_call`) to run code every time it's called.
3. **Replace a single op** (`edit::replace_op`) when the op count can stay the same.
4. **Insert arbitrary ops** (`edit::insert_ops`, `insert_ops_with_exits`). Every jump is relocated for you.

Write the new logic as a new function with `asm::FnBuilder` instead of long inline op sequences. It's easier to
read, validate, and trace (`mosa/<name>:<op>` in stack traces).

## 3. Write the mod

Create `crates/mosa-mods/src/<name>.rs` implementing `mosa_bc::Mod`, and register it in
`crates/mosa-mods/src/lib.rs`. `screen_wrap.rs` is the reference example; `debug_spawn.rs` and
`trace_calls.rs` are short examples of hooks, calling game functions, and globals.

Rules:

- **Find anchors by meaning, not by index.** Look ops up through field names (`edit::find_field_access`),
  calls (`edit::find_calls`, `call_target`) and nearby ops (`next_match`, `prev_match`). Pin each anchor down
  with `edit::expect_one` / `ensure!`, so a game update fails the build instead of patching the wrong op.
- Registers: take them from the anchors (`Field { dst, obj, .. }`), and check their types
  (`fun.regs[r.0 as usize]`) before relying on them.
- When patching several places in one function, go from the **last index to the first**, since inserting ops
  shifts later indices.
- Expose tunables as options (`fn options()` + `cfg.bool/f64/i64/str`). Add a `trace` option that prints what
  the mod does (`FnBuilder::print`).
- Document in the module doc comment what the vanilla code does and what you change, then add the mod to
  `docs/mods.md` and any new findings to `docs/game-internals.md`.

## 4. Verify

1. `mosa build --config <test.toml> --out work/hlboot.test.dat`. This runs the validator (register bounds,
   jumps, call arity and value kinds, fields, returns) on everything you touched, before and after
   re-serialization.
2. `mosa fn <patched fn> --input work/hlboot.test.dat --ops a..b` to eyeball the patch.
3. `mosa run --bytecode work/hlboot.test.dat --timeout 30 > log 2>&1`. A boot test: look for `SIGNAL`,
   `Uncaught` or a stack trace in the log. Injected code appears as `mosa/<fn>:<line>`.
4. **Runtime checks without input:** you can't drive the game with synthetic input (`xdotool` doesn't reach
   it). Use:
   - `trace-calls` to check whether code runs (e.g. `EvSheet_gameplay.update` only runs for 2 ticks on the
     title screen)
   - your mod's `trace` option
   - `debug-spawn` (or a fixture mod of your own) to set up a situation in a level
   - screenshots: `import -window $(xdotool search --name '^Mosa Lina$' | head -1) shot.png`
   - a human to press a key and get past the title screen into the first level
5. `cargo test --release` (relocation and validator tests against the real bytecode) and `cargo clippy`.

## HashLink pitfalls (learned the hard way)

- The **`String` opcode yields raw UTF-16 `hl.Bytes`**, not a `String` object. Use `FnBuilder::string_obj`
  (`String.__alloc__(bytes, len)`).
- **String pool index 0 is `"String"`**, but hlbc's `Resolve` prints index 0 as `<none>`. Use `Code::str`,
  which reads the pool directly.
- Jumps are relative: `target = pos + 1 + offset`. Backward jump targets must be `Label` ops.
  `Switch.end` marks the end of the switch block and may equal `ops.len()`; the default case falls through.
- Free functions have no name in the bytecode. hlbc reports string 0; `Code::func_name` names injected ones
  after their `mosa/<name>` debug file.
- A call's `dst` register must match the callee's return kind, or be `Void` to discard the result.
- hlbc's private findex tables are only rebuilt on load. After adding functions, use `Code::func`/`func_type`
  (linear search) or `Code::reload()`, not hlbc's `code.get(RefFun)`.
- Only `Mosa Lina_jit` runs bytecode. It needs the game directory as cwd and on `LD_LIBRARY_PATH`
  (`mosa run` does this).
- Setting `sprite.position` teleports physics objects (`Physics.syncPosWithSprite`).
- The game parks objects off-screen and deletes objects placed off-screen in the first ticks of a layout.
  Anything that changes edge or destroy behavior needs to account for that.

## Library map (`crates/mosa-bc`)

| module | purpose |
|---|---|
| `Code` (lib.rs) | load/save, `class`, `field`, `field_type`, `method`, `native`, `func(_mut)`, `func_type`, `func_name`, `op_location`, interning (`string`, `float`, `int`, `intern_type`, `ty_*`), `add_global` |
| `asm::FnBuilder` | new functions: registers, labels, jumps, constants, `get`/`set` fields, `call`, `print` |
| `edit` | `find*`, `expect_one`, `next_match`/`prev_match`, `replace_op`, `insert_ops`, `insert_ops_with_exits`, `guard_op`, `prepend_call`, `remove_ops`, `add_reg` |
| `validate` | `check_function`, `check_touched`, `kind` |
