# Loading chunk attachments

`chunk_data_load` receives `&dyn ChunkDiskHost` and a `ChunkDataLoadEvent` after
a full or proto disk record has decoded, before its attachment store is
published. A missing record, fresh generation and proto-to-full promotion do
not fire this hook. The callback uses `event.data.get(&KEY)` and
`event.data.set(&KEY, value)` to read and edit the typed store of this load.
For example, reading disk value 8 and setting 9 publishes 9 on success.

The worker host supplies only the job identity and immutable resource reads.
Borrow the identity with `let job = host.job();` (`&ChunkDiskContext`).
`registry(name)` lends `Option<&[String]>` in registry ID order from the frozen generation
retained by the worker. `None` means that generation has no such registry. Use the slice within
the callback, or copy it if it must outlive the callback. Neither metadata getter reads files;
`ConfigHost::setting` still returns an owned `Option<String>` from the same snapshot.
The host has no live backend and is not a `ServerHost`. Typed values need a
persistence codec, not `Wire`. Unknown bounded attachment entries are preserved opaquely.

Codec errors, invalid or over-budget disk NBT, and panics refuse publication of
the job's edits. Later listeners still see the last accepted envelopes;
`event.data.failure()` does not expose an earlier listener's error to them. Within
one callback, each operation runs and reports its own refusal even when another
operation already failed. `failure()` is the first failure of this callback, not
its latest operation. The job reports its first terminal failure.
A phase refusal drops only that listener's proposal and is reported each time;
only a panic disables a callback for this hook for the session. If the job fails,
the chunk still loads with the
attachments exactly as they were stored. No
`chunk_data_save` listener runs for that chunk for the rest of the session, so
half-initialised state never replaces the stored record, and the chunk's later
saves write its stored attachments back unchanged. The load diagnostic names the
chunk and the mod. Fix the mod and restart before retrying a disabled callback.

The load runs on the thread that decodes the record, never with access to live
world state. The [disk NBT budgets](chunk-data-save.md#disk-nbt-budgets)
are the same on load and save.
