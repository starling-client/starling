# Query attachment holders

Use `AttachmentQueries` to find loaded holders of an `AttachmentKey<T>` without
checking every entity, chunk or block entity. Register your descriptor during
`Hooks::register` to obtain an `AttachmentKey<T>`, or look up a dependency's key
with `Registrar::attachment_key`, as described in
[server attachments](../attachments.md). Import `AttachmentQueriesMut` for server
writes.

## Read a snapshot

Call `host.query(key)` on `&dyn Host`, then create a fallible cursor with
`query.iter()`. Advance with `rows.try_next()?`; `Ok(None)` means the cursor is
in scope and has reached the end. Each row contains a copyable `holder` and lends
its value through `row.with_value(scope, |value| ...)`. Get `scope` from
`host.attachment_read_scope()?`.
Copy what you need inside the loan; a borrowed value cannot outlive it.
Match `AttachmentHolder` to obtain an `EntityRef`, `PlayerRef`, `BlockEntityRef`,
`ChunkRef` or `LevelRef`. Keep the batch and its holder identities in process;
use the [attachment codec](../attachments.md#synchronization) for values that
need to be saved or synced.

For example, register a counter descriptor and sum it across its holders:

```rust,ignore
use mod_api::*;

fn register_counter(registrar: &mut dyn Registrar) -> Result<AttachmentKey<u32>, String> {
    registrar.register_attachment_type(AttachmentDescriptor::new("example:value", || 0u32))
}

fn total(host: &dyn Host, key: &AttachmentKey<u32>) -> Result<u32, AttachmentRowError> {
    let query = host.query(key).map_err(AttachmentRowError::Value)?;
    let scope = host.attachment_read_scope()?;
    let mut rows = query.iter();
    let mut total = 0;
    while let Some(row) = rows.try_next()? {
        total += row.with_value(scope, |value| *value)?;
    }
    Ok(total)
}
```

| Rule | Behaviour |
|---|---|
| Value bounds | Iteration requires `T: Send + Sync + 'static`, not `Clone`. |
| Key liveness | `host.query(&key)` and `AttachmentBatch::query(&key)` reject keys from an earlier installation with `AttachmentError::StaleKey`. After reinstall, an already-bound query, cursor or row refuses reads with `AttachmentRowError::Value(AttachmentError::StaleKey)`. A nonempty batch cannot be bound to a key for a different bucket. |
| Snapshot | Membership and stored value bindings are captured at query creation. Create a fresh query to see later sets, removals, loads or unloads. |
| Borrowing | Rows borrow from the batch. Reading a captured value borrows the original host through its read scope, so you can mutate the host between reads, but not during a read loan. |
| Defaults | Queries never initialize defaults, dirty holders or queue synchronization. An absent key produces an empty batch. |
| Saved data | Saved values on bound holders decode lazily when visited, without an earlier typed read. A decode or type error advances past the offending row and returns `AttachmentRowError::Value`, not a default. |
| Interior mutability | A snapshot does not deep-freeze interior mutability inside `T`. |

For an immutable keyed read, including in a render hook, call
`query.get(scope, holder, |value| value.copied())?` with an `AttachmentHolder`
(for example, for a `u32` attachment). The expected O(1) lookup precedes decoding
and does not scan rows. Decode and type errors use `AttachmentRowError::Value`
without calling the closure. Return only the data you need; use `|value| value.cloned()`
if you need to keep an owned `Option<T>` for a Clone type.

A `None` loan means the holder is absent from the snapshot, including when its data
has not arrived through synchronization. It does not prove the holder is alive:
the probe does not check residency. While in scope, a retained query lends its
captured value even after a live replacement or removal.

## Write explicitly on the server

Call `host.query_mut(&key)` on `&mut dyn ServerHost`. Values require
`T: Clone + Send + Sync + 'static`. Each row provides `holder()`, `with_value(...)`,
`set(host, replacement)` and `remove(host)`; it does not provide `&mut T`.

```rust,ignore
fn increment(host: &mut dyn ServerHost, key: &AttachmentKey<u32>) -> Result<(), AttachmentRowError> {
    let query = host.query_mut(key).map_err(AttachmentRowError::Value)?;
    let mut rows = query.iter();
    while let Some(row) = rows.try_next()? {
        let replacement = row.with_value(host.attachment_read_scope()?, |value| *value + 1)?;
        row.set(host, replacement)?;
    }
    Ok(())
}
```

- Writes take effect through the ordinary attachment write path. Captured rows
  retain their original values; use a fresh query to observe a write.
- `set` and `remove` validate the row's write token through
  `host.attachment_validate_write(token)`. Outside the world phase it returns
  `AttachmentRowError::Unavailable`. In the world phase an expired capture
  returns `OutsideTransaction` before residency is checked, even if the
  current dimension has no index. Unloaded, replaced or missing holders
  produce `StaleHolder`. A host for another dimension does not retarget the
  row, and a refused write does not revive the holder. There is no byte
  encoding that can restore its write token.
- Removing only the attachment value does not invalidate a loaded holder. A
  retained row can set a replacement value on that holder.
- Writes join your existing transaction. For an all-or-nothing loop, import
  `HostTransaction` and use `host.transaction`, as described in the
  [transaction guide](../transactions.md).
  There is no implicit transaction or commit on drop. Dropping a batch neither
  applies deferred changes nor undoes successful writes.
- `remove` returns the ordinary attachment operation's `Option<T>` and preserves
  its codec errors.

## Scope and order

Queries cover only the host's current dimension and do not load chunks or
entities. Copying a holder identity does not keep it loaded. Attachments belong
to an entity object, not its saved UUID: equal-UUID objects have separate values.
An entity that is not admitted to the level is absent from queries.

A capture made in a transaction is usable only while that exact frame is current
on its owning game thread. A parent capture can be used again when its child
closes; a child capture cannot be used in the parent even after the child
commits. After commit or rollback, capture a fresh query. Using an out-of-scope
capture reports `AttachmentRowError::OutsideTransaction` for lookup, iteration,
value loans and writes. Each cursor step reports this refusal without advancing,
even for an empty batch. After the child closes, stepping the same parent cursor
continues at its next unvisited row. A value read through `with_value` or `get`
also requires a scope borrowed from the capturing host.
The scope prevents that host from opening or closing a frame during the call.
A capture made outside any transaction can be read with
`AttachmentReadScope::default()` without borrowing a host.

A query is a residency view, not an immediate liveness filter. An entity queued
for removal can appear until cleanup runs. Its handle's stale rules apply, and
stale writes fail. Do not convert a scoped handle to an unscoped storage ID to
bypass those checks.

Traversal uses dense index order: insertion appends, and removal fills its slot
with the final row. The same ordered mutation history produces deterministic
order, but that order is neither sorted nor stable across structural changes.

## Costs and type identity

Native acquisition performs one indexed lookup and clones a bucket `Arc`, not
`T`. Empty native acquisition allocates nothing. Warm borrowed iteration makes
no allocations; a first saved decode can allocate in the codec and read cache.

Retaining a snapshot pins its old values. A subsequent live mutation can copy
row metadata and shared pointers, without cloning each typed value. Retaining
many batches or interleaving snapshots and writes increases storage and copying
costs. No index sweep runs without an attachment or residency change.

A decoded value requires matching Rust type identity. Registering an attachment
once via `Registrar::register_attachment_type` and looking it up from other mods
via `Registrar::attachment_key` checks the Rust type at registration time; an
incompatible stored value still produces `AttachmentError::TypeMismatch`,
without falling back to saved decoding. Share types through your mod
dependencies rather than declaring a different type with the same attachment
key.

## Store errors

A first store on a resident entity whose saved map binds an owning lifetime
registers that lifetime and publishes the holder to the dimension's index: mobs,
dropped items, experience orbs, projectiles, armor stands, item frames,
paintings, boats, minecarts, dragon parts and general entity rows. Every other
family stays unindexed until its saved binding lands: the store is saved, but no
query visits it. Each dragon part has independent contents. Removing, replacing or
unloading the dragon retires its parts even when snapshots retain their values;
parking a level preserves its resident parts. Stores in construction listeners
are indexed even when inspection triggered construction. Inspection and removal
do not themselves register a holder.

| Error | Meaning and action |
|---|---|
| `AttachmentError::HolderBusy` | Physical storage is temporarily unavailable during an engine pass. Retry after the pass. The refused store writes no value and registers no holder. |
| `AttachmentError::UnsupportedHolder(family)` | The physical family cannot own attachments. Retrying does not help. `family` is a lowercase identifier, not an engine type. |
| `AttachmentRowError::HolderBusy` / `AttachmentRowError::UnsupportedHolder(family)` | The corresponding error from a retained row write. |
| `AttachmentRowError::StaleHolder` | An in-scope row's holder is missing or its captured residency is invalid. |
| `AttachmentRowError::OutsideTransaction` | The capturing frame is not current on this thread, or the value read did not borrow its original host. An expired frame is refused even if the current dimension has no index. |
| `AttachmentRowError::Unavailable` | This host operation is not available in the current phase. |

Handle these errors locally. If a message needs to report a refusal, give the
message its own payload fields rather than encoding the attachment error.
