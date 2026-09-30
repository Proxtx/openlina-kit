---
name: openlina-modding
description: Make, change, test and share mods for the game Mosa Lina with openlina-kit (HashLink bytecode patches in Rust). Use for any Mosa Lina mod work - a new item, modifier, level or general mod, applying the change requests of an OpenLina pack (`lina pull` with the pack link), recording showcase gifs, installing a pack for the player, or uploading a mod to an OpenLina site (`lina publish`, only after the user agreed).
---

# OpenLina modding

Work in an `openlina-kit` checkout (a directory with `AGENTS.md` and `tools/lina/`). If the current directory isn't
one, look for it (`find ~ -maxdepth 6 -type d -name openlina-kit`) or ask the user; if there is none, offer to
clone https://github.com/Proxtx/openlina-kit (ask first). The full playbook is `AGENTS.md` in the checkout: read it
before patching game code. Game knowledge: `docs/game-internals.md` (index of `docs/game/*.md`); existing mods: `docs/mods.md`; project state: `docs/PLAN.md`.

## Setup (once per machine and game version)

```bash
lina() { ./lina "$@"; }          # wrapper: builds the tools when needed, then runs them
lina doctor                      # toolchain, wasm target, ImageMagick, game: fix whatever it reports
lina setup && lina dump && lina check   # pristine bytecode, searchable dump, validator self-test
```

With nix, run everything inside `nix develop`. Without nix, Rust comes from rustup (plus
`rustup target add wasm32-wasip1`) and ImageMagick from the system's package manager. Ask the user before
installing or updating anything, including rustup targets or toolchains. The game must be installed (Steam, Linux). Everything runs headless.

## A. A new mod

1. Pick the section: `items` (a tool Lina fires), `modifiers` (a rule rolled per level), `levels` (level packs),
   `general` (anything else). Look at the showcase mod of that section first:
   `portal-gun`, `screen-wrap`/`solid-edges`, `tumble`, `mod-menu`. Find out how the game does the thing by
   looking at it running (`lina probe … --at tick:30 <paths>`) and searching (`lina refs`, `lina class`).
2. `lina new <section> <id>`, then write `src/main.rs` with the SDK (docs/sdk.md): prefer core hooks
   (`lina hooks`) and the registries (`items`, `modifiers`, `levels`, `text`, `anims`) over raw patches.
3. Options for every tunable, a `trace` option, `[stats]` in `mod.toml` for the website, the design notes as the
   module doc comment.
4. Graphics as text grids: `art/*.toml` → `lina sprite … --out assets/images/…` (game) and `media/icon.png` (site).
5. Tests (docs/testing.md): scenarios in `tests/*.toml` that can fail (`lina test --mod <id>`, then `lina test`
   and `--wasm`; `-k`, `--failed`), a showcase gif
   (`lina gif mods/<id>/tests/<x>.toml`); look at the frames in `work/gif/frames/` before keeping it.
6. Add it to `docs/mods.md`; commit.

## Bugs (a mod hangs, crashes or misbehaves)

Follow docs/debugging.md: reproduce it as a failing scenario first (`new_run` + `new_run_items` for real play,
`chaos` for "sometimes"), look with `lina probe` and `trace-calls` (with arguments), explain with `lina refs`, fix
in the SDK when a registry misses a place the game lists things, keep the scenario as a test, run everything.

## B. A pack from the website (change requests)

A user gives you a pack link (`https://<site>/api/packs/<id>`) or JSON from the site's export page.

1. `lina pull <link>`: downloads the packages, puts the source of mods you don't have into `mods/<id>/`, writes
   `work/pull/<pack>/modpack.toml` and `REQUESTS.md` (every request with the mod's options).
2. Work through `REQUESTS.md`:
   - A request an option covers → set the option for this pack in `work/pull/<pack>/modpack.toml`. No code change.
   - Otherwise change the mod's code, **bump its version** in `mod.toml`, add a scenario proving the change, run
     `lina test --mod <id>` and `--wasm`, refresh the gif if the behavior it shows changed.
   - Section requests ("all items: …") apply to every mod of that section in the pack.
   - Conflicts ("Make A and B work together"): the user wants both. Change the mods so they coexist and drop the
     `conflicts` entry; refuse only the option combinations that truly can't work, with a clear build error.
   - A request you can't do safely (or that contradicts the mod's purpose): tell the user instead of guessing.
3. Check the pack as a whole: `lina build --pack work/pull/<pack>/modpack.toml` (and `lina run` if the user
   wants to play right away).
4. Install for the player: `lina pack --from work/pull/<pack>/modpack.toml --bundle pack-<pack>` (your local
   versions, the pack's options, requirements added), then `./openlina install
   dist/pack-<pack>.zip` (prints the Steam launch option; ask before the user changes Steam settings).
5. Report what changed per request (option or code, versions, test results).

## C. Uploading (only with the user's OK)

- There is no default site. `lina doctor` shows whether (and where) lina is logged in. If it isn't, **ask the
  user** for the site's URL (e.g. `https://openlina.example`) and whether they have an upload token (it comes from
  the site's maintainer). Best: the user logs in themselves in a terminal, `./lina login <site>`, and pastes the
  token at the prompt, so it never passes through you. Otherwise pipe it on stdin (`lina login <site> < file`);
  never put it on the command line, print it or commit it.
- Pulling needs no login: a full pack link names its site. Only a bare pack id needs the site from `lina login`.
- `lina publish <id>` runs the mod's scenarios on the wasm build, packages it with its source, and shows what
  would be uploaded (dry run). **Show that summary to the user and ask** whether to upload.
- Only after a clear yes: `lina publish <id> --yes`. New uploads are `unreviewed` until a maintainer approves them.
- A version can't be uploaded twice: bump `version` for every change. Only the owner of an id (or an admin) can
  upload new versions; to change someone else's mod for yourself, keep it local (B.4) or publish under a new id
  after asking the user.
- After finishing a mod (A.6), offer the upload; don't upload unprompted.

## Safety (someone else's mods)

- `lina pull` only extracts plain mod crates: no `build.rs`, no hidden files, no dependencies beyond the kit's
  workspace ones. Anything else lands in `work/pull/<pack>/quarantine/` for reading; don't move it into `mods/`
  yourself.
- Pulled mods carry `mods/<id>/.openlina-pulled`: lina builds and runs them only as wasm, and refuses them if
  they make the game reach outside the game (files, programs, network, Steam, reflection; see
  `openlina_sdk::caps`). Read their code and the change against the version you pulled. Delete the marker only
  when the user decided to trust the mod.
- `lina build` also warns when your own mods reach outside the game. Players' `openlina` refuses such mods
  unless they run `openlina allow <id>`; gameplay mods never need it, so rework the mod instead.
- Unreviewed mods: `openlina install` asks before installing them; tell the user the mod is unreviewed.

## Rules

- Never commit or upload game files (`work/`, `dist/` are ignored for that reason). Packages hold only mod code,
  assets you made, and media.
- Anchors by meaning, `ensure!` on every assumption, validate (`lina build` does), test in the game.
- Ask before anything that leaves the machine (uploads) or changes the user's game setup (Steam launch options).
