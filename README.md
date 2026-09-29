# mosa-mod

A modding framework for [Mosa Lina](https://store.steampowered.com/app/2477090/Mosa_Lina/), written in Rust.
It patches the game's HashLink bytecode (`hlboot.dat`); the game install is never modified.

The first mod is **screen-wrap**: items that leave the screen come back on the opposite side instead of being
destroyed. In long/boss levels, which scroll horizontally, only the top and bottom wrap.

## Quick start

```bash
cargo build --release
./target/release/mosa setup          # copy the pristine hlboot.dat to work/, check the game version
./target/release/mosa build          # apply the mods enabled in mods.toml -> work/hlboot.modded.dat
./target/release/mosa run            # play the modded game
```

The game directory defaults to `~/.local/share/Steam/steamapps/common/Mosa Lina`. Override it with
`--game-dir` or `MOSA_GAME_DIR`.

### Playing through Steam

In Steam: *Mosa Lina → Properties → Launch Options*:

```
"/path/to/mosa-mod/scripts/steam-launch.sh" %command%
```

The script starts the game with `work/hlboot.modded.dat`. It falls back to the vanilla game if no modded build
exists or `MOSA_VANILLA=1` is set. Clear the launch options to go back to vanilla for good.

## How it works

The game is written in Haxe on the Heaps engine. It ships two builds:

- `Mosa Lina`: HL/C, compiled to native code. This is what Steam launches, and it can't be modded this way.
- `Mosa Lina_jit` + `hlboot.dat`: the HashLink JIT VM and the same game as bytecode.

`mosa` loads `hlboot.dat` with [hlbc](https://github.com/Gui-Yom/hlbc), applies Rust-written patches, validates
them and writes a new bytecode file. That file is run with the game's own JIT VM
(`Mosa Lina_jit <file>`, from the game directory).

```
crates/
  mosa-bc/    library: lookups, assembler, code editing with jump relocation, validator, Mod trait
  mosa-mods/  the mods (screen-wrap, plus debug tools: trace-calls, debug-spawn)
  mosa/       the CLI
mods.toml     which mods `mosa build` applies, and their options
docs/         game internals, framework guide
AGENTS.md     playbook for AI agents (and humans) writing new mods
```

## Commands

| command | what it does |
|---|---|
| `mosa setup` | copy the pristine `hlboot.dat` to `work/hlboot.orig.dat`, report the game version |
| `mosa dump` | decompile everything into `work/dump/` (`hx/` pseudo-Haxe, `asm/` exact disassembly, `classes.tsv`, `functions.tsv`) |
| `mosa fn <Class.method or findex> [--hx] [--ops a..b] [--input file]` | show one function |
| `mosa callers <Class.method or findex>` | who calls this function / makes a closure of it |
| `mosa strings <text>` | search string constants and the functions that use them |
| `mosa mods` | list the mods and their options |
| `mosa build [--config f] [--mod id]... [--out f]` | apply the mods, validate, write the modded bytecode |
| `mosa run [--build] [--bytecode f] [--timeout s]` | launch the game with a bytecode file |
| `mosa check` | self-test: serializer roundtrip, and the validator on all vanilla functions |

## Mods

| id | description |
|---|---|
| `screen-wrap` | objects leaving the screen wrap to the opposite edge (top/bottom only in long/boss levels) |
| `trace-calls` | debug: print when chosen functions are called |
| `debug-spawn` | debug: spawn an object at a fixed tick of each level (test fixture) |

See [docs/mods.md](docs/mods.md) for their options and exact behavior.

## Limitations

- Only the JIT build can run mods. The launch script handles this for Steam.
- A game update replaces `hlboot.dat`. Run `mosa setup` again. Mods check every assumption they make about the
  code, so they fail at build time with a clear error instead of producing a broken game.
- Builds aren't byte-for-byte reproducible: hlbc stores class bindings in a `HashMap`, so their order in the
  output varies. Their order has no effect on the game.
