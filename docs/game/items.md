# Items (tools)

Written against Steam build 22056877 (`hlboot.dat` sha256 `57afd61a…d01c`). Line numbers refer to the original Haxe
sources listed in the debug info; findexes (`fn@N`) refer to this build. Index: [../game-internals.md](../game-internals.md).

- Items: `itemManager.itemPool` holds 48 item types `{aimType, baseAmmo, name, secondLayer}` (`box`, `bomb`,
  `phaser`, `rocket`, …); `itemManager.currentItems[k]` is `{locked, type, wins}` and is shared with the HUD slot
  `ev_manager_ev.b_item.insts[k].item` (`OClass_b_item`, which also has `ammo`).
- **Ammo** (found by the ammo-boost docs test): the game copies ammo with the two ops
  `Field r = type.baseAmmo; SetField holder.ammo = r` in 11 places: `EvSheet_manager_ev.manage` and `rollItemsRaw`
  (HUD slots), 3 `EvSheet_instancing_ev` closures, item blocks (`EvSheet_gameplay` closure ~L6147,
  `mld.objects.ItemBlock.createInstance`) and `StateSerializer.fromBin`. Shooting does `ammo - 1`, returning an item
  `ammo + 1`; taking an item block swaps ammo between the slot and the block (closure ~L17390).
  `OClass_test_item` objects copy their own ammo into slot 0 (title screen), and the `main` layout's preset slots
  get theirs from layout data. The HUD shows ammo as `ammoSprite.animFrame = ammo + 1`.
- **Firing** (`EvSheet_gameplay.shoot(playerId)`) runs a closure over the item slots (`foreach(b_item, …)`, source
  L1140-3720): it finds the player's selected slot, does `if (ammo > 0) ammo--`, then dispatches on the item name
  (`if (slot.item.type.name == "unbox") … else if …`, comparing against the game's string constants). Its
  environment enum holds the sheet, the player picker and the player's aim pickers (crosshair_point, gun, …,
  max_aim_point, mid_aim_point, short_aim_point). The crosshair is the reticle just in front of Lina.
- **The selected item**: the HUD slot (`b_item`, picker `sheet.b_item`) whose `nr` (1..4, F64) equals the player's
  `item_selected` (found by the ray-crosshair mod; `switch` moves to the next slot).
- Item types come from `OClass_item` objects (`NAME`, `ammo`, `aim_type`) via `ItemManager.initBaseItems`; aim
  types: shoot 0, short 1, short2 2, mid 3, mid2 4, long 5, long2 6, remote 7. The HUD icon is the `item_icon`
  object's animation named after the item; the HUD label is the text `TOOL_<NAME>` (from `loc.dat`, an xlsx).
- The roll draws 3 items and the 4th slot copies the 2nd (`rollItemsRaw` crashes if the pool has fewer than 2).
- **Item objects**: every item type also has a global `OClass_item` object (`ev_instancing_ev.item`, 48, invisible,
  `NAME`/`ammo`/`aim_type`/`unlocked`), laid out as a 12-column grid from (150, 140), 30 apart. `initBaseItems`
  builds `itemPool` from them at startup. The MANAGER screen draws its tool grid at their positions (offset −13, −18)
  and its unlock animation looks them up by `NAME`; the editor's tool override list (`editor.ItemOverride.init`) is
  built from them too. An item in the pool without an object hangs the run start once the roll draws it, so
  `openlina_sdk::items::register` adds one (the 49th goes to (150, 260)).
- **Item icons** (`OClass_item_icon` animations named after the item) have two frames: 0 is the 14×14 HUD icon, 1 a
  28×28 version the MANAGER grid (`icon_display` "unlock_show" sets `animFrame = 1`) and the editor show.
- **Run start** (a returning player walks off the hub's right edge): `EvSheet_manager_ev.manage` rolls 9 tools
  (`ItemManager.reroll`, into `pickedItems`) and plays an "unlocked_item" animation block per tool
  (`final_ani_block`), which waits for the tool's item object; jump then continues and `pick(rand, false, …)` takes
  the slots from `pickedItems`. **Raw random** (pause menu › Change › Raw random, input command 8) skips that:
  the manager layout calls `rollItemsRaw` (= `pick(rand, true, …)`, 3 draws from the whole `itemPool`, mod items
  included) and `goToNextRaw` for every level. The editor's Play button and level previews use `rollItemsRaw` too.
