# Items by kind

Use `STORAGE` (`sunrise:storage`) to move items by kind in bulk, rather than by slot.
Use `ITEMS` (`Capabilities.Item.BLOCK` in NeoForge) when you need slot-indexed access.
NeoForge 26.3's transfer API has no by-kind counterpart to `STORAGE`.

A block's `ItemStorage` answers by kind, with `u64` amounts:

- `amount(host, resource)` and `space_for(host, resource)`: how much of one kind it holds, and the planning bound
  for that kind. The default handler decides offer-dependent admission on the actual insert, not in `space_for`.
- `insert(host, tx, resource, amount)` and `extract(...)`: writes in the caller's transaction, never more than asked.
- `visit_entries(host, &mut visitor)`: calls `visitor(&kind, amount)` for each occupied entry, in provider order.
  For a container, this is the order of the slots exposed on the queried side, not necessarily physical slot order.
  Each slot has its own amount, even when several hold the same kind.

**Who answers it.** The game answers `STORAGE` for every container or composter port `ITEMS` reaches. Its own containers are asked in
one pass over the slots the side reaches, by the rules hoppers follow, so a chest, a hopper, a composter and a furnace give the same
answers through either capability. A container whose `ITEMS` a mod answers is wrapped slot by slot. A mod's bulk store
(a drive, drawers, a vault) answers `STORAGE` with a provider of its own, one entry per kind however many items it
holds: register it `before_game` if the block also has slots.

## Finding an entry

To find the first entry with at least `minimum` items, stop as soon as it matches:

```rust
use mod_api::{ItemResource, ItemStorage, ServerHost, StorageError};
use std::ops::ControlFlow;

fn first_at_least(
    storage: &dyn ItemStorage,
    host: &mut dyn ServerHost,
    minimum: u64,
) -> Result<Option<(ItemResource, u64)>, StorageError> {
    let mut found = None;
    storage.visit_entries(host, &mut |kind, amount| {
        if amount >= minimum {
            found = Some((kind.clone(), amount));
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })?;
    Ok(found)
}
```

Providers skip empty entries and report repeated kinds separately. Read only as far as the visitor asks:
`Continue(())` requests the next entry, and `Break(())` ends the visit with `Ok(())` without reading another.
Exhaustion also returns `Ok(())`; a read failure returns `Err`.

The provider may hold a read borrow until the visit returns. Use each kind within its callback or clone it
as above; do not re-enter the provider or write inside the callback. Act on the result only after the visit succeeds.

## Moving items

`transfer(host, from, to, resource, max, tx)` moves one kind: all or nothing, a partial take
retried, and `StorageError::Overanswered` when a handler answers more than it was asked.
It queries the source amount and destination space before extracting and inserting.
A partial move can require additional calls. `transfer_matching(host, from, to, filter, max, tx)` moves up to `max`
items accepted by the filter. It does not read the source when `max` is zero. Otherwise it visits the source once
in provider order to collect matching kinds (deduplicated by kind, since `transfer` moves across the whole storage
by kind), lets the read borrow end, and transfers each collected kind once in first-seen order until `max` items
have moved or the list ends. Totals that do not fit in 64 bits return `StorageError::Overflow` rather than
wrapping.

**Cost.** A bulk store can use a by-kind index to avoid scanning slots. The game's adapter answers in one pass over the slots
(O(slots)), with no work per item moved, so a question about a chest costs the same whether it moves one item or a
stack. A wrapped mod `ITEMS` handler costs one handler call per slot.

## Choosing a capability

Use `ITEMS` and `resource_util::move_resources` for slot-specific operations and small transfers between
ordinary containers. Use `STORAGE` for bulk stores and by-kind routing. The cost depends on
the provider and the slots it exposes; `STORAGE` does not guarantee constant-time access.
