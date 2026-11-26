# Loading and saving chunk attachments

Both hooks receive `&dyn ChunkDiskHost`, which supplies only the job identity (`host.job()` ->
`&ChunkDiskContext`: world epoch, dimension, coordinates, full/proto kind and phase) and immutable
resource reads. It grants no live chunk lookup, scheduling, packets or world writes.
`registry(name)` lends `Option<&[String]>` in registry ID order from the frozen generation retained
by the snapshot (`None` means that generation has no such registry; copy it if it must outlive the
callback). Neither metadata getter reads files; `ConfigHost::setting` returns an owned
`Option<String>` from the same snapshot. The host has no live backend and is not a `ServerHost`.

`event.data` is a typed, job-local attachment store. Use its `get`, `set` and `remove` methods
with an issued `AttachmentKey<T>` whose descriptor has a persistence codec (`.serialize(codec)`);
`T` does not need `Wire`, and disk operations never invoke `sync_codec`. A codec error, missing
persistence codec, stale key, or envelope that the ordinary saved-record codec cannot write and
read returns `Err` for that operation without modifying stored envelopes or poisoning unrelated
edits; inspect `err.reason` for the typed `ChunkDiskFailureKind`. To undo a group of edits when an
operation returns an error, return that error from `event.data.transaction(...)`. A panicking
listener loses only its edits and is disabled for that hook for the session; the log and local
error toast name the mod, and edits from the other listeners still publish. Unknown attachment
entries are preserved opaquely.

## Loading (`chunk_data_load`)

`chunk_data_load` fires with `ChunkDataLoadEvent` on the owning server thread after a full or proto
disk record has decoded, before its attachment store is published and the chunk is admitted. A
missing record, fresh generation and proto-to-full promotion do not fire this hook. Use
`event.data.get(&KEY)` and `event.data.set(&KEY, value)` to read and edit the typed store (for
example, reading disk value 8 and setting 9 publishes 9 on success). The ordinary disk reader
decodes the input before this hook, and a load callback cannot suppress the save hook.

## Saving (`chunk_data_save`)

`chunk_data_save` fires with `ChunkDataSaveEvent` on the owning server thread after capture,
before the snapshot goes to background encoding. To serialize a mod block entity's live state
into its own payload, use
[`block_entity_save_additional`](../../concepts/parts/block-entity-types.md#saved-data) on the
server thread, before the disk worker sees the capture. If block-entity serializer edits cannot
be written, the worker retries with stored block-entity payloads and keeps the result of
`chunk_data_save`. Capturing 7, writing 8 in the callback and later writing 9 in the live chunk
produces disk value 8 without changing live value 9.

A captured record caches its callback result, so disk encoding and pending reads reuse it and
never invoke the callback twice. A callback panic does not stop the save: the remaining
listeners' edits go to the ordinary writer.

Every captured record runs the save callback once, for full saves (explicit, forced, late, unload
and closing) and proto saves alike. The writer may coalesce older captures without writing them.
Gameplay save entry points check that the entity pass is settled. When a save listener is
installed, eager saves and the normal streaming/unload pass also defer while an action savepoint
holds the pass open. The low-level record capture and synchronous disk write do not themselves
impose a transaction gate; a callback observes only the record its caller captured. A record that
the writer cannot encode or write is reported with its coordinates and storage type. A flush
waits for all attempts to finish. An individual record failure does not fail the command, timer
save or shutdown, but a failed region sync fails the flush. The server does not retry the failed
snapshot; a later edit can be saved in a new snapshot.

## Disk NBT validation

Attachment edits use the normal saved-record NBT writer and reader rules, at
`sunrise:attachments`'s actual nesting in the record. A bad envelope does not stop a valid sibling
edit or terrain from saving. There are no separate attachment budgets. The ordinary writer still
encodes the complete record; an ordinary record failure leaves the prior record intact without
dropping neighboring records. As in vanilla `NbtIo.StringFallbackDataOutput`, strings too long for
modified UTF-8 are written empty.
