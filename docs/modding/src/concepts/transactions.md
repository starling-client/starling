# Transactions

Use a transaction when related **journaled** changes should succeed or fail together. Import
`HostTransaction` and use the server host and borrowed `TransactionContext` from the same closure:

```rust,ignore
use mod_api::{HostTransaction, ServerHost, TransactionError};

fn update(host: &mut dyn ServerHost) -> Result<(), TransactionError<()>> {
    host.transaction(|host, tx| {
        // Pass this host and tx to each capability write.
        Ok(())
    })
}
```

`Ok(value)` requests a commit; you get the value back only if it commits. `Err(error)` aborts
and returns `TransactionError::Aborted(error)`. `Unavailable` means the closure never ran;
`CommitRefused` means it ran but the host rolled back its work. The closure takes
`&mut dyn ServerHost`, even when you open it on a concrete host.

Resident subject scopes also offer `transaction`: use it when the same operation needs both a
live subject view and other host access. End one borrow before starting the next and abort if
any required write or view is refused; see
[Combining subject and world writes in one transaction](../how-to/world/machine-menu-data.md#combining-subject-and-world-writes-in-one-transaction)
for a worked example. A server main-thread payload handler can use its context's
transaction to stage replies with its world writes; configuration-task completion remains immediate.
See the [API reference](../reference/api.md) and
[Scratch stacks and wrapped places](capabilities/item-places-and-components.md#scratch-stacks-and-wrapped-places)
for other scoped entry points.

## Commit and rollback

Writes are visible inside the transaction. The engine records storage before its first write in
that frame. A nested commit merges into the parent for that level; only the outermost commit
makes the changes final and publishes the deferred effects. Returning an error or unwinding out
of a frame restores its recorded state without publishing those effects.

| Storage | Examples of journaled state |
| --- | --- |
| Block entity | Item slots, data slots, saved data, one-shot client update fields, custom name and attachments |
| Hopper | Cooldown |
| Cauldron | Block state |
| Composter | Block state and its own random stream |
| World edits | Changed blocks and the block entities they replace, including block writes and fluid pickup or placement |
| Player | Inventory and equipment slots, cursor, ender chest and persistent data, including guest and fake players |
| Entity inventory | Vehicle and pack-animal slots |
| Item entity | Stack; an emptied item entity is removed at root commit |

The transaction is not a snapshot of the world. Level-local work joins only the journal for its
own level; a command that enters another level does not carry that journal along. Player-owned
storage can be restored even if the player moved, died or left before the transaction closed;
restoring the storage does not rewind their movement or death.

Default engine-managed storage defers its implicit change notices. Explicit changed, comparator,
neighbour and data-send notices are immediate and survive abort. Rollback writes do not issue
notices. Mod-owned memory, live level random draws, and direct operations such as non-player
`LivingRef::set_equipment` are not made reversible by a transaction. Use journaled
[capabilities](capabilities.md) instead when an operation must be reversible.

Ordinary block and fluid writes expose tentative states before publishing replacement effects.
Bulk block edits have their own publication rules; `destroy_block` prepares loot and experience
against the still-standing block, and its item entities, experience, break effects and game event
publish only on commit. `drop_resources` drops what `destroy_block` would without breaking the block
(no break effect, removal or game event, for a mod that breaks many blocks and sends the effect once);
in a transaction its loot is rolled at once and its item entities and experience wait for commit. See [Block Parts](parts/block-parts.md)
and [World fluid](capabilities/block-fluids.md#world-fluid) for examples.

## Deferred level, entity, and player effects

Server presentation effects such as sounds, particles and explicit level, entity or player
effects join the innermost transaction **in the level active at the call**. They publish in journal
order on that level's outermost commit, or disappear on abort. Delivery still uses the level named
by the request, even if that level is parked. A `game_event` for vibration listeners is different:
it runs at the call.

Arguments and handles are checked at the call. An accepted request does not promise a client
will receive it: a directed sound can lose its player before commit. Level-sound hooks run at
request time, with the transaction still open, so they can edit or cancel it. The commit delivers
the resolved record without running the hooks again. A deferred sound draws its seed only on
publication. See [Level sounds](../how-to/world/level-sounds.md),
[Level events](../how-to/world/level-events.md) and [Particles and movement queries](../how-to/client/particles-and-movement-queries.md).

## Spawning and removing entities

A spawn is available to handles and selectors immediately, but joins aren't announced until root
commit. Joins are processed in spawn order before removal effects. A refused join or aborted
spawn leaves a stale handle, not an ID that can be reused. Removing an entity makes it unavailable
immediately, while dismounts, leave and other removal effects wait for commit. Abort restores the
resident entity without a leave or join; a spawn removed within the same transaction is never
announced. Join notifications in the same level requested while a commit is announcing its joins
are delivered after that announcing batch, in request order. Travel between levels is not
journaled. See [Spawning](../how-to/world/spawning.md) and
[Hook notes](../reference/hook-notes.md).

## Transaction authority

`TransactionContext` is a loan, not a handle to store, send, share or encode. Use the context for
the **innermost** open frame on that host. Using an ancestor or a foreign context returns
`InvalidContext` before the handler runs, even for a zero-amount write. Resource helpers that take
an optional context use `Some(tx)` for the current frame or `None` to open a root when none is open.

## Joining an open transaction

Use `current_transaction` to borrow the innermost frame without opening a child. It returns
`None` without calling the closure if there is no frame. Returning or unwinding from the borrowed
closure does not close the frame. Use `transaction` when the work needs an independent savepoint.

## Inspecting a transaction

| Query | What it tells you |
| --- | --- |
| `tx.depth()` | Nesting depth, zero at the root; not proof that this context is current. |
| `host.transaction_matches(...)` | Whether the supplied context is current, or no frame is open when passed `None`. |
| `host.transaction_lifecycle()` | The server's process-local phase, not a transferable value. |

| `TransactionLifecycle` | Meaning |
| --- | --- |
| `None` | No transaction is open. |
| `Open` | A frame is open and none is closing. |
| `Closing` | A nested frame is committing into its parent or aborting. |
| `RootClosing` | The outermost frame is closing. |

Implicit engine-managed container notices run during `RootClosing`, outside the transaction that
triggered them. Explicit mod notices issued in an open frame run immediately, during `Open`.

For slot addressing and player writes, see [Entity item handling](capabilities/entity-items.md) and
[Server player services](../how-to/entities/server-services-and-players.md#inventory-ender-chest-and-saved-data).
