# Attachments

Declare every attachment during `Hooks::register`:

```rust,ignore
let _ = registrar.register_attachment_type("example:counter", false);
```

The registrar records any refusal even if the returned result is ignored.
The boolean states whether the attachment syncs to clients. A persistence codec
alone does not make it synced. Only a package that owns the namespace (through
its manifest or `served_by` delegation) may declare an id.
The owner's declaration allows other packages to use that id.
An undeclared use returns `AttachmentError::Undeclared` with the calling package
id and attachment id. Synced declarations refuse a `side = "server"` package
(or a `side = "both"` host declaring into a `side = "server"` served package's
namespace) before registration is installed. Here “carries a synced attachment” means
**declares** one: a server package may use A's synced attachment if A is a
client-compared package. If A is itself a server package, A is refused.
Load checks declaration conflicts (including foreign namespaces) first, then
undeclared uses naming the caller, then server-package declarations.
Declarations must match the key's sync policy.


Use `AttachmentType<T>` to store mod data on a player, entity, block entity,
chunk or level. Declare a type once, then pass the hook's host and a `Holder`
to read or update its value.

```rust,ignore
use mod_api::{AttachmentCodec, AttachmentError, AttachmentType, Compound, Holder, Tag};

static COUNTER: AttachmentType<i32> = AttachmentType {
    namespace: "example", name: "counter", default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("count".into(), Tag::Int(*n))].into()),
        decode: |data: &Compound| match data.get(&"count".into()) {
            Some(Tag::Int(n)) => Ok(*n), _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};
// Inside a server hook, or mixin code, with `host: &mut dyn ServerHost`:
if !COUNTER.has(host, Holder::Chunk([0, 0]))? { /* no counter stored */ }
let old = COUNTER.get(host, Holder::Chunk([0, 0]))?;
COUNTER.set(host, Holder::Chunk([0, 0]), old + 1)?;
Ok::<(), AttachmentError>(())
```

## Reading and writing

Call `get`, `set`, `remove`, `has` and `get_existing` with the host and holder. `T` is
`Clone + Send + Sync + 'static`; `get` returns a clone and inserts the default on
first access. `has` and `get_existing` read without inserting (NeoForge's `hasData`
and `getExistingData`): `has` says whether a value is stored, saved or set, even
before it decodes, and `get_existing` returns `None` instead of the default. Neither
adds anything to the holder's saved data. `set` runs the optional codec before
changing the value, returning an `AttachmentError` if it cannot encode.
Keys are the pair `(namespace, name)`; a key already decoded as another Rust type is
an error. On a server, a missing holder is an error, not a new phantom holder; a client
keeps a copy for any holder it is asked about (see the sync paragraph below).
`AttachmentError` distinguishes `MissingHolder`, `TypeMismatch`, `Decode`, `Encode`,
`InvalidNamespace` (a malformed saved compound), `InvalidCopyPolicy` (death-copy
without a codec) and `InvalidSyncPolicy` (sync without a codec), `HolderBusy` (storage temporarily
unavailable; retry after the pass) and `UnsupportedHolder` (the holder family has
no attachment storage; retrying does not help).
`host.attachments()` exposes
the host's `AttachmentStore`. Reading clones the value; copying a holder's data
shares its stored values without cloning them.

`AttachmentHolder` keeps an existing typed handle when a system needs to store a holder
identity: `Entity`, `Player`, `BlockEntity`, `Chunk` or `Level`. It is `Copy`, does not keep
anything loaded, and follows each handle's ordinary stale-world and unload rules. Unlike
the storage-relative `Holder`, it carries the handle's world scope.

## Querying holders

Import `AttachmentQueries` to use `host.query(&TYPE)` on either host side. The
returned batch is a **snapshot** of membership and stored value bindings in the
host's current dimension. Create a cursor with `query.iter()` and step it with
`rows.try_next()?` until it returns `Ok(None)`. Borrow each value with
`row.with_value(scope, |value| ...)` without cloning it. Get `scope` from
`host.attachment_read_scope()?`. Iteration decodes saved entries lazily; malformed
or incompatible values produce row errors, not defaults. Later sets, removals and unloads appear only in fresh queries. This
is a residency view: an entity queued for removal can appear until the removal sweep. Captured handles do not keep holders loaded, and stale writes fail.
Interior mutability inside `T` is not deep-frozen by a snapshot.

Batches, holder selectors and write tokens stay in the running game; they are
not `Wire` values. Persist or sync the attachment value through its registered
`AttachmentCodec`, not by encoding the query or a holder's residency.

On a server, import `AttachmentQueriesMut` for `host.query_mut(&TYPE)` (`T: Clone`).
Each borrowed row has `holder()`, `with_value(...)`, and explicit `set(host, replacement)` /
`remove(host)` methods. These validate residency against the host's current
dimension and then use the ordinary attachment journal. There is no `&mut T`,
implicit transaction, or commit on drop: wrap a system in `host.transaction` for
all-or-nothing writes. Unload or replacement produces `AttachmentRowError::StaleHolder`.
Rows retain captured values even after successful write-through updates. A capture
made during a transaction can only be used while its capturing frame is current.
A value read borrows the capturing host for the whole call, so the frame cannot
close or open a child during that read. After commit or rollback, create a fresh
query; using the old one returns `AttachmentRowError::OutsideTransaction`. If a
child frame temporarily makes the capture unavailable, that refusal does not
advance the cursor. Continue with the same cursor when the parent is current again.

Traversal follows dense index order: insertion appends, and removal fills its slot
with the last row. The same ordered mutation history is deterministic, but the
order is neither sorted nor stable across structural changes. Retaining a batch
can make the next live mutation copy row metadata and shared pointers once;
retaining many batches keeps their old bindings alive. Values from an incompatible Rust type domain report `TypeMismatch` rather than
falling back to saved decoding.


## Holders and persistence

Use `Holder::Level`, `Holder::Chunk([x, z])`, `Holder::BlockEntity([x, y, z])`,
`Holder::Entity(id)`, or `Holder::Player(uuid)`. Entity IDs cover every entity in
the level apart from players: mobs, items, experience orbs, projectiles, tridents,
falling blocks, primed TNT, fireworks, boats, minecarts, armor stands, cushions, item
frames, paintings, End crystals, the ender dragon, ominous item spawners and the
server's `OtherEntity` records. An entity keeps its UUID through saves, so its saved
attachments load back with it; an orb saved with a `count` above one loads as several
orbs, and only the first keeps the attachments. Player UUIDs cover online guests and
the acting player. Mutable level
hosts can access their level, loaded chunks and block entities; read-only level hosts
cannot access attachments. Unserialized values disappear on unload or restart. A saved
value is stored under `sunrise:attachments`, then the mod namespace and attachment name,
as `{value: <compound>, copy_on_death: 1b, sync: 1b}` (a flag is omitted when false).
Unknown mods' saved entries survive load and save without being decoded. Level
attachments use the dimension's `data/sunrise/attachments.dat` SavedData
file, not `level.dat`.

## Synchronization

You cannot supply a holder-aware default, custom copy handler, conditional serialization,
recipient predicate or partial synchronization updates. The default is `fn() -> T`;
copying uses the declared policy, and synchronization uses the disk codec rather
than a separate wire codec.

A type with `sync: true` sends its value to Sunrise clients when `get` creates the
default, on `set` and `remove`, and on demand with `AttachmentType::sync_data(host,
holder)`; changes made in one tick are sent once, at its end. The recipients are the
players who track the holder: guests who were sent a chunk for a chunk or block entity,
who track an entity for an entity, the player and those tracking them for a player, and
everyone in the level for the level. A viewer held during a proxy handoff receives initial
pairings, including entities that spawn between tracking passes. It can read updates and
removals for tracked entities and other players through the same `Host::attachments` route
before placement, without requesting a resync. A client also gets all
of a holder's synced values when it starts seeing it: a chunk sent (with its block entities),
a level joined or re-entered, an entity tracked, and its own player. The wire form is the
codec's compound, so `sync` requires a codec.

Values travel on `sunrise:attachment_sync`, an engine declaration in the negotiated
payload table (play, to the client, version 1, optional): only a client whose connection
offered it is sent values, so a vanilla client is sent none. A message is prepared against
the player's current connection epoch and published at the root commit, in order with the
player's other packets; a rolled-back edit sends nothing, and a message prepared before a
reconfiguration never reaches the new one. Its size is bounded by the connection's frame
limits, not by a cap of its own. A message with a namespace or name over 256 UTF-16 code
units, a value that cannot be written, or one past the frame limits is not sent, and the
refusal is logged by name each time; the player stays joined. A full outgoing
queue closes that player's connection.

On a client, `Host::attachments` returns a copy of what the server synced, which `get`
decodes; `set` and `remove` change only that copy, which is dropped when the chunk,
entity or player goes. A client keeps no copy for the level hosts (`LevelRandomHost`):
they stay server-only there, though their changes on a server are synced.

A general entity row (`declare_general_entity`) has no data of its own on a client: its
synced attachments are what a Sunrise client knows of it beyond its type, position and
rotation. They arrive with the row, when the client starts tracking it, and go with it. Its
[renderer](../how-to/client/entity-renderers.md#a-general-entitys-frame) reads the last
published whole generation rather than consulting the client holder. A refusal to publish an
over-budget render generation does not change the client's synced copy.

## Copying on respawn and conversion

`copy_on_death` requires a codec. On death respawn, only opted-in (`copy_on_death`)
entries with a codec — including raw entries of an absent mod — are copied to the
replacement via their encoded representation; other entries stay only on the retained old
player. On return from the End after credits (`was_death = false`), every serializable
entry (any entry with a codec, including raw entries of an absent mod) is copied to the
replacement via its encoded representation, while entries without a codec stay only on
the retained old player. In both cases the copy runs even when no mod subscribes to
`player_clone`, and the replacement starts from fresh encoded entries rather than sharing
decoded instances with the old player. A mob's conversion (a villager struck into a witch,
a zombie villager cured, a piglin zombified, a tadpole grown) copies every entry with a
codec, raw entries of an absent mod included, to the new mob through its encoded form,
over what its construction set; entries without a codec stay on the removed mob. A
cancelled `living_conversion_pre` copies nothing, `living_conversion_post` sees the copy,
and the copy is not synced on its own (the new mob's spawn carries its synced entries).

## Reading the original player

`player_clone` supplies `event.original: OriginalPlayer<'_>` alongside the replacement's
`event.player` UUID. During that hook, `event.original.attachment(&COUNTER)` reads the old
player's value even when `copy_on_death` is false or the type has no codec, returning its
default without inserting it if absent; `COUNTER.get(host, Holder::Player(event.player))` reads
the replacement's retained value or fresh default. This works on death and on return from
the End. The original's `has_attachment`, `existing_attachment` (decoding and caching a
saved entry once without inserting a default), `with_decoded_attachment`, `set_attachment`,
and `remove_attachment` work on the retained old player without touching the replacement.
Cache writes, explicit writes and removals on the original are journaled to `host.transaction`,
so an aborted transaction restores both holders together. The replacement is already assigned its final respawn level, including
any `player_respawn_position` redirect. It has not been placed or published in the bulk
attachment index yet. Requery after placement: a bulk write token for the old player is
stale even though the replacement reuses that player's UUID.

The same `OriginalPlayer<'_>` view exposes `uuid`, `position`, `dimension`, `level`,
`health`, `experience`, `effects`, `persistent_data`, and `with_inventory` for the pre-copy
player. `with_inventory` borrows an `InventoryView` using vanilla `Inventory` slot numbering:
0–35 for the main inventory (0–8 hotbar), 36–39 for boots through helmet, 40 for the off
hand, 41 for body, and 42 for saddle. Rust's borrow checker binds `event.original` to the
callback lifetime, preventing the view from outliving `player_clone`. With no `player_clone`
listener, no original player state is retained.
