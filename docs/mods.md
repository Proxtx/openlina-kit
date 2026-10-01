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
| `tick` | `(sheet, layout) -> Void` | start of every gameplay tick (`EvSheet_gameplay.update`): levels, and also the hub (`help`), the tool selection (`manager`) and the title screens' first ticks; `openlina_sdk::world::is_level` tells levels apart |
| `player_edge` | `(pos, margin, sheet, player) -> Bool` | Lina (state "normal") left the screen and vanilla is about to call `player_death` (a pit, a side); return true to keep her alive. Also on the hub, where walking off the right edge starts a run |
| `edge_exit` | `(pos, edgewith, margin, sheet, kind, physics) -> Bool` | an object left the screen and vanilla is about to delete it; kind 0 physics object, 1 coin/fruit, 2 secondary physics object; return true to keep it |
| `modifier_pool` | `(pool, dx) -> Void` | a modifier is about to be drawn from `pool` (`LevelManager.rollRaw`/`reroll`); used by `openlina_sdk::modifiers` |
| `modifier_icon` | `(icon) -> Bool` | the HUD modifier icon is about to be set; used by `openlina_sdk::modifiers` |
| `item_pool` | `(itemManager) -> Void` | the item pool was built; used by `openlina_sdk::items` |
| `item_use` | `(slot, sheet, player, crosshair) -> Bool` | the player fires the item in `slot` (ammo already decremented); return true to replace the game's behavior |
| `loc` | `(key) -> String` | a text is looked up; return a text or null; used by `openlina_sdk::text` |
| `packs` | `(packManager) -> Void` | the game loaded its level packs (PackManager constructor); push packs; used by `openlina_sdk::levels` |

`edge_exit` also passes the object's `physics` behavior (last argument).

Source: `mods/core/src/main.rs`.

## portal-gun (items)

A new item, **Portal Gun** (`portal`, 4 ammo, long aim): each shot places a blue, then an orange portal on the
next surface along Lina's aim within `range` (400), half a portal in front of it, inside the play field; the
`ray-crosshair` marker shows that surface while the gun is selected. Only level geometry takes portals: a box,
fruit or vine segment in the way blocks the shot, and with nothing in range it fizzles (`mid_air`: the portal goes
to the end of the range instead). Any moving object that gets within `radius`
(14) of one portal comes out of the other, pushed out along its direction of travel with its velocity kept;
Lina too. Portals reset every level. Pool entry, HUD icon and label via `openlina_sdk::items`; portals are
`Sprite15` objects with the mod's own animations (`assets/images/openlina/portal-*.png`, drawn from `art/*.toml`).

Options: `ammo`, `radius`, `range`, `mid_air`, `preset` (`"ax,ay,bx,by"`: place both portals at level start, for
tests and showcases), `trace`. Tests: in the pool; two shots down place blue then orange on the platform under Lina
(she then goes through); the vine blocks a shot and the sky makes one fizzle; `mid_air`; a box loops through the
portals (showcase `media/loop.gif`).

Options:

<!-- options:portal-gun -->
| option | type | default | description |
|---|---|---|---|
| `ammo` | int | `4` | Portal shots per level. |
| `radius` | float | `14.0` | How close to a portal's center an object must get to go through. |
| `range` | float | `400.0` | How far the shot reaches from Lina along the aim: the portal lands on the first surface within this distance (kept inside the play field). Only level geometry takes portals; a box or other movable object in the way blocks the shot. |
| `mid_air` | bool | `false` | With no surface within `range`, place the portal in mid-air at the end of the range. Off: the shot fizzles. |
| `preset` | string | `""` | Place both portals when a level starts: "ax,ay,bx,by" (tests, showcases). |
| `trace` | bool | `false` | Print portal placements (also as `[pos] tick T portal-blue x y`), shots that place nothing, and teleports. |
<!-- /options -->

## swap (items)

A new item, **Swap** (`swap`, 3 ammo, long aim): each shot casts a ray from Lina along her aim, `range` (400) units
long. The first object the ray hits trades places with her: Lina goes where the object was, the object goes where
Lina was (velocities kept). If nothing is hit, nothing happens, and the shot is still used. With `walls_block`
(default) static level geometry stops the ray, so a wall in between means no swap; without it the ray passes
through walls and only movable objects count. Anything with a Box2D fixture can be swapped: boxes, fruits, frogs,
vines, enemies, the other player in co-op.

The ray is the game's own Box2D ray cast (native `world_ray_cast`, as used by the Line of Sight behavior
`fish.system.beh.LOS`) through `openlina_sdk::aim::AimRay`: the nearest hit, Lina herself skipped. While Swap is
selected, the `ray-crosshair` marker shows where that ray stops (`swap-box.toml` checks that it sits on the box
before the shot). Pool entry, HUD
icon and label via `openlina_sdk::items`; icon drawn from `art/icon.toml` (`assets/…/swap.png`, and at 2× as
`swap-big.png` for the tool selection and the editor).

Source: `mods/swap/src/main.rs`. Requires `core` and `ray-crosshair`. `tests/run-start-selection.toml` checks that a run start whose tool
roll drew swap goes on into the first level.

<!-- options:swap -->
| option | type | default | description |
|---|---|---|---|
| `ammo` | int | `3` | Swap shots per level (0 to 99). |
| `aim` | string | `"long"` | How the item is aimed (the game's aim types): shoot, short, short2, mid, mid2, long, long2 or remote. Only the direction counts; the ray is `range` long. |
| `range` | float | `400.0` | How far the ray reaches from Lina, in layout units (the play field is 600 x 338). |
| `walls_block` | bool | `true` | Static level geometry (tiles, walls) stops the ray: nothing is swapped when the ray hits a wall first. Off: the ray passes through walls and only movable objects count. |
| `check_ticks` | int | `12` | With trace: how many ticks after a swap to print where both objects really are (a check for tests). |
| `trace` | bool | `false` | Print every shot (`[swap] tick T: ray …`, `player (x, y) <-> box (x, y)`, `nothing in line of sight`, `blocked by <type>`, with the ammo left) and the check after each swap (`[swap] check: … near the object's old spot: true`). |
<!-- /options -->

Tests (`mods/swap/tests/`, 8 scenarios): in the pool with 3 shots; `ammo` option; a box right of Lina is swapped
and both positions hold 12 ticks later (read back from the game); aiming up at a box in the air swaps Lina up and
the box down (showcase `media/swap-up.gif`, also `media/swap-box.gif`); firing into the empty sky swaps nothing but
uses the shot; `walls_block` in both directions (the platform under Lina blocks a downward shot at a box below it;
with `walls_block = false` the same shot swaps); the swap checked by the `trace-positions` fixture alone.

## tumble (levels)

A level pack, **Tumble**, built in code with `openlina_sdk::levels`. "Tumble 1" is a drum: a ring of `segments`
(14) long tiles of `radius` (130) around the screen center, each turned to the tangent, one left out as a gap, plus
two shelves, a box and the fruit (`levels/tumble.toml` + the generated ring). During the last `turn_ticks` (180) of
every `period` (1200 ticks = 10 s) the drum makes a quarter turn clockwise: every static body is rotated around the
center, and whatever is loose inside tumbles and may fall out through the gap. Only while a Tumble level is played.

The pack shows up with the custom level packs (a "downloaded" pack named Tumble, creator openlina-kit); nothing is
written to `userdata`. Tests: loads and turns; turn count and timing with short periods (showcase
`media/turn.gif`); other levels don't turn.

Options:

<!-- options:tumble -->
| option | type | default | description |
|---|---|---|---|
| `trace` | bool | `false` | Print what the mod does to stdout. |
| `period` | int | `1200` | Ticks between quarter turns (120 ticks per second). |
| `turn_ticks` | int | `180` | How long a quarter turn takes, in ticks. |
| `segments` | int | `14` | Tiles in the ring (one of them is left out as the gap). |
| `radius` | float | `130.0` | Radius of the ring. |
<!-- /options -->

## solid-edges (modifiers)

A modifier (key `solid-edges`, HUD icon from `art/modifier.toml`); with `always = true` it applies in every level.
In levels that roll it, the screen border is a wall: every tick, objects crossing the visible play field are pushed back inside and
bounce (`bounce` 0.5 of the speed kept; slower than `rest_speed` 40 they stop; `friction` 0.9 of the speed along
the border kept per tick of contact). Player, frogs and fruits (unless `coins`) stay vanilla; long levels keep x
open; objects far off-screen stay vanilla. With screen-wrap in the pack: two rolled modifiers never meet; an
`always` one steps aside in levels that roll the other; both `always` is declared as a `[[conflict]]` in its
mod.toml, so builds, `openlina set` and the website refuse it with the reason. Tests: 6 scenarios, 3 of
them with screen-wrap. Idea by a friend of the project.

Options:

<!-- options:solid-edges -->
| option | type | default | description |
|---|---|---|---|
| `always` | bool | `false` | Apply in every level instead of as a rolled modifier (then it steps aside in levels that roll screen-wrap). |
| `bounce` | float | `0.5` | Share of the speed kept when bouncing off the border (0 = stop dead, 1 = perfectly elastic). |
| `friction` | float | `0.9` | Speed along the border kept per tick of contact (1 = frictionless sliding). |
| `rest_speed` | float | `40.0` | Impacts slower than this stop instead of bouncing (avoids jitter while resting on the border). |
| `coins` | bool | `false` | Fruits bounce too. Pushing fruits out is how levels are won, so this makes levels unwinnable. |
| `trace` | bool | `false` | Print every real bounce (not resting contact) to stdout. |
<!-- /options -->

## moon-gravity (modifiers)

A modifier (key `moon-gravity`, id 4779, HUD icon from `art/modifier.toml`, also in "dx" runs); with `always = true`
it applies in every level instead (no icon then). In levels that roll it, gravity is `factor` (0.5) of vanilla:
boxes, fruits and Lina fall slower, and Lina's jump goes about twice as high (45 instead of 22 units) and lasts twice
as long. It scales the game's own gravity strength (`g` in the "gravity" group of `EvSheet_gameplay.update`, see
[game/world.md](game/world.md#gravity)) once per tick, so vanilla's low-gravity objects, flipped gravity and
no-gravity zones keep working on top of it. Left alone: water and `z_up` lift, Lina while digging, `secondary_physics`
objects. Combines with every other modifier mod (a level has one modifier; an `always` moon gravity stacks with any).

Source: `mods/moon-gravity/src/main.rs`. Requires `core`.

<!-- options:moon-gravity -->
| option | type | default | description |
|---|---|---|---|
| `factor` | float | `0.5` | Gravity in moon levels, as a share of vanilla gravity (0 = floating, 1 = vanilla; up to 4). |
| `always` | bool | `false` | Apply in every level instead of as a rolled modifier (and on other screens with gameplay physics, e.g. the tutorial; no HUD icon then). |
| `trace` | bool | `false` | Print `[moon-gravity] <layout> tick 1: gravity x<factor>` when a layout with moon gravity starts. |
<!-- /options -->

Tests (`mods/moon-gravity/tests/`, 6 scenarios, positions read by `trace-positions`): a box falls 32 units in 170
ticks instead of vanilla's 65; without the modifier the box and a jump match vanilla exactly and nothing is printed;
Lina's jump peaks 45 units up at tick 200 where vanilla has her landing (showcase `media/moon-jump.gif`); `always`
without the modifier; `factor = 0.25` (16 units); the game's roll draws it (seed 12) and the HUD plays its icon.

## ray-crosshair (general)

A utility for item mods: while Lina holds an item that asks for it, a small ring marks where her aim meets the next
surface, the first tile, wall or object along the direction of the game's reticle within the item's range (the
reticle itself stays: it shows the direction). Item mods opt in with `openlina_sdk::aim::show_crosshair(code, item,
range)` and `requires = ["core", "ray-crosshair"]`, and fire with the same `aim::AimRay`, so they act exactly where
the marker is: swap and portal-gun do. With nothing in range, or another item selected, the marker is hidden. Only
the first player gets one.

How it works: it defines the number hook `ray_crosshair_range(slot) -> F64` (item mods return their range for their
item). On every level tick it finds the selected slot (the `b_item` whose `nr` is Lina's `item_selected`), asks the
hook, casts the ray from Lina through the reticle (`world_ray_cast`, static bodies included, Lina excluded) and
moves its marker (a `Sprite15`) there, half its size back along the ray: level objects are drawn over sprites that
overlap them. Showcase `media/marker.gif`.

Source: `mods/ray-crosshair/src/main.rs`. Requires `core`. Tests: `mods/ray-crosshair/tests/` (the marker in front
of the vine, by position; hidden for other items and back; on the floor when aiming down, hidden for the sky; the
showcase). `trace` also prints the marker as `[pos] tick T ray-crosshair x y` for `position` expectations.

<!-- options:ray-crosshair -->
| option | type | default | description |
|---|---|---|---|
| `trace` | bool | `false` | Print what the aim hits every `trace_every` ticks (`[ray-crosshair] tick T: aim hits <type> at (x, y)`, or `marker hidden`) and the marker as `[pos] tick T ray-crosshair x y` (for `position` expectations). |
| `trace_every` | int | `30` | With trace: print every this many level ticks. |
<!-- /options -->

## mod-menu (general)

Adds `OPENLINA MODS (n)` to the pause menu, after the game's MODDING MENU: a submenu listing every mod of the pack
(except dev mods) with its version and, with `show_options`, its option values. It learns the pack from
`openlina_sdk::runner::pack_info` (the host passes every mod the resolved pack).

Options:

<!-- options:mod-menu -->
| option | type | default | description |
|---|---|---|---|
| `show_options` | bool | `true` | List each mod's option values under it. |
| `trace` | bool | `false` | Print the entries it adds to stdout. |
<!-- /options -->

## screen-wrap

A **modifier**: in levels that roll it, objects that leave the screen reappear on the opposite side, keeping their
velocity. It is registered with `openlina_sdk::modifiers` (key `screen-wrap`, id **8900**), drawn by the game's own
modifier roll like the vanilla modifiers (also in "dx" runs), and shown in the HUD with its own icon
(`assets/images/openlina/screen-wrap.png`, drawn from `art/modifier.toml`). With `always = true` it applies in every
level instead. In long levels and boss arenas (`bossMode`), which scroll horizontally, only the top and bottom wrap,
the same way vanilla only tests those edges there.

With `player = true` Lina wraps too: instead of `player_death` at the screen edge (a pit, a side), she comes back on
the opposite side (showcase `media/player-wraps.gif`). The hub (layout `help`) stays vanilla, since walking off its
right edge starts a run; her other deaths (explosions) stay as well.

Source: `mods/screen-wrap/src/main.rs`. Requires `core`. Tests: `mods/screen-wrap/tests/` (9 scenarios).

<!-- options:screen-wrap -->
| option | type | default | description |
|---|---|---|---|
| `always` | bool | `false` | Apply in every level instead of as a rolled modifier. |
| `coins` | bool | `false` | Also wrap fruits. Pushing fruits out is how levels are won, so this makes levels unwinnable. |
| `player` | bool | `false` | Also wrap Lina: leaving the screen (falling into a pit, walking off a side) brings her back on the opposite side instead of losing the level. Her other deaths stay. |
| `secondary` | bool | `true` | Also wrap secondary physics objects. |
| `max_overshoot` | float | `200.0` | Only wrap objects at most this far past the edge. Physics objects move at most 100 units per tick (the Box2D speed cap, observed in game), so keep this above 100. Parked objects sit about 1000 out. |
| `min_tick` | int | `5` | Don't wrap during the first ticks of a layout, when levels delete objects placed off-screen. |
| `trace` | bool | `false` | Print `[screen-wrap] tick T kind K (x, y) -> (x', y')` for every wrap. |
<!-- /options -->

What stays vanilla:
- The player still dies at the edge.
- Frogs still count as "landed" and disappear.
- Objects the game parks far off-screen (e.g. at −1000, −1000), or places off-screen in level data and deletes
  on the first ticks, are still destroyed. That's what `max_overshoot` and `min_tick` are for; without them,
  junk would drop into levels.

How it works: it subscribes a handler to the core `edge_exit` hook (and, with `player`, to `player_edge`). The handler moves the object by exactly one
play-field size and returns true, so an object that just crossed the bottom limit reappears just past the top
limit and keeps falling into view. See [game/world.md](game/world.md#screen-and-edges) for the vanilla
edge test.

Verified in game (all by `lina test`): the game's roll draws it (seed 11); with the modifier a spawned box keeps
wrapping; without it the box reaches the edge and is deleted; fruits stay vanilla unless `coins`; `always` works
without the modifier. The HUD shows the icon.

## cannons (general)

Every level gets `count` (3) of the game's own cannons (`cannon_base`, created with `Layout.createObject` like level
objects, which adds the barrel and face parts). At layout tick `tick` (10) of every gameplay layout but the hub, title
screens and tool selection (`skip_layouts`), the span [`x_min`, `x_max`] is cut into `count` bands and one cannon
appears at a random x in each, at height `y` (40), and falls onto what is below. Positions closer than
`player_distance` (120) to Lina are rolled again, so no cannon drops onto her and shoots point-blank. The cannons
behave like vanilla ones: asleep until Lina makes noise within 111 units, then they aim, charge and fire explosive
`heavy_shot`s; with `awake = true` they start awake (showcase `media/awake-cannons.gif`). A retry after a death
reloads the level, and the cannons come back once.

Source: `mods/cannons/src/main.rs`. Requires `core`. Tests: `mods/cannons/tests/` (8 scenarios: three per level and
the `count` option, `player_distance` both ways, none on the hub or tool selection, asleep vs `awake`, retries
without duplicates and the trace's counter they rely on).

<!-- options:cannons -->
| option | type | default | description |
|---|---|---|---|
| `count` | int | `3` | Cannons added to every level (0 to 20). |
| `awake` | bool | `false` | The cannons start awake and shoot right away. Off: like vanilla cannons, they sleep until Lina lands or bumps into something within 111 units of them. |
| `player_distance` | float | `120.0` | Keep cannons at least this far from Lina (horizontally) when they appear, so none drops onto her and shoots point-blank. |
| `tick` | int | `10` | Layout tick at which the cannons appear (120 ticks per second). |
| `y` | float | `40.0` | Height at which the cannons appear (0 = top of the 338 high play field); they fall onto whatever is below. |
| `x_min` | float | `60.0` | Left end of the span the cannons are spread over (the play field is 600 wide). |
| `x_max` | float | `540.0` | Right end of the span. It is cut into `count` equal bands, one cannon at a random x in each. |
| `skip_layouts` | list | `["help", "main", "first_screen", "manager"]` | Layouts that run the gameplay sheet but are not levels: the hub, title screens and tool selection. |
| `trace` | bool | `false` | Print every cannon placed (`[cannons] <layout> tick T: cannon i at (x, y)`). |
<!-- /options -->

## ammo-boost

Every item the player gets starts with `factor` times its vanilla ammo (`baseAmmo`): the item slots
(`OClass_b_item`) and item blocks placed in levels (`OClass_item_block`), including state restores
(`StateSerializer.fromBin`). Ammo use, refunds and the slot/item-block swap stay vanilla.

Source: `mods/ammo-boost/src/main.rs`. Requires `core`.

<!-- options:ammo-boost -->
| option | type | default | description |
|---|---|---|---|
| `factor` | int | `2` | Multiply the starting ammo of every item by this (0 to 1000). Items without ammo stay empty. |
| `trace` | bool | `false` | Print every boosted value (`[ammo-boost] <item>: ammo 2 -> 4`) and, on tick 1 of every gameplay layout, the slots' real ammo compared with `baseAmmo` (`level start slot 0: frog ammo 6 = 2 x 3`). |
<!-- /options -->

How it works: the game always copies ammo as `Field r = type.baseAmmo; SetField holder.ammo = r`
(11 places in build 22056877); the mod inserts `r = boost(r, type)` between the two ops.

Not boosted: ammo that doesn't come from `baseAmmo` (level-authored `test_item` objects, the `main`
layout's preset slots). Written from the docs alone by an agent with no other context (the docs test).

## inspect (dev)

Prints parts of the game's state at chosen moments: `[inspect] <moment> <path> = <value>`. `at`: `tick:N` (level
tick N) or `layout:<name>@N` (the first frame of that layout at or after its tick N, for screens without gameplay
ticks such as the MANAGER tool selection). `print`: paths from `Main.i` (`game.itemManager.pickedItems[].type.name`),
`@<type>` for the main layout's objects of a type (`@item_icon[].sprite.anim`) or `$<class>.<static>`
(`$fish.game.oclass.OClass_item_icon._animData`); `[]` walks an object array, `[3]` picks one element. Fields are
read typed where the type is known (a superclass is cast to the one subclass that has the field) and dynamically
otherwise; a field no known type has fails the build. `lina probe` writes and runs such a scenario from the
command line.

Options:

<!-- options:inspect -->
| option | type | default | description |
|---|---|---|---|
| `at` | list | `["tick:1"]` | When: `tick:N` (level tick N), `layout:<name>@N` (the first frame of layout <name> at or after its tick N; `layout:<name>` for its start). |
| `print` | list | `[]` | What: paths from `Main.i` (`game.levelManager.currentLevel.type.name`), `@<type>` for a layout's objects of that type (`@item_icon[].sprite.anim`) or `$<class>.<static>` (`$fish.game.oclass.OClass_item_icon._animData`). `[]` goes through every element of an array, `[3]` picks one. |
<!-- /options -->

## record (dev)

Prints `[record] level <name> modifier <m>` and `[record] slot k <item> <ammo>` at tick 1 of every gameplay
layout and `[record] tick T bits B` whenever player 1's input changes. `lina run --record` plays with it and
turns every level attempt into a replay scenario (`work/recordings/`: `level`, `modifier`, harness `slots`,
`inputs`); `lina recordings <log>` does the same for any log with those lines. A recorded and replayed harness run
gives identical positions; a human session replays as long as the level has no random elements (the harness seeds
the run itself).

<!-- options:record -->
No options.
<!-- /options -->

## trace-calls (debug)

Prints `[trace-calls] <function> call #N (<arguments>)  (fn@<findex> <file:line>)` to stdout: the first `first`
calls, then every `every`-th. Arguments (`args`, default on): numbers and booleans as values, strings as text,
objects as their static type; dynamic values, closures and refs as `_`. Functions are `pkg.Class.method`, `Class.method`, a findex, an injected function (`swap/use`) or
`hook:<name>` for a core hook.

```toml
[[mod]]
id = "trace-calls"
options = { functions = ["fish.game.evsheet.EvSheet_gameplay.update", "3751"], first = 3, every = 600 }
```

Options:

<!-- options:trace-calls -->
| option | type | default | description |
|---|---|---|---|
| `functions` | list | `[]` | Functions to trace: `pkg.Class.method`, `Class.method`, a findex, an injected function (`swap/use`), or `hook:<name>` for a core hook. |
| `first` | int | `3` | Print the first N calls of each function. |
| `every` | int | `600` | Then print every N-th call (0 = never). |
| `args` | bool | `true` | Also print each call's arguments (numbers, strings, objects as their class). |
<!-- /options -->

## debug-spawn (debug)

Subscribes to `tick`. Spawns an object at layout tick `tick` of every gameplay layout: `object` (default `"box"`) at (`x`, `y`)
(default 300, 60) on layer `layer`. It's a test fixture: with `screen-wrap`, the box falls through the floor
and wraps forever.

Several objects: `spawns = ["box@60:300,60", "s_ball@90:420,40"]` (`<type>@<tick>:<x>,<y>`, layer 0) instead of
the single-object options.

Options:

<!-- options:debug-spawn -->
| option | type | default | description |
|---|---|---|---|
| `object` | string | `"box"` | Object type to create (see fish.game.oclass.OClass_*). |
| `tick` | int | `60` | Layout tick at which to spawn. |
| `x` | float | `300.0` | x position (the play field is 600 x 338). |
| `y` | float | `60.0` | y position. |
| `layer` | int | `0` | Layer index. |
| `spawns` | list | `[]` | Several objects instead: ["<type>@<tick>:<x>,<y>", ...] (layer 0). Overrides object/tick/x/y/layer. |
<!-- /options -->

## trace-positions (dev)

Test fixture: at every tick in `ticks` (`"60-600/60"`, `"130"`, `"59,72,200-260/20"`), prints
`[pos] tick <t> <type> <x> <y>` for each object in `physics_obj` whose type is in `types` (default `["player"]`,
`["*"]` for all). Scenarios check it with `[[expect]] position = { tick, type, x, y, within, away }`, so a test
can see where things are without relying on the mod under test to print it (see `mods/swap/tests/swap-positions.toml`).

Options:

<!-- options:trace-positions -->
| option | type | default | description |
|---|---|---|---|
| `types` | list | `["player"]` | Object types to report (the `type` of physics objects: player, box, coin, frog, …); ["*"] for all. |
| `ticks` | string | `"60-600/60"` | Layout ticks: `from-to/step`, a single tick, or several separated by commas (e.g. "118,130,200-260/20"). |
<!-- /options -->

## harness (dev)

Drives the game for tests and recordings without input (source: `mods/harness/src/main.rs`):

<!-- options:harness -->
| option | type | default | description |
|---|---|---|---|
| `start_tick` | int | `5` | title-screen tick at which to skip to the game (like a key press) |
| `level` | string | `""` | Level to load, by name (see list_levels). Empty: keep the run's first level (the tutorial). |
| `level_n` | int | `-1` | Level number within `level`'s pack (-1: any). |
| `seed` | int | `1` | Seed for the level roll and every RNG of the run (items, modifiers, colors, ...), so runs are reproducible. |
| `modifier` | int | `-1` | Force this modifier id (-1: rolled). |
| `roll_modifier` | bool | `false` | Let the game draw a random modifier for the level (seeded), as it does in runs. |
| `modifier_key` | string | `""` | Force the modifier registered with this key (e.g. "screen-wrap"); overrides `modifier`. |
| `items` | list | `[]` | Items to roll from (up to 3, repeated if fewer; names from list_levels). The game assigns them with its own code (ammo included, so ammo mods apply); slot order follows the seed. Items marked secondLayer only appear in second-layer runs. |
| `inputs` | list | `[]` | Scripted inputs in level ticks: "60-90:right+jump" or "120:shoot". Actions: up down left right jump shoot switch restart. While set, the keyboard is ignored. |
| `end_tick` | int | `0` | Exit the game (code 0) at this level tick. 0: never. |
| `capture` | string | `""` | Capture frames "from-to/step" in level ticks, e.g. "60-300/2". |
| `capture_dir` | string | `"frames"` | Directory for captured frames (absolute, or relative to the overlay game dir). |
| `pause_tick` | int | `0` | Open the pause menu at this level tick (0: never). |
| `dump_menu` | bool | `false` | With pause_tick: print every pause menu item (including hidden submenu items). |
| `menu_open` | string | `""` | With pause_tick: two ticks later, select the pause menu item with exactly this text and press it (e.g. to open a submenu). |
| `capture_ui` | bool | `false` | Also draw the UI layer (pause menu, ...) into captured frames. |
| `list_levels` | bool | `false` | Print all levels in the pool and all items in the item pool, then exit. |
| `new_run` | bool | `false` | a returning player's run from the hub (tutorial done, RNGs seeded): the game's own run start with the MANAGER tool selection follows (`inputs` like `"30-700:right"` walk off the hub's right edge, `jump` confirms). The selection runs no gameplay ticks, so `capture`/`end_tick` also count layout ticks there |
| `new_run_items` | list | `[]` | With new_run: put these items (by name) into the first slots of the run start's tool roll. |
| `turbo` | int | `16` | Extra game steps per rendered frame (0 = real time). Steps keep their fixed length, so results don't change, only the wall-clock time. |
| `heartbeat` | int | `1200` | Print `[harness] layout <name> tick <t>` when the main layout changes and every this many of its ticks (0 = off), so a hang's log shows where it stopped. |
| `chaos` | bool | `false` | From tick 60 of every layout but the title, when no scripted input is active: random inputs (seeded, new every 15 ticks; never restart). Random play in levels, wandering into new runs in the hub. For soak tests. |
| `end_total` | int | `0` | Exit after this many game steps in total, on any screen (0 = off). For soak runs through deaths and new runs. |
| `roll_until_modifier` | string | `""` | Roll with the game's own code (up to 500 seeds) until it draws this modifier (key or id); prints `[harness] modifier <id> drawn by the game's roll after N roll(s)`. Tests a modifier is in the pool without a fixed seed. |
| `slots` | list | `[]` | Exactly these tools in the slots at level tick 1: `["box", "swap:2"]` (name, optional ammo). Replays of recordings use it; `items` rolls instead. |
<!-- /options -->

It prints `[harness] …` lines (`title skipped`, `loading level …`, `level tick 1: <name> modifier <m> frameTime <dt>`,
`end at tick N`, `ERROR …`), which `lina test` checks. Use it through scenarios (`lina test`, `lina gif`), see
AGENTS.md and docs/testing.md.
