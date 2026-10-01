# The play field: gameplay, edges, gravity, physics

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

## Gameplay

- The main per-tick logic is `EvSheet_gameplay.update` (fn@3767, ~22k ops, source `EvSheet_gameplay.hx`
  L9734–16300). hlbc's decompiler can't handle it. Read it with `lina fn 3767 --ops a..b` or
  `work/dump/asm/fish/game/evsheet/EvSheet_gameplay.asm`.
- On the title screen the gameplay sheet only runs for its first ticks (startup cleanup). It runs every
  tick in levels.
- **Coins are the fruits.** A level is won by collecting every fruit (`coin_fn`, fn@3732, counts `portal.coins`
  down). A fruit is collected when Lina touches it (`tryTouchCoin`, fn@3752: `state = 1`, `coin_fn`) or when it is
  deleted: pushed off the screen (`coinedgecheck`, fn@3751, destroys it) or with the delete tool. Deleting goes
  through the fruit's destroy listener (closure fn@10163 from `setupEvents`, L9490): if `state == 0` and it isn't
  glitched, `coin_fn(touched = 0)` and `state = 1`. `coin.glitched` fruits respawn at `spawnPos` instead until every
  player has touched them.
- The player loses at the screen edge (`player_death`, fn@3746). Items come from `ItemManager` and
  `EvSheet_gameplay.shoot` (fn@3739).

## Screen and edges

The play field is **600 × 338** layout units, with a black border of **25 units** (`margin`) around it. At 1080p
the scale is 3.2 (80 px of border).

The edge test in `update` (L16195–16268):

```haxe
var margin = 25;
for (p in player.insts)  // state "normal" only: player_death if outside [25,575]x[25,313] (x only !bossMode)
for (o in physics_obj.insts) {                           // L16219
    if (o.sprite.destroyed || (isStatic(o.physics.body) && layout.currentTick != 0)) continue;
    var pos = o.sprite.position, ew = o.edgewith;        // per-object slack
    if (pos.y > 338 - margin + ew || pos.y < margin - ew
        || (!bossMode && (pos.x < margin - ew || pos.x > 600 - margin + ew)))
        switch (o.type) {
            case "coin":   coinedgecheck(o);             // deletes the fruit, which collects it
            case "frog":   if (alive) frogland_count++; destroy;   // counted for "greenfrogs 1"; a steered frog: lost
            case "player": // handled above
            default:       o.sprite.destroy();
        }
}
for (o in secondary_physics.insts)                       // same test, no edgewith, always destroy
```

- The player test (L16200-16213) has no `edgewith`. It also runs on the hub (layout `help`), where walking off the
  right edge is how a run starts, so mods that keep Lina on screen must leave the hub alone (screen-wrap `player`).
  `update` calls `player_death` once more, for explosions (`setToExplode`, L10403).
- `bossMode = currentLevel.type.isLong` when a level starts (closure fn@9188). It is also set by `prepare_boss`.
  It's true for boss arenas and for custom levels with `mld.Level.longMode`: levels wider than the screen that
  scroll horizontally. Those levels skip the left/right test.
- The game **parks objects far off-screen** (e.g. at −1000, −1000) and places objects just below the screen in
  level data, relying on the edge test to delete them (first ticks of a layout). Mods that change edge
  behavior need to leave those alone.

## Gravity

Found by the moon-gravity mod. The Box2D world is created with gravity (0, 10) (`Layout.initialise`), but gameplay
layouts set it to 0 when they start (closure fn@9255 from `EvSheet_gameplay.setupEvents`, L6086); gravity is the
game's own. The "gravity" event group of `EvSheet_gameplay.update` (L16413-16472, ops ~15971-16130) does, every tick:

```haxe
var g = 1.05;
if (low_grav.insts.length != 0) g = 0.05;               // any OClass_low_grav object: low gravity
for (o in physics_obj.insts) {                           // dynamic bodies only (body_get_type != 0)
    var water = o.is_overlap_water ? 0.4 : 1, lift = o.is_overlap_water ? -0.03 : 0;
    var f = (o.personal_gravity * o.flipModifier * water * g + lift) * (1 - o.is_overlap_nograv);
    // + a z_up zone term: -mass * invWorldScale * is_overlap_z_up * water * 0.065
    body_apply_force(o.physics.body, (0, f), worldCenter, true);
    // water also damps the velocity by 0.95^(dt*60)
}
```

Lina is a `physics_obj` too (`personal_gravity` like the others); her jump sets a velocity, so gravity decides how
high and how long she flies. A digging player's fall speed is integrated separately (L16522, `personal_gravity *
14.76 * dt`). On greendemo 1 in vanilla, a box spawned in the air at (540, 60) is at y 124.7 170 ticks later (there is
noticeable damping), and a jump from the ground (y 193.9) peaks 22 units up after 40 ticks.

## Cannons

Found by the cannons mod. A cannon is a `cannon_base` (dynamic physics body, fields `awake`, `power`,
`timetoshoot`, `mortar`, `state`); `Layout.createObject("cannon_base", …)` also creates its container parts
`cannon_rohr` (the barrel, with a Line of Sight behavior), `spr_cannon_face`, `spr_cannon_nody`, `spr_cannonrohr`.
Cannons sleep (`awake = 0`). When a player makes an impact sound (`sfx_impact_cd`, update L10250-10253),
`EvSheet_gameplay.stealth(x, y)` (fn@3748, L4086) sets `awake = 1` on every cannon within 111 units of her. Awake
cannons (closure at L17084-17160) turn the barrel towards the nearest player, charge `power += 0.7 * dt`, play
`cannon_prep`, and fire a `heavy_shot` (explodes) once `power > timetoshoot * 0.8`.

`Math.random` (fn@664) and `Std.random` draw from HashLink's system generator (`$Std.rnd`, natives `rnd_float` /
`rnd_int`, one call site each), not from the game's seeded `Rand`. The harness replaces both calls with its own
generator seeded from `seed`, so scenarios that use them replay.

## Physics

- **Ray casts**: the native `world_ray_cast(world: b2World, callback, from, to)` (Box2D `RayCast`) takes
  `hxmath.math.Vector2Default` points in physics units (layout units × `layout.worldScale`) and calls
  `callback(fixture: b2Fixture, point, normal, fraction: F64) -> F64` for every fixture on the segment, in no
  particular order (return -1 to ignore, the fraction to clip the ray there, 1 to go on). `fixture_get_user_data`
  gives the fixture's `ObjectClass`. The game uses it in `fish.system.beh.LOS` (Line of Sight) and in
  `EvSheet_gameplay.prepare_boss`. A static closure (`StaticClosure` op) of a new function with exactly that
  signature works as the callback; keep state in globals.
- Static level geometry has `physics.immovable = true` (tiles such as `tile_short`); boxes, fruits and vines don't.
- `Physics.syncPosWithSprite` passes both `sprite.position` and `sprite.angle` to the body (`body_set_transform`),
  static bodies included: moving static tiles every tick makes moving geometry that pushes loose objects.
