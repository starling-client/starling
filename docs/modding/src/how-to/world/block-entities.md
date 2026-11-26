# Block entity hooks and helpers

## Neighbour changes

A block's `BlockPart::on_neighbor_change` receives the changed block entity's
position in `neighbor`. On authoritative servers, the comparator invalidation
walk visits the four horizontal neighbours, skipping unloaded adjacent chunks.
Vanilla comparator processing at each cell runs before its part is notified.
This is separate from `neighbor_changed`, the vanilla block neighbour callback.

A block one cell farther through a redstone conductor receives this notice only
when its `weak_changes` part sets `receive` to true (initially false). This query
belongs to the farther block. Reading that cell never loads a chunk.

These correspond to NeoForge's `IBlockExtension.onNeighborChange` and
`getWeakChanges`. With no parts, vanilla's comparator rules are unchanged.

## Extending block entity validity

Use `Hooks::block_entity_type_add_blocks` after retaining block and type keys from
`register`. `event.modify(kind, &[block])` extends one existing type;
`event.modify_matching(predicate, &[block])` extends all matching types in `event.types`
order. Read `event.types` to inspect the registered type keys, not other listeners' additions.
The methods only add memberships; a listener cannot edit or remove an earlier addition.
The event uses a `RegistrationContext` and runs after content declarations, before worlds
load. At freeze, duplicate additions are harmless. Unknown keys and blocks that do not
share the target's common vanilla superclass fail startup.

A block may be valid for several entity types. Its native declaration or original
vanilla template selects creation and ticking. For example, a block declared
with `template: "minecraft:vault"` and added to `minecraft:vault` creates and saves the
vanilla vault entity. Adding another valid type does not replace that factory. Sign
templates retain their existing membership defaults. Validity is frozen with the
canonical type table and participates in the multiplayer content identity.

## Dropped items during block-entity ticks

Use the tick scope's Host to read or write other objects. A dropped item's
`ItemEntityRef::set_stack` replaces its complete nonempty stack;
`EntityRef::discard` or `EntityRef::remove` removes the item. A removed item
stops resolving immediately. A numeric entity ID never retargets a
replacement or an item in another level.

Use `item.with_stack(host, |stack| stack.count())` to inspect a dropped item's stack without
copying it. For components or an owned snapshot, use
`item.with_stack(host, |stack| stack.project()).flatten()`.

If both the item and the ticking block entity must change or neither may
change, make both edits in `event.subject.transaction`. Take the Host for the
item write, then re-take the subject view. Return `Err` if the item operation
fails or the subject is unavailable; do not continue on a replacement. Item
stack writes and removals use the ordinary journal, and abort restores them
along with the subject's storage. Commit delivers the dropped item's removal
notice. With no transaction, these writes are immediate; there is no tick
batch or final write-back.

Pickup delay and target changes, spawning, motion and non-item removals are
not covered by the dropped-item journal. The transaction is not a whole-world
snapshot.
