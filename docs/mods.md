# Mods

Mods are selected and configured in `modpack.toml` (development) or installed with `openlina install`
(players):

```toml
openlina = 1

[[mod]]
id = "core"

[[mod]]
id = "screen-wrap"
options = { max_overshoot = 150.0 }
```

`lina build --mod <id>` applies only the named mods (plus what they require) with default options.
`lina mods` lists every mod and its options; `openlina set <id> key=value` changes an installed mod's option.

## core

The OpenLina core: hook points other mods subscribe to (`openlina_sdk::hooks::CORE_HOOKS`). It changes nothing
on its own; with no subscribers every hook falls through to vanilla.

| hook | signature | |
|---|---|---|
| `tick` | `(sheet, layout) -> Void` | start of every gameplay tick (`EvSheet_gameplay.update`) |
| `edge_exit` | `(pos, edgewith, margin, sheet, kind) -> Bool` | an object left the screen and vanilla is about to delete it; kind 0 physics object, 1 coin/fruit, 2 secondary physics object; return true to keep it |
| `modifier_pool` | `(pool, dx) -> Void` | a modifier is about to be drawn from `pool` (`LevelManager.rollRaw`/`reroll`); used by `openlina_sdk::modifiers` |
| `modifier_icon` | `(icon) -> Bool` | the HUD modifier icon is about to be set; used by `openlina_sdk::modifiers` |

Source: `mods/core/src/main.rs`.

## screen-wrap

A **modifier**: in levels that roll it, objects that leave the screen reappear on the opposite side, keeping their
velocity. It is registered with `openlina_sdk::modifiers` (key `screen-wrap`, id **8900**), drawn by the game's own
modifier roll like the vanilla modifiers (also in "dx" runs), and shown in the HUD with its own icon
(`assets/images/openlina/screen-wrap.png`, drawn from `art/modifier.toml`). With `always = true` it applies in every
level instead. In long levels and boss arenas (`bossMode`), which scroll horizontally, only the top and bottom wrap,
the same way vanilla only tests those edges there.

Source: `mods/screen-wrap/src/main.rs`. Requires `core`. Tests: `mods/screen-wrap/tests/` (6 scenarios).

| option | default | |
|---|---|---|
| `always` | `false` | Apply in every level instead of as a rolled modifier. |
| `coins` | `false` | Also wrap coins (fruits). Pushing fruits out is how levels are won, so this makes levels unwinnable. |
| `secondary` | `true` | Also wrap "secondary physics" objects. |
| `max_overshoot` | `200` | Only wrap objects at most this far past the edge. Physics objects move at most 100 units per tick (the Box2D speed cap, observed in game), so keep this above 100. Parked objects sit about 1000 out. |
| `min_tick` | `5` | Don't wrap during the first ticks of a layout. |
| `trace` | `false` | Print `[screen-wrap] tick T kind K (x, y) -> (x', y')` for every wrap. |

What stays vanilla:
- The player still dies at the edge.
- Frogs still count as "landed" and disappear.
- Objects the game parks far off-screen (e.g. at −1000, −1000), or places off-screen in level data and deletes
  on the first ticks, are still destroyed. That's what `max_overshoot` and `min_tick` are for; without them,
  junk would drop into levels.

How it works: it subscribes a handler to the core `edge_exit` hook. The handler moves the object by exactly one
play-field size and returns true, so an object that just crossed the bottom limit reappears just past the top
limit and keeps falling into view. See [game-internals.md](game-internals.md#screen-and-edges) for the vanilla
edge test.

Verified in game (all by `lina test`): the game's roll draws it (seed 11); with the modifier a spawned box keeps
wrapping; without it the box reaches the edge and is deleted; fruits stay vanilla unless `coins`; `always` works
without the modifier. The HUD shows the icon.

## ammo-boost

Every item the player gets starts with `factor` times its vanilla ammo (`baseAmmo`): the item slots
(`OClass_b_item`) and item blocks placed in levels (`OClass_item_block`), including state restores
(`StateSerializer.fromBin`). Ammo use, refunds and the slot/item-block swap stay vanilla.

Source: `mods/ammo-boost/src/main.rs`. Requires `core`.

| option | default | |
|---|---|---|
| `factor` | `2` | Multiplier for the starting ammo (0 to 1000). Items without ammo stay empty. |
| `trace` | `false` | Print every boosted value (`[ammo-boost] <item>: ammo 2 -> 4`) and, on tick 1 of every gameplay layout, the slots' real ammo compared with `baseAmmo` (`level start slot 0: frog ammo 6 = 2 x 3`). |

How it works: the game always copies ammo as `Field r = type.baseAmmo; SetField holder.ammo = r`
(11 places in build 22056877); the mod inserts `r = boost(r, type)` between the two ops.

Not boosted: ammo that doesn't come from `baseAmmo` (level-authored `test_item` objects, the `main`
layout's preset slots), and the harness `items` option, which writes slot ammo itself.

## trace-calls (debug)

Prints `[trace-calls] <function> call #N` to stdout: the first `first` calls, then every `every`-th.

```toml
[[mod]]
id = "trace-calls"
options = { functions = ["fish.game.evsheet.EvSheet_gameplay.update", "3751"], first = 3, every = 600 }
```

## debug-spawn (debug)

Subscribes to `tick`. Spawns an object at layout tick `tick` of every gameplay layout: `object` (default `"box"`) at (`x`, `y`)
(default 300, 60) on layer `layer`. It's a test fixture: with `screen-wrap`, the box falls through the floor
and wraps forever.

## harness (dev)

Drives the game for tests and recordings without input (source: `mods/harness/src/main.rs`):

| option | default | |
|---|---|---|
| `start_tick` | 5 | title-screen tick at which to skip to the game (like a key press) |
| `level` | "" | level to load by name, e.g. `"greendemo 1"`; empty keeps the run's first level (the tutorial) |
| `level_n` | -1 | level number within the pack (-1: any) |
| `seed` | 1 | seed for rolling the level, modifier and items |
| `modifier` | -1 | force a modifier id |
| `items` | [] | item names for the item slots, e.g. `["box", "bomb"]` |
| `inputs` | [] | scripted inputs in level ticks: `"60-90:right+jump"`, `"120:shoot"` (up down left right jump shoot switch restart); the keyboard is ignored while set |
| `end_tick` | 0 | exit the game (code 0) at this level tick |
| `capture` / `capture_dir` | "" / "frames" | save 600×338 PNG frames `from-to/step` |
| `list_levels` | false | print every level and item name, then exit |

It prints `[harness] …` lines (`title skipped`, `loading level …`, `level tick 1: <name> modifier <m> frameTime <dt>`,
`end at tick N`, `ERROR …`), which `lina test` checks. Use it through scenarios (`lina test`, `lina gif`), see
AGENTS.md.
