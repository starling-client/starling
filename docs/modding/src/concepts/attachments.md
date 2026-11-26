# Attachments

An attachment stores typed mod data on an object instead of in a map your mod has to keep in
step with the world. Describe the type and its policy with an `AttachmentDescriptor<T>`, declare
it during `Hooks::register`, and keep the `AttachmentKey<T>` it issues on your mod or Part.
Address the object with a `Holder`:

```rust,ignore
let counter = registrar.register_attachment_type(
    AttachmentDescriptor::new("example:counter", || 0).serialize(codec),
)?;
```

The registrar records refusals even if you ignore the result. Any valid lowercase
`namespace:path` ID not already declared is accepted, and the submitting package becomes its
declaring owner, whatever the key's namespace; a package that depends on the owner looks up
the same typed handle with `registrar.attachment_key::<T>("namespace:name")`. An undeclared
use reports `AttachmentError::Undeclared`. `.serialize(codec)` persists, `.sync(codec)`
synchronizes to clients and `.copy_on_death()` copies on respawn, independently of each other;
the last needs a persistence codec. Declaring a synced type requires a client-compared package;
a `side = "server"` package can use another package's synced type but cannot declare its own.
A key belongs to the installation that issued it: one kept from an earlier installation is
refused with `AttachmentError::StaleKey`.

## Reading and writing

Reads and `set` need `T: Send + Sync + 'static`; `remove` also needs `Clone` to return the
previous value. `get` inserts the default when no value is stored and lends `&T` to a read
closure, which returns only what it projects; copy what you need beyond the call. To inspect
without creating a default, use `has` or `get_existing`; neither allocates a holder map, and
`has` can report a saved entry even before it's decoded. `set` runs the persistence codec, if supplied, before changing the stored
value. The same ID cannot be decoded as two different Rust types.

```rust,ignore
use mod_api::{AttachmentKey, Holder};

// In a server callback with its host and the mod's key, for a resident chunk:
let chunk = Holder::Chunk([0, 0]);
let next = counter.get_existing(host, chunk, |value| value.copied())?.unwrap_or_default() + 1;
counter.set(host, chunk, next)?;
```

A missing server holder does not create storage. Distinguish a missing or temporarily busy
holder from one whose family does not support attachments at all. See `AttachmentError` and
`AttachmentKey` in the [generated API reference](../reference/api.md).

## Holders and persistence

A `Holder` addresses a level, chunk, block entity, non-player entity ID or player UUID in the
relevant storage scope. To retain a world-scoped identity, use `AttachmentHolder`, which carries
the typed handle; it does not keep the object loaded. Mutable level hosts can reach their level,
loaded chunks and block entities. Read-only level hosts cannot access attachments.

Call `.serialize(codec)` on the descriptor to persist the value as a compound: saving encodes the
value as it is then, including changes made through its interior mutability. Without a codec the
value is lost on unload or restart. Unknown mods' encoded entries survive load and save without
being decoded. Holder data is stored under `sunrise:attachments`, by namespace and name; each level
uses the dimension's `data/sunrise/attachments.dat` file rather than `level.dat`. See
`Holder::Entity` in the [generated API reference](../reference/api.md) for entity persistence
details.

## Querying holders

`host.query(&key)` captures resident holders and their stored value bindings in the host's current
dimension. Rows decode lazily and borrow the captured value; a malformed entry is an error, not
a default. Later writes and unloads appear only in fresh queries. A capture doesn't keep holders
loaded or deep-freeze interior mutability. Traversal is not sorted or stable across structural
changes. For usage and costs, see [Query attachment holders](attachments/querying.md).

On the server, `query_mut` gives explicit write-through operations that recheck residency and
use the ordinary journal. No mutable reference or implicit commit is lent. Wrap a group of edits
in a [transaction](transactions.md) if it must succeed together. A query captured in a transaction
can be used only while its frame is current; requery after it closes. Do not persist or encode
batches, selectors or write tokens.

## Synchronization

Persistence alone does not make a value synced. Set `.sync(codec)` on the descriptor: syncing
uses that codec's compound, which can differ from the saved one. Defaults created by `get`,
writes, removals and explicit `sync_data` requests are sent at the end of the tick in the order
accepted, with a block entity's changes coalesced to their final state; rolled-back changes are
not sent. Removing an absent synced key, or syncing one that holds no value, sends a removal
marker that also clears a client's local default. Clients receive the current value when they
start tracking its holder. A sync codec that panics is disabled for the session, and the mod is
reported in the log and in a local error toast.

Recipients are the clients that track the entity or chunk, the player and their trackers for
player data, or everyone in the level for level data. The connection must negotiate attachment
sync; vanilla clients get no attachment data. A client reads its local copy through the host.
Client writes affect only that copy, not the server; tracked copies go away with their holders.
Restricted level hosts do not expose client attachment copies. The default takes no holder,
and there is no custom copy handler, per-recipient predicate or partial update. For send limits,
failure outcomes (logged drops vs. queue disconnect) and connection-epoch behavior across
reconfigurations, see `AttachmentKey::sync_data` in the
[generated API reference](../reference/api.md).

## Player construction and saved data

On the server, `entity_constructing` runs with a fresh player attachment map. Set
initial values there without reading the player's saved state. On join, disk entries
overwrite matching constructor entries before `player_load_from_file`. On death or End
credits respawn, the eligible entries from the old player overwrite matching constructor
entries before `player_clone`. Entries set by the constructor that the load or copy does
not replace stay on the new player, including entries without a codec.

## Copying on respawn and conversion

| Replacement | What is copied |
| --- | --- |
| Player respawns after death | Only entries whose descriptor sets `.copy_on_death()`, which requires a persistence codec. |
| Player returns from the End after credits | Every serializable entry, regardless of `.copy_on_death()`. |
| A mob converts into a different mob | Every serializable entry; a cancelled conversion copies nothing. |

Copies use the encoded representation, including eligible raw entries from absent mods. The
replacement does not share the old decoded instances. Player copying does not depend on a
`player_clone` listener; after conversion the new mob's spawn supplies its synced data.

## Reading the original player

During `player_clone`, `event.original` lends the old player while `event.player` names the
replacement. Use the original view to read data that wasn't copied, including transient
attachments; a default read there doesn't insert it. Original-player writes can join a host
transaction, but change only the old player. The view cannot outlive the callback.

The replacement already has its final respawn level but isn't yet published in the bulk
attachment index. Requery after placement: an old write token is stale even though the UUID is
reused. See `OriginalPlayer` in the [API reference](../reference/api.md) and the
[Hook notes](../reference/hook-notes.md) for the callback's available views.
