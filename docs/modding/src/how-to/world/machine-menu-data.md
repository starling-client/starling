# Declaring a machine's menu data

A mod block entity has two different kinds of storage: external item access uses
transactional Parts, while the machine's own ticker uses its resident view. Outside the ticker,
use a `BlockEntityRef` and the Host to read or write internal slots and menu data. Menu data is transient:
creation and loading both start with an empty data vector, even when the saved
payload contains the machine's progress.

## State-aware creation

Declare the block entity type and its owning blocks during registration. To make
creation depend on block properties, answer `stateful_block_entity` with `true` for
that block. The `block_entity` hook then receives the actual placed
state, not just the block identity. Read its properties through the Host's state
registry and supply the initial saved compound. The registered declaration owns
the entity type; an unanswered state-aware creation makes no entity.

Creation runs inside the block write, after placement side effects, on servers
and clients. Changing properties within the same block preserves an existing
entity even when a fresh instance of the new state would have no entity.

## Size the data vector

Call `entity.set_data_slots(host, initial_values)` with the complete initial
vector when preparing a newly created or loaded machine. For a progress display,
use one element initialized from the progress value in its saved payload.
Do not reset the vector to its initial values on every tick.

Block-backed menus use the signed 16-bit property index of `ClientboundContainerSetDataPacket`.
The method accepts every index from zero through `i16::MAX`.
It returns `Ok(true)` when it changes the vector, `Ok(false)` for identical values,
or `BlockEntityDataError::TooManySlots` with the submitted count and limit. It
never silently clamps a vector. `BlockEntityDataError::Unavailable` means the
handle cannot reach a writable mod holder in the server level in place.

## Update individual values

After sizing, call `entity.set_data_slot(host, index, value)` for scalar updates.
This method does not resize: an absent index returns `Ok(false)`. Both methods use the
same silent storage write target and journal. Because menu data is transient and not
serialized, writes do not mark the chunk unsaved, send a block update, or emit a
neighbor notice; call `set_changed` explicitly when persisted state changes. Transactions retain their
ordinary behavior; an aborted outer transaction restores the vector that existed
before its first write, including writes committed by children. Block-backed menu
broadcast sends committed values at indices 0 through 32,767 as signed shorts, as
vanilla does. Larger indices remain in memory but are not sent by `ContainerSetData`.
To draw those values on the client, see [Draw a progress bar and a fuel gauge](../client/menu-gauges.md).

The public handle resolves one entity's lifecycle generation as well as its level
and position. Removal, replacement and chunk reload make it stale; in-place data
edits preserve it. It is not a borrowed-payload API.

## Internal items versus automation

For a configured mod container, `item_slot(host, slot)` snapshots an internal slot;
`set_item_slot(host, tx, slot, stack)` replaces it within an existing server
transaction. Any held mod block-entity handle can be used; there is no caller
ownership check. This deliberately bypasses inputs, outputs and face policy, so a
recipe can consume its input and produce its output atomically, including output
slots that automation cannot insert into. Use the `ITEMS` capability for external
insertion; it enforces automation policy. Vanilla containers are unavailable here.

Indices use the current container's declared count. Lazy slots read empty; saved
excess slots are neither exposed nor truncated. `None` clears, identical values
return `Ok(false)`. A committed change marks the chunk unsaved without notifying
comparators or neighbours. Call `set_changed` explicitly to notify. Nested abort
restores ordinary before-images, including committed-child writes. Invalid
indices, unknown/air/zero-count stacks and counts above the registered or
component-overridden maximum return `BlockEntityItemError`, never a partial write
or silent count clamp. A stale handle refuses a replacement rather than accessing
its container; acquire a new handle for the replacement.

## Borrow the ticking machine

`BlockEntityPart::block_entity_tick` receives a `BlockEntityTickEvent<'_>` and no
separate Host argument. It runs only if the block currently supports that entity type. If a resident
ticker finds an incompatible block, the game warns once and skips the tick. The next valid tick clears
the warning so a later mismatch is reported again.
Its `subject` lends the live row. Use the view for the machine's own data,
saved fields and internal slots. Derive owned values from the view before taking
the Host for other objects. Each `with_view` loan checks that
the same entity instance is still loaded in the same level (same generation) and
that the live block at `pos` is still valid for the block entity's type
(`BlockEntityType.isValid`). In-place block-state updates to another valid state
keep the loan valid and expose the updated live `BlockState` through `view.state()`;
removal, replacement, or an invalid block returns `None`.

```rust,ignore
fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
    let tick = &mut event.subject;
    let Some((level, pos)) = tick.with_view(|view| (view.level(), view.position())) else {
        return;
    };
    let above = [pos[0], pos[1] + 1, pos[2]];
    let obstructed = level.block(tick.host(), above) != Some(self.air);
    tick.with_view(|mut view| view.set_data_slot(0, i32::from(obstructed)));
}
```

`item_slot` lends a read-only stack; call `to_owned` only if you need it after
releasing the view. `set_nbt` edits a saved field without reloading the entity:
`"Items"` is reserved only on container-backed block entities whose block declares
engine-managed item slots (`item_slots() > 0`); on slotless block entities
(`item_slots() == 0`), `set_nbt("Items", ...)` writes or removes the NBT field
directly and clears any disk-decoded fallback item slots.
Ticker-view writes do not automatically mark the chunk unsaved, send a block
update, or notify neighbors—call `set_changed` explicitly through `tick.host()`
when persisted state changes (matching vanilla `BlockEntity.setChanged` /
`AbstractFurnaceBlockEntity.serverTick`). Normal menu sync still runs for transient
data slots. If a neighbor write must commit with the subject edit, use
`tick.transaction` and return `Err` if reacquiring the subject or checking its
inputs fails. A read-only or no-op loan does not copy or journal storage. A type
with no subscriber skips the tick entirely; pending load notices still come first.

### Combining subject and world writes in one transaction

To make a dropped-item edit and a resident block-entity write atomic, run both inside
`tick.transaction` and return `Err` if either write or the subject reacquisition is refused:

```rust
{{#include ../../../examples/src/block_entity_transaction.rs:block_entity_transaction}}
```

## One-shot client effects

To tell clients about something that happened once, such as a burst of particles, queue
fields instead of writing them to the saved `nbt`. In `block_entity_tick`, queue
fields and take the holder's handle in the same loan:
`tick.with_view(|mut view| { view.set_client_update_fields(fields); view.handle() })`.
If it returns `Some(holder)`, call `holder.sync(tick.host())` after the loan ends
and handle its `LevelWriteError`. The handle works while the holder is out for its own tick.
A later call to `set_client_update_fields` replaces the batch; an empty compound clears it.

The next block change that reaches a viewer puts the fields over what
`block_entity_update_tag` gives, or sends them alone without one, in one packet for every
current viewer, and consumes the batch. A change nobody sees leaves it waiting, and neither a
chunk sent to a player nor a resend of the blocks under a single player takes it. Removing
or unloading the block entity drops it. It is never saved, cloned or compared, and it does
not change the entity's revision.

If the merged tag exceeds the 2 MiB network-NBT limit, the game drops the fields,
logs a warning, and sends the original update tag instead. When a transaction rolls back
the block entity, it restores the batch waiting before the transaction's first edit.
If a sync was requested, that batch still goes out on the next block change with viewers.

A client merges the fields into its copy's `nbt` as any update field, and they stay there
until sent again: put a counter in them to tell one effect from the next.

## Retaining a handle

A `BlockEntityRef` names one live entity, not every entity later placed at its position. Removal,
replacement and chunk reload invalidate it; reacquire with `host.block_entity(level, position)`.
In-place data edits and temporary tick storage preserve identity. Option reads return `None` for
stale handles; writes return their error's `Stale` variant, never writing to a replacement.
`set_lock`, `set_name`, `merge_data`, `replace_data`, `set_changed` and `set_data_slot`
return `Result<bool, BlockEntityError>`: handle `Err(BlockEntityError::Stale)` by dropping
or reacquiring the handle. `Ok(false)` means unchanged data or an unsupported operation,
not a stale identity. Item and data-vector methods return `BlockEntityItemError` and
`BlockEntityDataError`, respectively.

`entity.lock(host)` borrows the stored vanilla item predicate while the shared Host
borrow lasts. An explicit empty predicate is present; no predicate is `None`.
To change it, `entity.set_lock(host, Some(lock))` still replaces the entire predicate;
`None` clears it. `entity.data(host)` instead builds an owned save snapshot without
identity or position metadata.

## When its chunk unloads

A machine that keeps something outside its saved data (a running job, an open handle, its place in an
energy or pipe network) lets go of it in `on_chunk_unloaded`, NeoForge's `onChunkUnloaded`: a
`BlockEntityPart` on the type is told once for each of its block entities in a chunk that unloads, on each
side that holds a copy, before the copy is dropped.

```rust,ignore
struct Node;
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Node {
    fn on_chunk_unloaded(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::OnChunkUnloadedEvent<'_>,
    ) {
        if let mod_api::HostSide::Server(_) = host {
            // The handle equals the one `host.block_entity` gave while it was loaded.
            NETWORK.lock().unwrap().remove(&event.block_entity);
        }
    }
}
// In Hooks::register:
// registrar.block_entity_part(&node_type, Node)?;
```

`event.pos` is where it stands and `event.entity` borrows its data at the call: on a server what the chunk was
just saved with, on a client its copy as the server last sent it. On a server the chunk is saved before
the call, so a change made in it is not kept, and the chunk may already have left the level, so its
handle's reads find nothing there. It is not called when the world closes, nor on a client when its whole
level goes, and never for a vanilla block entity. The chunk-wide notice is `chunk_unload`, a
server hook told after the chunk's block entities; a client is told of the block entities alone
([hook notes](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/on_chunk_unloaded.rs)).

## Client ticks: a machine's hum

What a running machine does on the client alone (a looping hum, smoke from its chimney, a part eased between
two server updates) hangs off its client tick, vanilla's client `BlockEntityTicker`. Attach a
`BlockEntityPart` that implements `block_entity_client_tick`. The type binds a client ticker for its
accepted states and receives the callback once per client tick for each copy this client holds, while the level runs (not
paused or frozen), in vanilla's ticker order: a chunk's initial tickers join in packet order, and a state
change that adds a ticker joins it when enabled. A loaded copy ticks whether or not it is drawn: hidden behind a
wall, out of view or beyond `block_entity_view_distance`, where `block_entity_submit` is not called. It is
never called on a server, and a type with no client tick method has no client ticker.

Use `BlockEntityTypeBuilder::ticker_when` if one type should tick only in some states or on a
particular side. Its callback receives immutable `BlockMetadata`, the typed `BlockState` and the
`LogicalSide`. For example, `.ticker_when(|blocks, state, _side| state.property(blocks, "enabled") ==
Ok(Some("true")))` binds only enabled states on sides that have a subscribed Part. The decision is
prepared with the block-state table, not asked on every tick. Server tickers use the same registration
and run `BlockEntityPart::block_entity_tick` through the server scope.

The event carries `event.subject: BlockEntityClientScope<'_>`: call `event.subject.with_view(|view| ...)`
to read the resident copy's handle, position, level, block state, data slots, item slots and merged NBT
directly (no per-tick snapshot copy), then call `event.subject.host()` for the `ClientHost`. The resident
data is what the frame's handling of the server's packets merged into the copy (`block_entity_update_tag`,
and `block_entity_data_received` told before): a frame handles what has arrived, then runs its client ticks,
so every tick of one frame sees the same data, and an update that arrives during a frame is seen from the
next frame's first tick.

Here a machine whose update tag carries `running` starts a looping hum on the first client tick it runs and
fades it out when it stops:

```rust,ignore
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The hums playing, by the machine they belong to.
static HUMS: LazyLock<Mutex<HashMap<mod_api::BlockEntityRef, mod_api::SoundRef>>> =
    LazyLock::new(Default::default);

struct Hum;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Hum {}
#[mod_api::sunrise_part]
impl mod_api::BlockEntityPart for Hum {
    fn block_entity_client_tick(
        &self,
        event: &mut mod_api::BlockEntityClientTickEvent<'_>,
    ) {
        let Some((handle, pos, running)) = event.subject.with_view(|view| {
            let running = matches!(view.nbt().get(&"running".into()), Some(mod_api::Tag::Byte(1)));
            (view.handle(), view.position(), running)
        }) else {
            return;
        };
        let host = event.subject.host();
        let mut hums = HUMS.lock().unwrap();
        match (running, hums.get(&handle).copied()) {
            (true, None) => {
                let [x, y, z] = pos.map(|c| f64::from(c) + 0.5);
                let play = mod_api::SoundPlay::at("mymod:machine.hum", mod_api::SoundSource::Blocks, [x, y, z])
                    .looping(0);
                if let Some(sound) = host.play_local_sound(&play, 0) {
                    hums.insert(handle, sound);
                }
            }
            (false, Some(sound)) => {
                // A fade to 0 stops the sound when it ends.
                sound.fade(&*host, 0.0, 20);
                hums.remove(&handle);
            }
            _ => {}
        }
    }
}

struct MyMod;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for MyMod {
    // A machine removed, replaced or whose chunk unloaded ticks no more, and its handle no
    // longer reads: its hum stops.
    fn client_tick_post(&self, host: &mut dyn mod_api::ClientHost, _: &mut mod_api::ClientTickPostEvent) {
        HUMS.lock().unwrap().retain(|machine, sound| {
            let here = machine.position(&*host).is_some();
            if !here {
                sound.stop(&*host);
            }
            here
        });
    }
}
```

The copies are told in the order they arrived, a chunk's own in its map's order, as vanilla's client ticker
list gives. One removed, replaced by another type, changed to a block state without a client ticker or whose
chunk unloads is not told again from that tick on, so a looping sound it started plays on until something else
stops it: the example stops a hum once its machine's handle no longer reads (removal, replacement and unload
all end a handle), and a type with states that don't tick would also stop it when the state changes. Leaving
or changing levels stops every sound. Add particles from the same tick with `host.add_particle`
([hook notes](../../reference/hook-notes.md#client-ticks-hums-smoke-and-moving-parts)).

`player.open_menu(host, entity)` also returns `Result<bool, BlockEntityError>`:
`Ok(true)` means opened, or queued if called inside a transaction. A queued open runs at
outermost commit and the provider can still refuse; an abort discards the request. `Ok(false)`
means a non-provider, cancelled open, unconnected/wrong-level player or an active speculative menu
action. `Err(BlockEntityError::Stale)` means the entity
does not resolve. It never opens a replacement through an old handle.

## Blockless status menus

To open a content mod menu type with no block (from an item, an entity or a command),
call `player.open_mod_menu(host, request)` with a `ModMenuOpen` request (NeoForge's
`openMenu(provider, extraDataWriter)`). `menu` is the type's `MenuKey`. `extra`, at most
1,048,555 bytes (vanilla's 1 MiB clientbound custom payload limit minus the 21-byte
envelope header), is one-time context. Decode `open.extra` in the registered client screen factory.
It is an empty slice when no payload matched this opening. Reusing a window id does not reuse
its previous context. Keep owned decoded values if the screen needs them later; the bytes are
borrowed only during construction. `pos` is an optional hint offered to the factory and later
returned by `screen_pos()`. Neither is live data or block backing.
Pass `Some(hand)` from an item's use to let `menu_still_valid` compare that hand's current
stack with the one at open. Other openers pass `None`. The menu's declared container slots,
if any, start empty; [`owned_slots`](../../concepts/capabilities/menu-slots.md#menu-owned-slots)
declares a temporary container that belongs to the window, and adding `results` makes it a
blockless processing menu that computes outputs in `menu_slot_changed`, handles takes in
`menu_slot_take`, and returns its non-result inputs to the player on close (dropping overflow, or
dropping on disconnect or death, and discarding result slots). Opening returns `Ok(Some(MenuRef))`
(call `menu.container_id(host)` for its window id) or `Ok(None)` when vetoed. Invalid type,
connection or client support, opening bytes, data count or
title returns `OpenMenuError`. Neither a refused nor a cancelled open closes the old menu.
In a transaction `Ok(Some(staged))` returns a `MenuRef` for the staged request, not a live
window. It cannot read or change any live menu. `container_opened` and installation wait until
block changes have replayed at outermost commit. The veto runs in the player's level outside
the transaction. Obtain the live handle after commit from `player.container_menu(host)` or in
`player_container_open`; an abort leaves the old menu and its window counter untouched.
An open requested during menu removal also returns a staged handle and waits for the close
and any earlier requests for that player. Initialize the installed menu in `player_container_open`.

For a slotless status screen, declare `data/mymod/sunrise/menu/status.json` as
`{ "slots": 0, "player_inventory": false, "data_slots": 2 }`. Register a screen factory
for `mymod:status` with an empty slot layout. Add a draw callback to the screen for its presentation; attach a `MenuPart` for server menu behavior. Open it with two signed values, in index order:

```rust,ignore
let mut request = ModMenuOpen::new(status, Text::translate("mymod.status", vec![]));
request.data = vec![-80_000, 7];
let _menu = player.open_mod_menu(host, request)?; // Option<MenuRef>
```

The negotiated blockless payload sends initial `i32` values after the window opens, at
indices 0 through 65,535. Larger declared indices remain in server storage. A client
without the required layout is refused with `OpenMenuError::PlayerUnavailable`. In the draw
callback, read `frame.data.get(index)`; values are not available before they arrive. Server code
gets the live `MenuRef` from `player.container_menu(host)` or the open event and calls, for
example, `menu.set_data_slot(host, 0, 120_000)?`. It returns `true` for a changed value,
`false` for a repeat, and refuses unsupported or stale menus and undeclared indices.
Updates go only to this player and this window. Within a transaction, another write sees the
pending value, but a full resend only includes committed values. Commit sends the final changed
values and abort discards pending writes. Closing or replacing the menu discards its data.
Buttons and validity still use `menu_button` and `menu_still_valid`; neither needs a slot.

The menu the player had open closes on the server first, as a close does: its `removed`
(`MenuPart::menu_removed` for an owning mod menu, with any detached `owned_slots` and
`opening_hand`), the cursor stack and crafting grids back to the inventory, and then one
`player_container_close` before `containerMenu` resets to the inventory menu. Then
the new menu is built, its screen opened and its contents sent, and `player_container_open`
tells of it. A native open requested in a transaction or during Close returns a staged identity,
not a usable window. It runs on commit or after the current Close; initialize the new owner's slots
and data in `player_container_open`. An abort leaves the old menu alone and does not advance the wire
counter. On disconnect, `player_logged_out` runs before the old menu's owner and Close listeners.
`player_container_open` v2 and `player_container_close` v2 both name
the type string `menu_type` and supply `menu: Option<MenuRef>` (`player_container_close` v2 also
passes `player: PlayerRef`), valid during the callback as `PlayerRef::container_menu` is. The closing
handle goes stale after `player_container_close` returns. It is `None` for crafting tables, merchants,
workstations, lecterns and mount inventories. `replace` is the Java `MenuProvider`'s choice
(`shouldTriggerClientSideContainerClosingOnOpen`), as for
[`PlayerRef::open_chest_menu`](../../concepts/capabilities/menu-slots.md#a-mods-chest-menu):
`MenuReplace::CloseFirst`, vanilla's default, first sends the client a close for the old
window, which centres the mouse again; `MenuReplace::InPlace` sends none, so the client keeps
its screen and its mouse where they are until the new screen replaces it, as when one
screen of a machine leads to the next.

### Window ids

Each menu the server opens for a player takes the next id of that player's container counter
(vanilla's `ServerPlayer.containerCounter`), 1 to 100 and round again. `player.container_counter(host)` reads
the id the last open took, so the next one is `counter % 100 + 1`: the id to hand a menu built before the open,
as Java's `MenuProvider.createMenu(containerId, ...)` is. `player.next_container_counter(host)` advances the
counter as an open does without opening anything, for an open that decides after the advance that no window
follows (a `createMenu` that answers `null`). A mod modelling `ServerPlayer.openMenu` from inside the player's
own action closes the open menu first with `menu.close_container(host)`, which closes at the call there.

## The menu's screen

Register a constructor in `Hooks::register` with `register_menu_screen`, as with NeoForge's
`RegisterMenuScreensEvent`. The client calls it once for the opening, after the menu's type and
slots are known and before the window is shown. It returns a fresh `MenuScreen`. If a custom
menu or a stock opener with excluded slots has no registered factory, the client warns and
keeps its current menu without sending a close. When a selected factory returns `None` or
produces an invalid layout, the client refuses that opening.

Stock menu types already have a screen. To put your own screen over a hopper-like
menu, declare a distinct type in `data/mymod/sunrise/menu/hopper_menu.json`
with `{ "slots": 5, "player_inventory": true }`. Set `ModContainer.menu` on your
block to `Some("mymod:hopper_menu".into())`, with the same five backing slots, and
register your factory with `r.register_menu_screen("mymod:hopper_menu", factory)`.
In the factory, place the five slots at `[44 + 18 * i, 20]` in GUI pixels and the
player inventory at `[8, 51]` to match the hopper layout. The vanilla hopper
continues to open its built-in screen.

```rust,ignore
use mod_api::{ClientHost, MenuScreen, MenuScreenLayout, MenuScreenOpen, ScreenSlot};
use std::sync::Arc;

r.register_menu_screen("mymod:backpack", Arc::new(
    |_: &dyn ClientHost, open: MenuScreenOpen<'_>| {
        if open.backing_slots != 9 {
            return None;
        }
        Some(MenuScreen::new(MenuScreenLayout {
            width: 176.0,
            height: 133.0,
            slots: (0..9)
                .map(|i| Some(ScreenSlot {
                    position: [8.0 + 18.0 * i as f32, 18.0],
                    no_item_icon: None,
                }))
                .collect(),
            inventory_x: 8.0,
            inventory_y: 51.0,
            ..Default::default()
        }))
    },
)).unwrap();
```

`open.menu` names the active type; `open.block` is the verified mod block whose container opened
it, or `None` without a matching block. This lets one constructor handle different openers
without retaining the borrowed context. Factories are keyed directly by menu type, matching
NeoForge's `RegisterMenuScreensEvent` and vanilla's `MenuScreens.SCREENS`: any mod may register
a screen for a custom menu type (including another mod's or a non-built-in `minecraft:*` menu
type), while built-in vanilla menu types that already have screens in `MenuScreens.SCREENS` and
duplicate registrations for the same menu type are refused.

Add only the callbacks the new screen uses: `with_draw(|host, frame, draw| ...)`,
`with_click(|host, frame, right| ...)` and `with_tooltip(|host, frame, slot, lines| ...)`.
The window owns the callbacks until the screen is closed or replaced. Decode `open.extra`
in the factory and move any needed owned values into the callbacks. Live integer updates
arrive separately in `frame.data`.

### Custom menu screens and initially inactive slots

Define a custom menu type for your machine's slots and register its screen factory with
`r.register_menu_screen(MENU, ...)`. Leave `None` (or omit trailing entries) in
`MenuScreenLayout::slots` for slots that should start inactive/hidden (`Slot.isActive()`);
their physical indices stay unchanged, and any mod may activate or hide them later with
`host.set_screen_slot`.

```rust,ignore
use mod_api::{ClientHost, MenuScreen, MenuScreenLayout, MenuScreenOpen, ModContainer, Registrar, ScreenSlot};
use std::sync::Arc;

fn register_machine(r: &mut dyn Registrar) -> Result<(), String> {
    const BLOCK: &str = "mymod:machine";
    const MENU: &str = "mymod:machine";
    let block = r.block_key(BLOCK);
    r.block_container(&block, ModContainer {
        slots: 5,
        faces: std::array::from_fn(|_| vec![0, 1]),
        inputs: vec![0],
        outputs: vec![1],
        insert_rules: vec![],
        menu: Some(MENU.into()),
    });
    r.register_menu_screen(MENU, Arc::new(
        |_: &dyn ClientHost, open: MenuScreenOpen<'_>| {
            if open.block != Some(BLOCK) {
                return None;
            }
            Some(MenuScreen::new(MenuScreenLayout {
                width: 176.0,
                height: 166.0,
                slots: vec![
                    Some(ScreenSlot { position: [44.0, 35.0], no_item_icon: None }),
                    Some(ScreenSlot { position: [116.0, 35.0], no_item_icon: None }),
                ],
                inventory_x: 8.0,
                inventory_y: 84.0,
                ..MenuScreenLayout::default()
            }))
        },
    ))
}
```

A `None` entry or omitted trailing slot stays indexed in the container backing and starts
inactive on the client screen, without changing server-side slot validation. To prevent
players from placing into or taking from a backing slot, attach a `MenuPart` to the menu type
and set both `event.may_place` and `event.may_pickup` to `false` in `menu_slot_rules` when
`event.container == MenuContainer::Main` and the backing index is one of those slots.
If the extra backing indices are not needed, declare only the exposed slots on the menu type.

A click callback returns `None` to leave the press alone or `Some(id)` to consume it and
send that button to the server. The active menu's server `MenuPart` handles accepted IDs
in `menu_button` after the normal window and menu checks and `menu_button_valid`. The owner
is the declaring package of the custom menu type or the verified opener of a stock menu, not
the package supplying its screen or recipe book. Do not use a client hit test or
opening data as authority. `event.subject` is a live `BlockEntityScope`: call
`event.subject.with_view(...)` to read or edit the backing resident block entity in place
(it returns `None` on a blockless menu, where `event.context.pos` is also `None`, or after
removal/replacement), `event.subject.host()` for server Host operations, and
`event.subject.transaction(...)` for atomic edits across the subject and world storage.
Write cursor changes through `event.carried`; each write validates at the boundary, and a
panicking button callback is contained (discarding its carried-stack edit and rolling back
any open transaction) and disabled for `menu_button` for the rest of the session. If a
different hook panics, a button callback that returns normally can still change the cursor.
In the block's [`menu_quick_move`](../../concepts/capabilities/menu-slots.md#writing-slot-storage),
`event.subject` is the same live `BlockEntityScope` (per-write boundary validation in place,
with `subject.transaction(...)` when edits must commit atomically across the subject and other
world storage). A panicking quick-move callback loses its `stack` and `handled` outputs and
is disabled for that hook, but a later Part can still handle the move.
A [bound recipe book](../items/recipe-book-categories.md#binding-a-book-to-a-menu) works for
blockless menus when its recipe slots have native slot backing. Register a screen factory
for the bound menu and set `MenuScreenLayout::recipe_book` to the bound book; a missing or
different book or invalid geometry refuses that client opening.

## Laying a screen's slots out again

A screen whose slots move while it is open (Tinkers' tinker station, which lays its input slots out for the
tool being built) calls `host.set_screen_slot(menu, slot, Some([x, y]))` on the client, or `None` to hide a
slot ([moving or hiding a mod screen's slots](../client/screens.md#moving-or-hiding-a-mod-screens-slots)). Only
the screen changes: server-side slot rules do not change.

## Receiving synchronized data

Implement `block_entity_data_received` on the block entity's part to react on the
client after its synchronized tag has merged. It receives `event.subject: BlockEntityClientReadScope<'_>`
(`with_view` lends a read-only `BlockEntityReadView` that already sees the merged copy, and `host()`
borrows `&dyn ClientHost`) together with the arriving compound `event.tag`. This
listen notice carries `BlockEntityDataSource::Chunk` or `UpdatePacket`. A chunk
with an empty compound sends no notice. An update packet with an empty compound
sends one notice; a packet with no tag sends none. Repeated identical packets
each notify, and `event.tag` contains only the arriving fields, while the scoped view
sees the merged copy.
It cannot change the arriving tag. Vanilla entities, missing tags, missing client
entities and mismatched types do not call it. Server tag production uses
`block_entity_update_tag`.
