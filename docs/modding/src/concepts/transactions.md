# Transactions

Use `HostTransaction` on a `ServerHost` to group capability writes. The closure's
`Ok` commits; `Err` aborts. Pass the borrowed `&Transaction` to each handler or
place write.

```rust,ignore
use mod_api::{HostTransaction, ServerHost};

fn update(host: &mut dyn ServerHost) -> Result<(), ()> {
    host.transaction(|host, tx| {
        // Pass host and tx to your capability writes.
        Ok(())
    })
}
```

## Commit and rollback

- Writes apply immediately, so subsequent steps in the transaction see them.
- The game snapshots each storage before its first write. Handlers use the game's
  journal rather than keeping their own.
- A nested commit merges into its parent. Only the outermost commit makes storage
  changes final and publishes deferred engine-managed container effects.
- An abort restores recorded storage without publishing those commit effects.
- Direct writes by mods to their own storage and rollback restores issue no notices.
- Explicit changed/comparator/neighbour/data-send notifications run when issued,
  including inside a transaction, and survive abort.
- A panic unwinding out of the closure aborts its transaction. The parent remains
  open if you catch the panic inside the parent closure.
- Closing a transaction aborts any transactions left open inside it before closing
  the parent.

The journal covers these storages, including writes through capability handlers:

| Storage | Recorded state |
|---|---|
| Block entity | Item slots, transient `data_slots`, saved compound, queued one-shot client update fields, custom name and attachments |
| Hopper | Cooldown |
| Cauldron | Block state |
| Composter | Block state and composter-local random stream; fill feedback, game events and maturation scheduling wait for outer commit |
| World block set, removal, bulk edits, and fluid pickup and placement | Block state, and a block entity the new block removes |
| Player | Inventory slots, armour, offhand, body, saddle, cursor, ender chest and persistent data, for host and guest players |
| Entity inventory | Chest or hopper minecart, chest boat, chested donkey, mule or llama inventory |
| Item entity | Stack; an emptied entity remains until the outermost commit removes it |

An abort restores recorded player storage even if the player died, is in another level, or
is no longer the acting player when the transaction closes. This includes fake players.
The player need not be available through a mod's handle at that point. Restoring these
writes does not itself undo the death or move the player between levels.

At the outermost commit, default `ITEMS` and item `STORAGE` writes to vanilla containers
and engine-managed mod slots publish their implicit change notifications.
Other deferred effects include jukebox playback, cauldron and composter block updates and
item drops, including items placed at a block, thrown by a player or entity, or split off an
item entity. Explicit mod-issued sounds, particles and level, entity and player events are
deferred too: see [Deferred level, entity, and player effects](#deferred-level-entity-and-player-effects).

A world pickup or placement of fluid (`level.pickup_fluid`, `level.place_fluid`) writes
its block at the call and the rest at the outermost commit; an aborted one leaves the world
unchanged, where NeoForge's may not. `level.play_fluid_effects` queues a fluid's sound and
game event the same way: they play only at the outermost commit and are dropped on abort,
unlike `trigger_sound_and_game_event`, which acts at once. The `block_drops` listeners of a
replaced block see it as it was, and a cancelled drop keeps the replacement.

`LevelRef::set_block` and `LevelRef::remove_block` expose the tentative block state
right away; block updates and replacement effects wait for the outermost commit
in that level. Removal uses the block entity as it stood at replacement, including
any edits made before the replacement. For a direct write, the resident row's drops
join before it is removed and before neighbor notifications. The drops already own
the stacks they carry: a join listener that replaces the same position again does
not spill those stacks a second time. The first write only removes the original
block entity. A replacement installed by the listener does not inherit the
original entity's saved tag or lock. The replacement keeps its own items and
saved data, including a lock set on its new handle, even if the position returns
to the state the first write requested. On abort, the journal restores the
original entity and undoes those edits. `host.edit_blocks` also quietly restores the
original blocks, block entities and connection groups on abort; it publishes whole
chunks with their resulting connection groups on commit, without drops or neighbor updates. If both kinds of write touch one position,
they replay in call order; a later bulk edit cannot substitute its new block
entity for the ordinary removal's old one. `LevelRef::destroy_block` returns
`Ok(None)` while a transaction is open, before running loot, experience or break
effects.

`ItemEntityRef::set_stack` writes the replacement stack immediately and joins the
innermost transaction in the item's level, if one is open. An abort restores the old
stack. A command run in another level (`execute in`) does not share the original
level's journal; open a transaction in the other level to stage writes there. A block-entity
tick does not open a transaction for you. Use `event.subject.transaction` to make
a subject edit and a dropped-item write atomic:

```rust,ignore
use mod_api::{BlockEntityScope, ItemEntityRef, ItemStack};

fn update(
    tick: &mut BlockEntityScope<'_>,
    item: ItemEntityRef,
    replacement: ItemStack,
    progress: i32,
) -> Result<(), &'static str> {
    tick.transaction(|tick, _| {
        if !item.set_stack(tick.host(), replacement).map_err(|_| "invalid stack")? {
            return Err("dropped-item write was refused");
        }
        let changed = tick.with_view(|mut view| view.set_data_slot(0, progress))
            .ok_or("subject is unavailable")?;
        if !changed {
            return Err("progress was not updated");
        }
        Ok(())
    })
}
```

Other direct writes are not journaled: `LivingRef::set_equipment`, non-player
`LivingRef::hurt_and_break`, and the armor-stand `slot_set` paths take effect immediately,
even inside a transaction. Equipment sounds, equip/unequip events from
`LivingRef::set_equipment`, and the equipment-break entity event from non-player
`LivingRef::hurt_and_break` also happen immediately so the write and its notification stay
on the same side of the rollback boundary. An abort does not undo these writes or effects.
Use journaled capability handlers when you need rollback.

## Deferred level, entity, and player effects

An explicit server-side presentation effect joins the innermost open transaction in the level
active at the call. It waits for the outermost commit for that owning level and is discarded on
rollback or an aborted nested transaction. Delivery still uses the level named by the request,
including for sounds or particles targeting a parked level. An independent transaction in another
level publishes its own effects at its own commit.

Calls covered by these rules:

- `LevelRef::play_sound`, `LevelRef::send_particles`, `LevelRef::level_event`,
  `LevelRef::block_break_effect`, `LevelRef::global_level_event` (and the `LevelWriteHost`
  counterparts `play_sound`, `send_particles`, `level_event`)
- `EntityRef::play_sound`, `EntityRef::play_bound_sound`, `EntityRef::broadcast_event`
- `PlayerRef::play_sound`, `PlayerRef::stop_sound`

An accepted `LevelRef::game_event` is posted at the call for vibration listeners, not deferred by
this rule.

Argument and handle validation still happens at the call, so invalid requests return `false`
immediately. For directed sounds, `true` means the request was accepted, not that a client received
or played it. A deferred request is discarded on abort or if the player no longer resolves at commit. For level and entity sounds, `play_level_sound_at_position` and
`play_level_sound_at_entity` run synchronously at the call inside the open transaction, so
listeners can edit or cancel the sound and have their writes or effects journaled (in the
listener's level when a transaction is open in that level, and for player-owned state in the
enclosing transaction); only the resolved effect record is deferred. On the outermost commit, pure
delivery sends every deferred record in journal order across all effect kinds, replaying in each
request's level (even if another level is active when the transaction commits) without running mod
callbacks or draining unrelated queued effects.

Outside a transaction, a queued level sound draws its seed at the call even if a later hook
cancels it. Inside one, it draws a seed only when the resolved sound is published. Directed
`PlayerRef::play_sound` draws from the request level's random at an immediate send or when
published at commit. An aborted request does not advance either stream.

## Spawning and removing entities

Entities a transaction spawns or removes follow the same commit and rollback rules:

- A spawn builds the entity at the call: its handle works, its attachments and Parts exist,
  `entity_constructing` has run and selectors find it. Its `entity_join_level` does not run
  until the outermost commit, in the order the entities were added, before any removal
  of the transaction takes effect. A listener reads the entity at the commit, not during the
  call. A join a listener cancels at the commit means the entity was never announced and the
  handle the spawn returned reads stale from then on.
- An abort takes out every entity the transaction spawned, with its attachments and saved data, and
  tells no mod: neither `entity_join_level` nor `entity_leave_level` runs. An entity id is
  never reused, so the aborted entity's id stays burnt and a stale handle never names a later entity.
- A removal (`EntityRef::discard`, a kill) marks the entity at the call: its handle and selectors
  stop finding it, and a mob reads as removed (it is flagged at the call; any other entity is
  listed as removed until the commit has run its removal, joins told by then included).
  Passengers dismount, the leave
  is told and the other effects of removal wait for the outermost commit; an abort puts the
  entity back as it was and tells no one. An entity spawned and removed in the same
  transaction is never announced, so it never leaves.
- A spawn is refused, inside a transaction as outside one, once the entity ids run out.
- A transaction covers the entities of the level it was opened in; with transactions of several levels open (a command run `in` another level can open one), an entity belongs to the innermost one of its own level. A command it runs in another
  level (`execute in`) spawns or removes there as it would outside a transaction when no
  transaction is open in that level: the join and the leave are told at once, and an abort does
  not take the entity back. With one open there, the innermost of that level journals it, as above. A join listener that
  spawns, removes or commits a transaction while a commit is telling its joins is told about its
  own entities after the commit's batch, in the order it asked.
- Travel between levels is not journaled: an entity that arrives in another level during an aborted
  transaction stays where it arrived.

`mob_split` and `baby_entity_spawn` open a transaction after constructing the pending children.
An accepted list is validated and applied before deferred join callbacks run. A veto, empty list,
stale inputs (including a parent marked for removal), invalid child or contained listener panic
discards the children and rolls back journaled listener writes in that level. Work in another level
commits at once when that level has no open transaction. Construction draws stay spent, and the
parent's death or the parents' breeding cooldown is not undone.

For batched projectiles and thrown tridents, `projectile_impact` runs in the same transaction
as the captured hit. Canceling the hit does not abort listener writes. Discarding the
projectile makes its handle stale during the callback; the outermost commit notifies
`entity_leave_level` once if the entity was announced. If a listener aborts its own nested
discard and only cancels the hit, the original projectile stays alive.

## Transaction authority

`Transaction` identifies an open transaction with a game-issued write token. It
does not grant permission to close it. A write with a token that does not name an
open transaction changes nothing, including a token retained past its closure.

Closing requires a separate `TransactionSeal`. The `host.transaction` guard holds
that seal; its closure receives only the write token. If you use the raw
`transaction_open` method, you receive the seal and must close it with
`transaction_close`. The game refuses a seal that does not match an open
transaction and logs `Sunrise: refused to close a transaction the seal does not end`.

## Inspecting a transaction

These methods are server-only:

| Method | Result |
|---|---|
| `host.transaction_current()` | Innermost open `Transaction`, or `None`; not a closing seal |
| `host.transaction_depth(tx)` | `Some(0)` for the outermost transaction, increasing by one per nesting level; `None` if it is not open |
| `host.transaction_lifecycle()` | `TransactionLifecycle` describing the open or closing phase |

| `TransactionLifecycle` variant | Meaning |
|---|---|
| `None` | No transaction is open |
| `Open` | A transaction is open and none is closing |
| `Closing` | A nested transaction is committing into its parent or aborting |
| `RootClosing` | The outermost transaction is closing |

Implicit engine-managed container notices run during `RootClosing`, outside the transaction
whose commit triggered them. Explicit mod notices issued during a transaction run during `Open`.

## Entity inventory slots

`ItemPlace::EntitySlot { entity, slot }` addresses an entity inventory slot, like
NeoForge's `Capabilities.Item.ENTITY`. Read and write it like any other item place.
Reading or writing a chest minecart's or chest boat's slot unpacks its pending loot table
without a player (`ContainerEntity.getChestVehicleItem` / `setChestVehicleItem`), without
loading chunks. The fill preserves occupied slots. Reads and writes use the vehicle's live
menu slots if open.

The player `set_inventory_slot`, `set_ender_chest_slot`, `give_stack` and
`set_persistent_data_key` writes use the open transaction journal. Aborting restores
the old values; equipment effects and given-stack overflow wait for root commit.

These writes reach a player in another level than the hook's: their inventory and data
change at once, wherever they are. `give_stack`'s overflow drops, pickup visuals and sounds
go to the player's own level at root commit. `drop_stack` still returns the stack, `destroy_block`
breaks nothing, and no equip sound plays when the player's level is not in place.
