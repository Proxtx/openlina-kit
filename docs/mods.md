# Mods

Mods are enabled and configured in `mods.toml`:

```toml
[screen-wrap]
enabled = true
max_overshoot = 150
```

A table without `enabled` counts as enabled. `mosa build --mod <id>` applies only the named mods, with default
options. `mosa mods` lists every mod and its options.

## screen-wrap

Objects that leave the screen reappear on the opposite side, keeping their velocity. Vanilla destroys them. In
long levels and boss arenas (`bossMode`), which scroll horizontally, only the top and bottom wrap, the same way
vanilla only tests those edges there.

Source: `crates/mosa-mods/src/screen_wrap.rs`.

| option | default | |
|---|---|---|
| `coins` | `false` | Also wrap coins (fruits). Pushing fruits out is how levels are won, so this makes levels unwinnable. |
| `secondary` | `true` | Also wrap "secondary physics" objects. |
| `max_overshoot` | `200` | Only wrap objects at most this far past the edge. Physics objects move at most 100 units per tick (the Box2D speed cap, observed in game), so keep this above 100. Parked objects sit about 1000 out. |
| `min_tick` | `5` | Don't wrap during the first ticks of a layout. |
| `trace` | `false` | Print `[screen-wrap] tick T (x, y) -> (x', y')` for every wrap. |

What stays vanilla:
- The player still dies at the edge.
- Frogs still count as "landed" and disappear.
- Objects the game parks far off-screen (e.g. at −1000, −1000), or places off-screen in level data and deletes
  on the first ticks, are still destroyed. That's what `max_overshoot` and `min_tick` are for; without them,
  junk would drop into levels.

How it works: it injects `mosa_screen_wrap(pos, edgewith, margin, sheet): Bool` and guards the vanilla
`destroy()` (and optionally `coinedgecheck()`) calls in `EvSheet_gameplay.update`:
`if (mosa_screen_wrap(...)) skip;`. Wrapping moves the object by exactly one play-field size, so an object
that just crossed the bottom limit reappears just past the top limit and keeps falling into view. See
[game-internals.md](game-internals.md#screen-and-edges) for the vanilla edge test.

Verified in game: a box spawned by `debug-spawn` in the first tutorial level falls, wraps from y≈346 to y≈−6
and loops indefinitely (4,500+ wraps in one session, reaching the 100 units/tick speed cap). The title screen and the level start show no spurious wraps.

## trace-calls (debug)

Prints `[trace-calls] <function> call #N` to stdout: the first `first` calls, then every `every`-th.

```toml
[trace-calls]
functions = ["fish.game.evsheet.EvSheet_gameplay.update", "3751"]
first = 3
every = 600
```

## debug-spawn (debug)

Spawns an object at layout tick `tick` of every gameplay layout: `object` (default `"box"`) at (`x`, `y`)
(default 300, 60) on layer `layer`. It's a test fixture: with `screen-wrap`, the box falls through the floor
and wraps forever.
