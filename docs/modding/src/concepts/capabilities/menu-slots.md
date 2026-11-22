# Menu slots

## Backing indices and screen slots

A menu's slot indices belong to the menu, not the window. Declare the slots the menu exposes
in the menu type itself, and enforce server-side placement or pickup restrictions through
`MenuPart::menu_slot_rules`. In the
[screen factory](../../how-to/world/machine-menu-data.md#the-menus-screen), each entry in
`MenuScreenLayout::slots` corresponds to its backing index; `None` (including omitted trailing
entries) marks a slot that starts inactive/hidden (`Slot.isActive()`).

For a mod menu backed by `ItemPlace`, pickup and quick move use these same
rules. A display-only backing slot should refuse both `may_place` and
`may_pickup`; a direct write to the backing `ItemPlace` still works. Hiding a
slot in the client layout does not change what the server allows.

Any mod can move, activate, or hide a slot of the open screen for `menu` through
[`ClientHost::set_screen_slot`](../../how-to/client/screens.md#moving-or-hiding-a-mod-screens-slots)
while this opening is shown. That change affects only the client; the server can still
quick-move items into a locally hidden slot, and the slot's acceptance rules still apply.

## Mod-created recipe input sets

`Hooks::recipe_property_sets` runs once after recipes resolve for each load and again after
`/reload`. Add `RecipePropertyAddition::new("mymod:cooking_pot_input", items)` to its
`event.additions`: a key in the listener's manifest-owned namespaces, or in a namespace delegated to it through `served_by` (see [Declare content in code](../../how-to/content-mods.md#declare-content-in-code)), creates a set. Ownership
uses the package ID plus the manifest's `namespaces`, so a package named
`ported_farmersdelight` with `namespaces = ["farmersdelight"]` can create
`farmersdelight:cooking_pot_input`. A later listener may extend an existing set but cannot
create a new foreign key. Invalid keys, unknown vanilla keys and air are skipped with a
warning; unregistered items do not participate in acceptance or network sync.

Attach a `MenuPart` to the registered menu type and filter its input slot with the same set:

```rust,ignore
struct PotInput;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for PotInput {
    fn menu_slot_rules(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::MenuSlotRulesEvent<'_>,
    ) {
        if event.slot == 0 {
            event.may_place &= host.mod_recipe_property_contains(
                "farmersdelight:cooking_pot_input", event.stack.item(),
            );
        }
    }
}
// In Hooks::register:
// registrar.menu_part("farmersdelight:cooking_pot", PotInput);
```

The `Host` exposes the `mod_recipe_property_contains(set, ItemKey)` query, and `menu_slot_rules` receives it (the read-only `Host`). This query covers mod-created sets only: unknown sets, unknown
items and vanilla set keys return false. Existing vanilla station acceptance is unchanged.
The server reads its owning load; the client reads local preload sets before its first
`UpdateRecipes`, then the authoritative server generation. A packet replaces the entire
generation, including an empty one. Closing a screen or changing dimension retains it;
disconnecting discards it. Reads allocate nothing and do not rescan recipes or registries.
A client without this mod predicts vanilla here; the server corrects it.

Parts that read `context.setting(..)` or `side()` have no counterpart on the Host.

### Who and where: the slot's context

A rule that depends on more than the stack reads `event.context`, a `MenuSlotContext<'_>` lent
for this ask on either the server or the predicting client. `context.player` is the UUID of
the player whose menu it is, and `context.pos` locates the block entity the menu belongs to
(`ContainerLevelAccess`). `event.container` names the container the asked slot is in, and
`event.slot` indexes inside it. Read a live slot with `context.slot(container, index)`: use
`MenuContainer::Main` for the menu's own slots, `Grid` (row by row) and `Result` (slot 0) on a
mod menu type with a crafting grid, and `MenuContainer::Player` for the player's inventory
(`0..36` inventory, `36..40` armor feet-to-head, `40` offhand, matching vanilla `Inventory`).
Permission and capacity are separate live queries; after a quick-move transfer, the next
destination sees the new source remainder as well as the changed destination.
For example, a machine can use the view to accept only one upgrade card of each kind, or a grid
slot can check whether a result is waiting to be taken:

```rust,ignore
use mod_api::MenuContainer;

fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut mod_api::MenuSlotRulesEvent<'_>) {
    let context = event.context;
    // Upgrade slots 4..8 of the main container take one card of each kind.
    if event.container == MenuContainer::Main && (4..8).contains(&event.slot) {
        let item = event.stack.item();
        event.may_place &= !(4..8).any(|i| {
            i != event.slot
                && context.slot(MenuContainer::Main, i).is_some_and(|s| s.item() == item)
        });
    }
    // A crafting grid takes no more while a result waits to be taken.
    if event.container == MenuContainer::Grid {
        event.may_place &= context.slot(MenuContainer::Result, 0).is_none();
    }
    // `context.pos` names the block entity to read through `host`.
    let _ = (host, context.pos, context.player);
}
```

`player` is None where no player is known and `pos` None for a menu that is not a block's (one
opened with no block by `PlayerRef::open_mod_menu`, a vehicle's). `slot` converts just the requested
stack and returns it as an owned `ItemStack`. It returns `None` for an empty, absent, or
unrepresentable stack. There is no matrix to enumerate or retain. `event.max_stack` is bounded
to `0..=99` and the item's stack limit (matching vanilla `Slot.getMaxStackSize` and
`Container.getMaxStackSize`): setting `0` allows no items to be placed into the slot. Each live
`mayPlace`, `mayPickup` and `getMaxStackSize` check during a click queries the parts directly
against the current menu slots, matching vanilla `AbstractContainerMenu.doClick` and `Slot`.
Inside `menu_slot_rules`, `MenuRef::slot_holds` and `slot_gives` use the slot's own rules
without asking menu Parts again, even on another slot or menu. `context.slot` still reads
the slots of the ongoing click. Ask for an answer that includes Parts outside the callback.
If a part panics, its answers are discarded and that callback is disabled for the session.
The log and error toast name the mod; other parts still answer the check.

## Slots that keep what they show

On a mod menu type, `menu_slot_rules` also answers `Slot.mayPickup` in `event.may_pickup`. It arrives true and
is asked with the stack the slot holds, on the server and in client prediction alike. A slot that only shows
something, such as a cooking pot's preview of its result, refuses both rules: nothing can be taken from it and
nothing put in it.

```rust,ignore
struct PotPreview;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for PotPreview {
    fn menu_slot_rules(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::MenuSlotRulesEvent<'_>,
    ) {
        if event.slot == 6 {
            event.may_pickup = false;
            event.may_place = false;
        }
    }
}
// In Hooks::register:
// registrar.menu_part("mymod:cooking_pot", PotPreview);
```

Every click that would take from the slot asks it: a pickup click with either button (after
`item_stacked_on_other`, before any take, merge or exchange), a shift-click (once, before the move), a hotbar or
offhand swap that takes the slot's item, a throw (a refusal drops nothing), and a double-click's pick-all
(it gathers from the other slots when the clicked one refuses, and skips each slot that refuses). A drag
spread, a creative clone, putting a stack into an empty slot and a click outside the window never ask it; a drag
that ends on a single slot is a pickup click on that slot. A refused click leaves the slot and the cursor as they
were, and the server sends back the slots and cursor a client predicted otherwise. A slot that refuses placement
gives up all of its stack or none of it. The slot's own rule comes first, and one ask answers `may_place` and
`may_pickup` together. Vanilla menu types keep their slots' own `mayPickup`, and a shift-click a part answers
with `menu_quick_move` is asked the pickup rule before that hook. Each live check during a click
queries the rule directly against the current slots.

## Taking from a slot

`menu_slot_take` is `Slot.onTake` for a mod menu type open on your block entity: heard on the server after a player
took a stack out of one of the block's own slots, once per take. It refuses nothing (`may_pickup` above refuses); a
result slot uses it to give experience, unlock a recipe or use up its inputs, through the `Host`. `item_crafted` and
`item_smelted` stay the crafting grid's and the furnaces' events and are never heard for a mod menu.

```rust,ignore
struct PotOutput;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for PotOutput {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for PotOutput {
    fn menu_slot_take(&self, event: &mut mod_api::MenuSlotTakeEvent<'_>) {
        // Slot 6 is the result: each item taken uses up one input from slot 0.
        if event.slot == 6 {
            event.subject.with_view(|mut entity| {
                if let Some(input) = entity.item_slot(0).and_then(|s| s.to_owned()) {
                    let _ = entity.set_item_slot(0, input.shrink(event.taken.count()));
                }
            });
        }
    }
}
```

`taken` is a copy of what left the slot and `left` what the slot holds afterwards (`None` when it was emptied). A
click hands over what it took: all of the slot or half of it rounded up to an empty cursor, all of a result onto a
matching cursor, the slot's stack on a hotbar or offhand swap, each throw, and each slot a double-click's pick-all
took from (with the part taken). A shift-click hands over the amount it moved, and a shift-click on a crafting result
is heard once for each craft it repeats. An exchange of the cursor and the slot, putting a stack in, a drag, a creative
clone and a refused take are not takes. Taking from a linked container or from the player's inventory does not notify
the block behind the menu.

`menu_slot_take` (contract version 2) lends the live resident block entity and the server `Host` sequentially through
`event.subject: BlockEntityScope<'_>` (`with_view` returns `None` if the block entity was removed or replaced during
the call). Writes through `with_view` validate at the boundary and apply in place, marking the chunk dirty and firing
container updates when items change (matching vanilla `BlockEntity.setChanged`). The two take routes differ in their
transaction boundary:

- **Game-routed clicks** (pickup, swap, throw, pick-all, crafting, or a shift-click left unhandled by `menu_quick_move`):
  each take runs inside the click's enclosing transaction before the click commits, and before the next iteration (so a
  listener can refill a crafting grid and the next iteration sees it). A panicking listener is contained and disabled
  for `menu_slot_take` for the rest of the session, and the click aborts its enclosing transaction (undoing both the
  click and the listener's writes) before resending the menu.
- **Handled `menu_quick_move` shift-clicks**: `menu_quick_move` applies its remainder to the resident container slot
  and publishes `BlockEntity.setChanged` before calling `menu_slot_take`, with no enclosing click transaction. Wrap
  writes that must commit or abort together in `event.subject.transaction(...)`. A panicking listener is contained and
  disabled for `menu_slot_take` for the rest of the session; any open `event.subject.transaction(...)` aborts, live
  writes made outside a transaction survive (and still publish container updates), and the earlier quick-move
  settlement survives.

Nothing is asked when no mod listens.

## Crafted-by processing for held stacks

After delivering an item into a menu slot or onto the cursor, server-side code can use the
held `place`, the caller's transaction `tx`, and the actual crafted count `amount` (`i32`):

```rust,ignore
place.on_crafted_by(host, tx, amount)?;
```

The operation applies vanilla crafted-by processing to the held stack: it awards the crafted
statistic, applies item-specific post-processing such as a filled map's pending scale or lock,
and runs the normal stat hook. The statistic and any newly materialized map data are committed
with the caller's transaction. A stack not held in a menu slot or cursor is refused with
`CraftedByError::NotHeld`. This operation does **not** perform the separate
`recipe_crafted` criterion ([tracking](https://github.com/sunrise-org/sunrise/issues/1977)).

## Keeping a mod menu open

The server asks the open mod menu's `MenuPart` for `menu_still_valid` each tick and before processing a
matching non-spectator container click, as a Java menu overrides `AbstractContainerMenu.stillValid`. A click
for a different window is dropped; a matching spectator click resends the menu without asking the part.
In `menu_still_valid` v3, `event.menu` is the menu type. For a blockless open, `event.valid` starts
true and `event.pos` is `None`, even if the open request supplied a position for the client's screen.
Set `event.valid` to false to close the menu and return the cursor stack as on a normal close.

An item opener can set the request's `opening_hand` to `Some(hand)` regardless of what backs the slots.
`event.opening_item` then contains the `hand` and `stack_matches`: the hand holds the same item with the
same components as at open, ignoring the count. This compares stack values, not instance identity. A part
can set `event.valid = item.stack_matches` to close on a mismatch. With no supplied hand, `opening_item`
is `None`; in either case the game leaves the decision with the part.

For a block menu, `event.pos` is the backing block and `valid` starts as the reach check: the player's
eyes must be within their block interaction range plus 4 of each backing block. A large machine with a
core block deep inside it can override that answer:

```rust,ignore
struct ShellReach;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for ShellReach {
    fn menu_still_valid(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::MenuStillValidEvent<'_>,
    ) {
        // Keep the block's menu open while the player stands within 12 blocks of the core.
        let Some(pos) = event.pos else { return };
        let d: f64 = (0..3)
            .map(|k| (event.eye[k] - (f64::from(pos[k]) + 0.5)).powi(2))
            .sum();
        event.valid = d < 12.0 * 12.0;
    }
}
// In Hooks::register:
// registrar.menu_part("mymod:torus", ShellReach);
```

Vanilla menu types keep their own check. A part that leaves `valid` alone keeps the game's answer, and a menu
whose block entity is gone closes without asking. Answer from the event and `LoopContext` alone; this
callback has no `Host`.


## Reading and clicking

Client hooks can read the local player's menu with `host.local_player()?.container_menu(host)`,
including the inventory when no container is open. `MenuRef::slot_count`, `slot` and `carried`
read on either side; `slot` and `carried` return an outer `None` when the handle or slot is
unavailable and `Some(None)` for an empty slot. Each read checks the world epoch and menu generation,
so a handle cannot read a replacement, reopened or respawned menu. These methods do not click or
change slots.

Server hooks can also read the acting player's supported menu through
`player.container_menu(host)`. Inventory, equipment, cursor and ender contents come from
that player's current turn state, not the temporarily swapped-out record. The unsupported
menu kinds, permanent exclusions and stale-generation checks still apply.

Mutable client hooks can submit a vanilla click with
`menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])`. The slot is the same menu-order
index used by `slot`, not a hotbar index or a GUI slot id. For example:

```rust,ignore
{{#include ../../../examples/src/lib.rs:menu_quick_move}}
```

The typed actions are `Pickup` (with `MouseButton::Left` or `Right`), `QuickMove`,
`Swap` (with `SwapWith::Hotbar(0..=8)` or `Offhand`), `Clone`, `Throw`,
`Drag` and `PickupAll`. `Pickup` accepts `None` for outside, dropping the carried stack.
A `Drag { kind, slots }` owns its menu-order slot list and always submits a complete gesture;
`DragKind::Split` divides evenly, `One` places one per slot and `Clone` fills slots in creative mode.
Every action in a batch is validated before any prediction or sending; out-of-range slots/hotbar
indices, creative-only actions in survival and empty batches refuse without effects.
Stale/replaced/respawned menus, spectators, non-client hosts and the creative inventory screen refuse with `false`.
`true` means submitted, not server acceptance: prediction uses the normal menu engine and sends
`ContainerClick` with its state id, changed slots and carried item. Server corrections take precedence.
Inventory reads and direct server-side item transactions keep their existing contracts; a click does
not expose direct client inventory writes or enforce a mod's locked-slot policy. Call from an explicit
input action rather than submitting every render/tick.

`menu.close(host)` closes the menu's screen as Escape does and tells the server (`ContainerClose`);
it returns `false` while that screen is not open, so the inventory menu closes only while the inventory
screen shows. `menu.click_button(host, button)` presses one of the open menu's buttons
(`ContainerButtonClick`, as a loom's pattern list does), for input the screen's own
buttons don't take; the server's menu decides what it does, and the inventory menu has none.

## Writing slot storage

`ItemPlace::MenuSlot { menu, slot }` is slot `slot` of a player's open menu, a `MenuRef` (`player.container_menu(host)`). NeoForge 26.3's `ItemAccess` reaches a player's inventory slot (`forPlayerSlot`) and cursor
(`forPlayerCursor`), not a menu's slot. It is the storage the slot shows, named by `menu.slot_backing(host, slot)`
(`MenuSlotBacking`), and a write goes to that storage through its own handler, so it is kept once by the transaction
and the storage's rules hold: the player's inventory, armour and off hand by their
[`ENTITY_ITEMS`](entity-items.md) index, a chest's or a furnace's slot through its `ITEMS` handler, a chest minecart's
or a chested mount's through its `ENTITY_ITEMS`, and the player's ender chest, inventory crafting grid and a mod's chest
menu (`open_menu`, reached with `host.menu(uuid, window)`), kept by the game. The slot's own rules come first, as a click has them:
`menu.slot_holds(host, slot, &stack)` is how many of a stack it holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`: a
furnace's fuel slot one bucket, an armour slot one piece that fits it, a shulker box's slot no shulker box, a result
slot none), and `menu.slot_gives(host, slot)` whether it gives up what it holds (`Slot.mayPickup`: bound armour stays
on outside creative). An insert fills that slot alone, and what doesn't fit stays out. A crafting grid a transaction
changed has its result made again at the outermost commit, once for each player however many writes it took
(`InventoryMenu.slotsChanged`, with `prepare_item_craft`); the result slot takes nothing and gives nothing,
since taking it is a craft. A menu closed, replaced or reopened since the `MenuRef` was made, or one whose player left,
reads and writes nothing. `menu.cursor()` is that menu's cursor (`ItemPlace::PlayerCursor { menu }`,
`forPlayerCursor`), which reaches nothing once the menu is gone, so a kept cursor never reaches the next menu's. The player's screen follows with the tick's container sync.

```rust,ignore
// Put four of the menu's offered emeralds into slot 0 of the open chest, or nothing.
let (slot, cursor) = (ItemPlace::MenuSlot { menu, slot: 0 }, menu.cursor());
host.transaction(|host, tx| {
    let took = cursor.extract(host, tx, &emerald, 4);
    if took == 4 && slot.insert(host, tx, &emerald, 4) == 4 { Ok(()) } else { Err(()) }
})
```

`menu_clicked` (contract version 2) runs as one action: what a listener writes through the `Host` stays when it
returns and is undone with the rest of the action when a listener fails. `menu_clicked` hears the raw slot, button
and click type on a menu a mod opened (a single-slot drag once); menu-button clicks, recipe placement and creative
set-slot never reach it, and it cannot know whether the click changes anything.

`menu_quick_move` (contract version 6) lends the resident block entity and the server `Host` sequentially through
`event.subject: BlockEntityScope<'_>`. It runs after the live menu allows pickup, and `stack` copies what the clicked
slot shows, including a linked container's stack or the current crafting result. Return `Flow::Cancel` to handle the
move. The remainder goes back to the actual clicked source (`None` empties it). If you handle a result, consume its
crafting inputs yourself: the remainder stays available until the grid you leave changes.

Journaled destination edits and the clicked-slot remainder commit together. If the move is left unhandled, the
listener's writes roll back before the same menu routes the original click without asking pickup again. An invalid
remainder, changed source stack, or stale menu rolls back the writes and refuses the move without falling back.
Writes through `event.subject.with_view(...)` validate at the boundary and apply in place. A handled move settles the
clicked slot before publishing implicit container notices (`BlockEntity.setChanged`) for its resident slot edits and
`ITEMS` writes, then calls `menu_slot_take` where applicable. Deferred notices go only to still-resident holders.
`BlockEntityRef::set_changed` is immediate; a same-block write made in that callback does not queue another notice.

A panicking quick-move Part is disabled for this hook and loses its journaled writes and outputs. If no later Part
handles the move, the menu resyncs instead of running ordinary routing. A panic contained in a nested hook does not
by itself reject the move.

Inside `menu_clicked`, `ItemPlace::MenuSlot` can take from and give to the clicking
player's inventory and hotbar slots as well as the menu's own storage. Inserts fill
only the named slot and return the amount that fits. Abort the transaction if a
partial move should leave both places unchanged. Stale menu or player identities
refuse writes.

## A mod's chest menu

`host.open_menu(uuid, rows, title, items, replace)` opens a chest menu (`generic_9x1` to `generic_9x6`) for a player
in place of the menu they have open and returns its window id; `menu_clicked` and `menu_closed` name it too. Its items
change in place: `host.menu(uuid, window)` is its `MenuRef` while the player still has that window open (None for
another window, a closed menu or a player who left), and a write to `ItemPlace::MenuSlot { menu, slot }` in a
transaction is sent with the next tick's container sync, one `ContainerSetSlot` per changed slot, as vanilla's
`broadcastChanges` sends a menu a mod changed. Nothing reopens and the cursor stays where it is.

```rust,ignore
// In menu_clicked: show the next page's item in slot 4 of the same menu.
if let Some(menu) = host.menu(uuid, window) {
    let slot = ItemPlace::MenuSlot { menu, slot: 4 };
    let _ = host.transaction(|host, tx| {
        slot.extract(host, tx, &shown, 64);
        if slot.insert(host, tx, &next, 1) == 1 { Ok(()) } else { Err(()) }
    });
}
```

To rebuild the whole menu at once (a page turned, a list refiltered) `host.menu_set_items(uuid, window, items)`
replaces everything it shows and sends it as one `ContainerSetContent` for that window, now rather than with the next
tick's sync, as vanilla's `sendAllDataToRemote` does: the screen, the title and the cursor stay. Like `open_menu` it pads
`items` with empty slots and drops the ones past the menu's last. It returns `Result<(), MenuItemsRefusal>` and each
refusal sends and changes nothing: `Client`, `NoPlayer` (no placed player has that UUID), `NotOpen` (`window` is not
the player's open native menu), `OwnedSlots` (the menu owns its storage; use `ItemPlace::MenuSlot`), or
`Stack(StackWriteError)` (an item the world lacks, a count or damage the item does not allow). For one slot or a few,
write them through `host.menu` as above: they cost one packet each and no replacement of the others.

A title or a row count changes only with a new menu, as in vanilla: `open_menu` again, with a new window id, the
title and all the items. The old menu closes on the server as a close does, its cursor stack going back to the
player's inventory, and packets for its window are ignored. `replace` is the choice a Java `MenuProvider` makes
(`shouldTriggerClientSideContainerClosingOnOpen`): `MenuReplace::CloseFirst`, vanilla's default, first sends the
client a close, which centres the mouse again; `MenuReplace::InPlace` sends none, so the new screen takes the old
one's place and the mouse stays where it is. Opening from the player's inventory sends no close either way.
`player.open_mod_menu` takes the same `replace` for a content mod's menu type
([machine menu data](../../how-to/world/machine-menu-data.md)).

## Menu-owned slots

A blockless mod menu can own transient storage, like the container holding a crafting grid or a
stonecutter's input. A type with `owned_slots` can only open without a block and cannot declare
`results` or a `crafting` grid. Declare one stack limit (1–99) per container slot:

```json
{ "slots": 2, "player_inventory": false, "owned_slots": [1, 4] }
```

Each window starts with empty slots. Read them with `MenuRef::slot` and edit them through
`ItemPlace::MenuSlot` in a transaction. `MenuRef::slot_backing` reports `MenuSlotBacking::Owned`.
Insertion respects the smaller of the slot limit and the item's limit. Neither `back_mod_menu_slots`
nor `menu_set_items` replaces this storage. The client gets normal slot updates, with no new packet.

Ordinary clicks move items up to the slot cap on both sides. To make a filter slot, handle `menu_clicked`:
read the cursor, insert a copy without extracting from it, and return `Flow::Cancel`. The server sends the
committed menu and cursor regardless of whether their item IDs and counts match the prediction.

The storage is never saved. `menu_closed` v2 sees its final `owned_slots` in slot order and the supplied
`opening_hand`, if any. The old window and its handles are already stale. Unlike vanilla's `clearContainer`,
the game does not return or drop those values automatically. A part that wants real inputs returned must
return them using the server Host. A listener can instead write a filter selection to the held item. Check
that the player and item are still available before writing; do not give filter copies back as real items.
The cursor's ordinary close behavior is unchanged.

## Slots backed by a held item or an entity's handler

A menu opened with no block (`player.open_mod_menu`, or `open_menu`'s chest menu) can show storage that is not a block
entity, as NeoForge's `StackCopySlot` and `ResourceHandlerSlot` do. Right after the open, back its first container
slots with `player.back_mod_menu_slots(host, window, places)`, one `MenuSlotPlace` per slot:

- `MenuSlotPlace::HeldItem { hand, slot }`: slot `slot` of the container inside the stack in the player's `hand`, through
  its `ITEM_ITEMS` handler (a backpack's contents). While the menu is open, the hotbar slot holding a main-hand stack takes
  no click, so the backpack can't go into itself.
- `MenuSlotPlace::EntityItems { entity, slot }`: slot `slot` of `entity`'s `ENTITY_ITEMS` handler, as a mod's provider
  answers it.

From then on `MenuRef::slot_backing` names the slot `MenuSlotBacking::Place(place)`. A pickup or shift-click on the
menu, and an `ItemPlace::MenuSlot` write, go through that storage's own handler in one transaction, so an aborted
transaction leaves it as it was. The client sees ordinary synced slots; there is no new packet. Nothing runs per tick:
the slots are mirrored after each click on the menu and after each `ItemPlace` write to the player's places (their hand,
a container held there, the menu's slots), and only the slots that changed are written. These writes queue a change
notice, deduplicated per menu; there is no next-tick scan. A handler changed independently of an `ItemPlace` write
is checked and mirrored at the next click, not automatically on the next tick. Before each click and at each
change notice the menu checks its places: it closes once one stops resolving (the backpack left the hand, the entity is
gone), or once a `HeldItem` hand no longer holds the stack bound when the slots were backed (another stack put in its
place, another hotbar slot selected). Stacks are values, so the bound stack is told apart by its item and components
other than its contents. Slots past `places` stay the mod's own (`menu_clicked`). A drag, a number-key swap, a
double-click collect or a throw on a backed menu is still the mod's to answer.

Backing the same window again replaces its places. A slot no longer backed (an empty list, or a shorter one) is
emptied: it never keeps the copy it showed, which stays in the storage it came from. A backing that cannot be made
changes nothing and says why with a `MenuSlotBackRefusal`:

- `NoPlayerOrWindow`: the player is not online here, or `window` is not the menu they have open.
- `UnsupportedMenu`: the open menu is not a blockless mod menu, or it declares `owned_slots`.
- `TooManyPlaces`: more places than the menu's container slots.
- `PlaceUnresolved { index }`: `places[index]` names no storage now.

```rust,ignore
let mut request = ModMenuOpen::new(backpack_menu, title);
request.opening_hand = Some(Hand::Main);
let window = player.open_mod_menu(host, request)?;
if let Some(window) = window {
    let places = (0..27).map(|slot| MenuSlotPlace::HeldItem { hand: Hand::Main, slot }).collect();
    if let Err(refusal) = player.back_mod_menu_slots(host, window, places) {
        log::warn!("backpack menu not backed: {refusal:?}");
    }
}
```

The open and the backing can sit in the hook that reacts to the player, such as the backpack's own use. The menu
handle resolves and the backing is set during the action; only mirroring the backed slots to the client waits until
the action is over, later in the same tick.

A vanilla menu and a menu no mod backed never take this path: their slots keep the backing they had.

## Recipe transfer into a mod menu

A recipe viewer's "+" calls `RecipesClientHost::transfer_recipe_id(recipe, max)` (or `transfer_recipe(book_id)`
for a recipe-book recipe). The game's own transfer fills crafting grids only, so for a machine's menu the
server first asks the `MenuPart` of the open mod menu type:

```rust,ignore
fn menu_transfer(
    &self,
    host: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::MenuTransferEvent<'_>,
) -> Option<mod_api::RecipeTransferResult> {
    // Look up event.recipe, check event.player's inventory, and unless event.dry_run
    // move one set (as many as fit with event.max) into the block entity at event.pos.
    let _ = (host, event);
    Some(mod_api::RecipeTransferResult::Moves)
}
```

Return `None` to hand the transfer on to the game's grid transfer (`NotIntoMenu` for a menu without a
crafting grid). A dry run, the viewer's check, must move nothing. Vanilla menu types are never asked. A solo
world and a LAN host's own player get the same contract: the game is the server there and asks the part of the
player's open block menu before its own grid transfer, on the same thread.

`menu.slot_kinds(host)` is a client read in menu index order, with exactly
`slot_count` entries. It describes the client's layout and content declaration,
not the server's backing or acceptance rules. `Player(index)` uses hotbar 0–8,
main 9–35, armour 36–39 (feet to head), off hand 40. Container equipment is
`Equipment`. General storage, including unmarked linked slots, is `Storage`.
Furnaces, blast furnaces, and smokers report the ingredient and fuel slots as `Input`
and the output as `Result`. Brewing stands report the bottle, ingredient, and fuel
slots as `Input`. Content menus report `Input` for the slots listed in their
[menu type's `inputs`](../../how-to/content-mods.md#menu-types), even when those slots
are linked. Workstations without a classified layout (anvil, enchanting table, loom,
and similar types) return `Other` for every slot, including player slots. A game menu with no known client slot count returns `None`, as do
stale or foreign handles, missing content types, and calls without an open menu
or outside a world. Creative returns the underlying inventory menu's 46 slots,
never creative tab wrappers; read the screen type to distinguish them.
The answer is stable for one open menu. Cache it: each read builds one Vec
from the layout table and content declaration, with one allocation.
