# Dropped items during block-entity ticks

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
