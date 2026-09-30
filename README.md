# openlina-kit

Everything to create, test and package mods for [Mosa Lina](https://store.steampowered.com/app/2477090/Mosa_Lina/),
the toolkit behind OpenLina (the mod hub in `../openlina-web`). Written in Rust; agents are first-class users
(see [AGENTS.md](AGENTS.md)).

Mods patch the game's HashLink bytecode (`hlboot.dat`). The game install is never modified: the patched game runs
from an overlay directory of symlinks.

## Layout

```
tools/sdk/        openlina-sdk: bytecode lookups, assembler, code editing, validator, hooks, mod runner
tools/lina/       lina: the development CLI (inspect the game, scaffold/build/run/package mods)
tools/openlina/   openlina: the player helper (install packs, build, run, Steam launch wrapper)
mods/<id>/        one crate per mod: mod.toml + src/main.rs (+ assets/, media/)
modpack.toml      which mods `lina build` applies, and their options
docs/             plan, game internals, mods
flake.nix         dev shell: Rust + wasm32-wasip1 target, imagemagick, gifsicle (optional)
rust-toolchain.toml  the same toolchain for rustup users
lina, openlina    wrappers: build (if needed) and run the tools
```

## Quick start (development)

With nix (`nix develop` gives Rust with the wasm target, imagemagick, gifsicle) or without it: install Rust with
[rustup](https://rustup.rs) (the toolchain and the `wasm32-wasip1` target come from `rust-toolchain.toml`) and,
for gifs, ImageMagick (and optionally gifsicle) from your package manager.

```bash
nix develop         # optional
./lina doctor       # checks toolchain, wasm target, ImageMagick, the game; says how to fix what's missing
./lina setup        # copy the pristine hlboot.dat to work/, check the game version
./lina build        # build the mods in modpack.toml and apply them -> work/hlboot.modded.dat + work/game
./lina run          # play it
```

`./lina` and `./openlina` run the tools from their release build and (re)build them when the sources changed
(`target/`, or `target/nix/` inside the nix shell).

The game directory defaults to `~/.local/share/Steam/steamapps/common/Mosa Lina` (`--game-dir` or `MOSA_GAME_DIR`).

## Mods

A mod is a small program: game bytecode in on stdin, patched bytecode out on stdout, options in
`OPENLINA_OPTIONS`. The same crate builds natively (fast development) and for `wasm32-wasip1`, which is what
players get: the helper runs `patch.wasm` in wasmtime with no file or network access.

Mods build on **hooks** provided by the `core` mod instead of patching the same game code, so independent mods
can be combined. See [AGENTS.md](AGENTS.md) for how to write one and [docs/mods.md](docs/mods.md) for the mods.

| id | section | |
|---|---|---|
| `core` | core | hook points (`tick`, `edge_exit`, items, modifiers, texts, level packs); changes nothing on its own |
| `screen-wrap` | modifiers | objects leaving the screen come back on the other side |
| `solid-edges` | modifiers | the screen border is a wall; objects bounce off it (always on) |
| `portal-gun` | items | Portal Gun: shoot two portals, objects and Lina pass through |
| `swap` | items | Swap: trade places with the first object in line of sight |
| `tumble` | levels | Tumble: a drum of tiles that makes a quarter turn every 10 s |
| `mod-menu` | general | pause-menu entry listing the pack's mods and options |
| `ammo-boost` | general | more ammo for every item |
| `trace-calls` | dev | print when chosen functions are called |
| `debug-spawn` | dev | spawn an object in every level (test fixture) |
| `harness` | dev | drive the game without input: level, items, modifier, scripted inputs, frame capture, exit (tests) |

## Packages and packs

```bash
lina pack core screen-wrap --bundle my-pack   # dist/<id>-<version>.zip per mod + dist/my-pack.zip
```

A package is `mod.toml` + `patch.wasm` + `assets/` (overlaid onto `fish/game/res/`) + `media/` (website) +
`source/` (the mod's crate, so others can change it). `lina pack --from <modpack.toml> --bundle <name>` bundles
the mods of a modpack with its options.
A pack zip holds the `openlina` helper, `modpack.toml` and `mods/<id>/`. Players unzip it and run:

```bash
./openlina install .        # installs into ~/.local/share/openlina, builds, prints the Steam launch option
openlina run                # or play from Steam with: "<data dir>/bin/openlina" steam %command%
openlina set screen-wrap coins=true
openlina list | uninstall <id> | build | launch-option
```

Safety: `openlina install` asks before installing mods the site hasn't reviewed (`--yes` skips the question), and
after patching it checks what every mod makes the game able to do (`openlina_sdk::caps`). A mod that reaches
outside the game (files, programs, network, Steam, reflection, changed constants or types) is refused until you
run `openlina allow <id>`; the allowance ends when the mod changes. Gameplay mods never need that.

## The OpenLina website

[openlina-web](../openlina-web) is the mod hub: people pick mods, add change requests and export a pack.

```bash
lina login https://<site> <token>   # token from the site's maintainer; stored in ~/.config/openlina/lina.toml
lina pull <pack link>               # packages, source into mods/<id>/, work/pull/<pack>/REQUESTS.md
lina publish <id>                   # tests (wasm), package, dry run; `--yes` uploads (agents ask the user first)
```

Claude Code picks up the skill in `.claude/skills/openlina-modding/` (new mods, change requests, publishing).

The helper keeps its state in `~/.local/share/openlina` (`OPENLINA_HOME` overrides it): installed mods,
`modpack.toml`, a copy of itself for the launch option, and the overlay game directory `game/`.

## `lina` commands

| command | |
|---|---|
| `setup`, `check` | copy the game bytecode; self-test (roundtrip + validator on all vanilla functions) |
| `dump` | decompile everything into `work/dump/` (`hx/`, `asm/`, `classes.tsv`, `functions.tsv`) |
| `fn`, `callers`, `strings` | inspect one function, its callers, string constants |
| `mods`, `new <section> <id>` | list mods; scaffold a new one |
| `build [--mod id] [--wasm]`, `run [--build] [--timeout s] [--headless]` | apply mods (native or wasm) and play; `--headless` runs without a window (tests) |
| `pack [ids] [--bundle name]` | wasm packages and pack zips in `dist/` |
| `test [files] [--mod id] [--wasm]` | run scenario tests headless (`mods/*/tests/*.toml`) and check their logs |
| `gif <scenario>` | record a scenario's `[gif]` section into a showcase gif (game renderer, headless) |
| `sprite <file> [--out png] [--scale n]`, `sprite --palette` | pixel art from text grids |

## Limitations

- Only the JIT build (`Mosa Lina_jit` + `hlboot.dat`) can run mods; Steam starts the native build, hence the
  launch wrapper. Windows ships the JIT too (untested).
- A game update replaces `hlboot.dat`; mods check every assumption they make about the code and fail at build
  time instead of producing a broken game.
- Builds aren't byte-for-byte reproducible (hlbc stores class bindings in a `HashMap`); this doesn't affect
  the game.
