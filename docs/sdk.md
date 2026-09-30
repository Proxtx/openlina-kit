# The SDK (`tools/sdk`, crate `openlina_sdk`)

## Library map

| module | purpose |
|---|---|
| `Code` (lib.rs) | load/save, `class`, `field`, `field_type`, `method`, `find_fn` (any name people write: `Class.method`, findex, `swap/use`, `hook:tick`), `native`, `func(_mut)`, `func_type`, `func_name`, `op_location`, interning (`string`, `float`, `int`, `intern_type`, `ty_*`), `add_global`, `global_string` |
| `asm::FnBuilder` | new functions: registers (`reg`, `reg_f64`/`reg_i32`/`reg_bool`), labels, jumps, constants, `get`/`get_new`/`set` fields, `call`/`call_new`, `static_obj`, `new_obj`, `cast`, `string_obj`, `string_of`, `print`, arrays (`array_len`, `array_get`, `new_array_obj`, `empty_f64_array`, `for_range`), `jstr_ne`, `exit`, globals (`get_global`, `set_global`, `clear_global`), `static_closure` |
| `edit` | `find*`, `find_calls`, `find_field_access`, `expect_one`, `next_match`/`prev_match`, `replace_op`, `insert_ops`, `insert_ops_with_exits`, `guard_op`, `prepend_call`, `remove_ops`, `add_reg` |
| `hooks` | `CORE_HOOKS`, `find`, `signature`, `handler`, `subscribe`, `define` (`lina hooks` lists them) |
| `items` | `register` an item (see the checklist below), `is_item`, `crosshair_pos`; behavior in an `item_use` handler |
| `modifiers` | `register` a modifier (pool + HUD icon), `is_active`, `id_of`, `current_modifier` |
| `levels` | `LevelPack::from_toml` + `register`: level packs built in code (core `packs` hook); `current_level_name`, `jump_unless_in_pack` |
| `text` | `set(key, value)`: texts the game looks up (`Localisation.loc`), e.g. `TOOL_<NAME>` item labels |
| `anims` | `ensure` / `ensure_frames`: an animation from mod images in an object class's animation map |
| `physics` | `RayCast`: the nearest object on a line (Box2D `world_ray_cast`), e.g. line of sight |
| `runner` | `run_mod` (the `main` of every mod), `pack_info` (every mod of the pack with its options) |
| `manifest` | `ModManifest` (mod.toml), `ModPack` (modpack.toml), `resolve_order` |
| `validate` | `check_function`, `check_touched`, `kind` |
| `caps` | `Snapshot`, `diff`: what a patch makes the game able to do outside the game |

Read the module docs (`//!`) of the one you use; `mods/` has a showcase mod for each registry.

## Registries: every place the game lists the thing

A registry is only complete when it covers every place the game enumerates that kind of thing. Missing one shows
up far from the mod: a mod item without an item object hung the run start's tool selection. Before adding to a
registry, and when a mod "works in tests but not in the real game", go through its list. Unverified entries say
so; confirm them with `lina probe` and update this list.

**Items** (`items::register` covers the checked ones):

- [x] `itemManager.itemPool` entry (`item_pool` hook, after `initBaseItems`): rolls, raw random, level loads
- [x] a global `OClass_item` object (`ev_instancing_ev.item`, 12-column grid): the MANAGER tool selection (grid,
      unlock animation by `NAME`), the editor's tool override list
- [x] `OClass_item_icon` animation named after the item, frame 0 (14×14 HUD) and frame 1 (28×28: MANAGER grid,
      editor); `big_icon`
- [x] label `TOOL_<NAME>`
- [x] firing: `item_use` hook in `EvSheet_gameplay.shoot`
- [x] ammo: `baseAmmo`, copied in 11 places (see game-internals "Ammo")
- [ ] item blocks in levels (`mld.objects.ItemBlock.createInstance`, `findItem`): should work through the pool,
      untested
- [ ] level tool overrides (`mld.Level.createLayout`, `findItem`): should work, untested
- [ ] saved runs (`StateSerializer` / `ItemManager.writeBin`/`readBin`): what happens when a saved run holds a mod
      item and the mod is gone is unknown
- [ ] stats (`Wins`, `picked`, `drawn` on the item object; persisted?): unknown for mod items

**Modifiers** (`modifiers::register`):

- [x] the roll pools (`LevelManager.rollRaw` / `reroll`, `modifier_pool` hook), "dx" pool via `in_dx`
- [x] HUD icon (`modifier_icon` hook)
- [ ] co-op draws `coopModifier` from its own pool (`[1,2]`): mod modifiers are not in it
- [ ] other lists (stats, pause menu, saved runs): unknown

**Levels** (`levels::LevelPack::register`): a downloaded-style pack built in code at the core `packs` hook; shows
with the custom level packs. Workshop pages, level editor and saved progress are not covered.

## HashLink pitfalls (learned the hard way)

- The **`String` opcode yields raw UTF-16 `hl.Bytes`**, not a `String` object. Use `FnBuilder::string_obj`.
- The game keeps most string literals in globals (`GetGlobal … // "text"`), not `String` ops: search with
  `lina refs`, not `lina strings`.
- **String pool index 0 is `"String"`**, but hlbc's `Resolve` prints index 0 as `<none>`. Use `Code::str`.
- Jumps are relative: `target = pos + 1 + offset`. Backward jump targets must be `Label` ops. `Switch.end` marks
  the end of the switch block and may equal `ops.len()`; the default case falls through.
- Free functions have no name in the bytecode. Injected ones are recognized by their debug file
  (`openlina/<name>`); hooks by `openlina/hook/<name>`.
- A call's `dst` register must match the callee's return kind, or be `Void` to discard the result.
- hlbc's private findex tables are only rebuilt on load. After adding functions, use `Code::func`/`func_type`
  (linear search), not hlbc's `code.get(RefFun)`.
- `f.reg(f.code().ty_f64())` doesn't borrow-check: use `f.reg_f64()` (or take the type first).
- Strings compare by value with `JEq`/`JNotEq` (the game does this); `jstr_ne` compares with a literal.
- Dynamic field reads (`DynGet`) work on virtuals and anonymous objects; on class instances prefer typed reads
  (`get_new`, after a `cast` to the class).
- Only `Mosa Lina_jit` runs bytecode. It needs its resources relative to the working directory and the game dir on
  `LD_LIBRARY_PATH` (`lina run` / `openlina run` do this).
- `Main.mainLoop` runs once per rendered frame on every screen; `fish.system.Game.update` once per game step;
  `EvSheet_gameplay.update` (the core `tick` hook) only in gameplay layouts; `EvSheet_manager_ev.update` not in the
  hub.
- Setting `sprite.position` teleports physics objects (`Physics.syncPosWithSprite`).
- The game parks objects off-screen and deletes objects placed off-screen in the first ticks of a layout.
  Anything that changes edge or destroy behavior needs to account for that.
- `Layout.createObject(layout, type, …)` only creates types registered in `ObjectClasses.createInstance`
  (not plain `Sprite`; `Sprite15`/`Sprite21` are plain decorative types that work).
- An object class's animation map (`$OClass_x._animData`) is built when its first instance is created; adding to
  it earlier makes the game skip its own animations. `anims::ensure` waits for it. Some objects show a fixed frame
  (item icons: frame 1 in menus), so give every frame they use.
- Mods run as separate processes on the whole bytecode: a mod cannot see another mod's Rust state, only what the
  previous mods left in the bytecode (hooks, debug file names) and `runner::pack_info`.
