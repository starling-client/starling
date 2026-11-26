# Block entity types

Declare a block entity type and its valid blocks during `Hooks::register`, then attach its part:

```rust,ignore
let press = r.block_entity_type("press")
    .blocks(&["press", "mymod:big_press"])
    .build();
r.block_entity_part(&press, Press).expect("native block entity type");
```

The builder qualifies a bare type ID and each bare block name with the registering mod's namespace,
so `"press"` above is `mymod:press`. Explicit identifiers keep their namespaces; the blocks may be
declared later in `blocks`. The type defines the save ID, wire type
and valid blocks; `block_entity` returns only initial `Compound` data (or none for an empty compound). It is fired
inside the write that creates the entity (`LevelChunk.setBlockState`) and receives the read-only level, a `LevelSide`,
on a server and on a client of one.
Unknown blocks, duplicate types (including built-in keys), overlapping membership and attempts
to reassign an existing vanilla block are refused at load. Refused native block, item or block entity declarations stop startup, as does unreadable
known-components input; the game does not fall back to vanilla content. An explicitly declared mod
template block uses its declared type; undeclared template blocks retain their vanilla behavior. Adding mod blocks to vanilla types is not supported by this builder.
The hooks about the entity (`block_entity_tick`, `block_entity_update_tag`, `pre_remove_side_effects`,
`on_chunk_unloaded`, `menu_quick_move`) find it by that declared type; the others (`block_entity` and the block hooks) by block. The factory is a `BlockEntityPart` method, not a global `Hooks` method.

`r.block_entity_type_key("other:press")` looks up another mod's type, and
`r.block_entity_type_key("minecraft:chest")` looks up a vanilla type. Both return the same opaque
`BlockEntityTypeKey` as the declaration builder: equality and hashing use the shared load-owned
registration slot, not the type's blocks. Bare names use the `minecraft:` namespace. The final
registry resolves requests after all declarations and refuses unknown IDs, naming the requesting mod
and type. A key retains its identity until restart. `BlockEntityRef::kind(host)` returns
`Option<BlockEntityTypeKey>`: compare it with `Some(chest_type)` if you already have the key.
To read the canonical name, use `kind.id(host)`, which borrows it as `Option<&str>`.
Use a type ID, not a block ID (`minecraft:shulker_box`, not `minecraft:red_shulker_box`).

Lookup keys may answer capabilities for vanilla and native types. `block_entity_part` accepts only
native types and returns `BlockEntityPartError::VanillaType { kind }` for a built-in type, or
`InvalidKey { kind }` for an invalid key. Native Parts are expanded after every mod has registered,
so a Part can target a type declared later.

## Attaching to another package's type

A Part attaches by type id, whoever declared the type: `r.block_entity_type_key("owner:press")` then
`r.block_entity_part(&press, MyPart)`. Only a package that lists the declaring package in its
`[dependencies]` (required or optional) may do so, and that row must apply on the current
installation side and survive instance overrides. Otherwise load is refused by name,
even when that package is installed:
`mod b attached <Part> to block entity type owner:press, declared by package owner; declare owner as a dependency`.
A type that no package declares keeps the refusal `mod b attached <Part> to unknown native block entity type owner:press`;
when `b` lists the installed package that owns the type's namespace (its id or one of its manifest `namespaces`) as a
required dependency, the refusal says that dependency does not declare it. If the type's namespace is
the ID of an absent package, its Part and the associated key request are skipped only when an optional
dependency row applies on the installation side and survives instance overrides. A capability
provider using that key still requires the type. A present package must declare the requested type.
This keeps resolution independent of what else is installed. Parts of several packages on one type run in
dispatch order (`mod_index`, then registration order), the same on every load.

Declaring the type, `transform` and the type's initial data stay with the owner, and a vanilla type is still refused.
Block entity types can only be declared by a package's code, not by content files.

A `side = "server"` package whose Part on such a type listens to a client hook is refused like any other
client-side Part.

The Part's state (`block_entity_part_with_state`) uses an issued `AttachmentKey<T>` from
`register_attachment_type`. A dependent package can obtain the same key with `attachment_key`;
lookup does not declare a new attachment. The value is saved in the block entity's attachment
compound under the key's `(namespace, name)`. When the package is removed, its saved entry
remains unread and can be decoded if the package returns.

## Removal side effects

`pre_remove_side_effects` runs on the authoritative server before the retiring block entity
is removed and its neighbors are notified. `event.entity` lends the outgoing block entity's
actual data, and its attachments are still readable through the level host. The host permits
reads and random draws, not world writes. `UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS` (256) skips
this callback. It also runs when a menu button callback removes its backing block: don't
manually drop the container contents in the button callback.

Use `event.effects` to append contributions. The game applies them before finishing
removal, in this order:

1. Scatter the base container slots, unless a Part called `effects.set_keep_contents(true)`.
2. Scatter each `effects.drop_slot(Some(stack))`; `drop_slot(None)` takes the three
   position draws of an empty `Containers.dropItemStack` slot without spawning anything.
3. Create each `effects.spawn_item(stack, position)` at that position with the default
   pickup delay (10 ticks), as a lectern or jukebox does.
4. Apply each `effects.award_experience(amount, position)` via `ExperienceOrb.award`.

Within each group, contributions keep their order. Stacks use the normal checked item
boundary and positions must be finite. Each call returns its own error: one refusal
neither consumes a random draw nor loses any valid contribution. A panicking Part loses
its contributions without changing earlier Parts' effects. A Part that contributes nothing
doesn't cause any extra removal effects.

## Part state

A Part that keeps a value on each block entity of its type (a machine's progress, the last
position of a moving head) declares it with `BlockEntityPartState` instead of keeping a side
table. The game owns the values: one per block entity, in its attachment map, saved with the
chunk through the attachment codec and kept by transactions.

```rust,ignore
struct Press {
    progress: AttachmentKey<u32>,
}
impl BlockEntityPartState for Press {
    type Data = u32;
    fn state_key(&self) -> &AttachmentKey<u32> {
        &self.progress
    }
}
// Inside Hooks::register(&self, r: &mut dyn Registrar, _) (returns `()`):
if let Ok(progress) = r.register_attachment_type(
    AttachmentDescriptor::new("mymod:press_progress", || 0).serialize(PROGRESS_CODEC),
) {
    let _ = r.block_entity_part_with_state(&press, Press { progress });
}

// In its tick (or a menu hook), get the live handle from the view, then edit through the Host:
let Some(entity) = event.subject.with_view(|view| view.handle()) else {
    return;
};
let done = self.edit_state(event.subject.host(), entity, |_, _tx, progress| {
    *progress += 1;
    *progress >= 200
});
```

- `self.state(host, entity, |value| *value)` reads the counter from a snapshot through
  `&dyn Host`, lending a default without inserting it when absent. `state_existing` lends
  `Option<&T>` instead. `state_in` and `state_existing_in` do the same through `&dyn LevelHost`.
- `edit_state` takes `&mut dyn ServerHost` and opens a nested transaction. The closure gets the
  host, its `&TransactionContext`, and a mutable draft of the value. No store borrow is held
  while it runs. The draft is encoded and stored through that journal when the closure returns.
  `PartStateError::Unavailable` means the transaction could not open and the closure did not run.
  If the edit closure returns a `Result`, that result is its output, not a commit vote. To make
  a failed storage call abort the whole edit, propagate its error in an enclosing `host.transaction`.
- Every call checks the key and handle first. A stale key from a prior installation gives
  `PartStateError::Attachment { error: AttachmentError::StaleKey, .. }`. A handle from another
  level gives `PartStateError::OtherLevel`. A removed, replaced or unloaded block entity gives
  `Stale`. After a reload the saved value is read under the new handle.
- An edit stores nothing, and its transaction aborts with everything the closure wrote, in these
  cases:
  - the closure panics and the panic unwinds. A panic that aborts
    the process cannot be rolled back;
  - the value was replaced while the closure ran (`Changed`);
  - the value does not encode.
- An edit of the same block entity and key started inside the closure is refused with `Reentrant`
  and stores nothing. Only that inner edit is refused: the outer edit goes on and commits. An edit
  of another key is allowed.
- An undecodable saved value refuses before the closure runs and keeps its bytes.
- A transaction that aborts puts back the values, the chunk's unsaved mark and any block entity it
  removed, with its attachment map still bound. Handles of block entities made inside it stay
  `Stale`. Removed block entities retire only when the outermost transaction commits. For the
  gameplay entry points that defer saving and unloading during an open action savepoint, see
  [saving chunk attachments](../../how-to/world/chunk-data.md#saving-chunk_data_save).
- No size or depth limit of its own applies to a value. A saved value follows the chunk's NBT rules:
  vanilla 26.3 reads region files with `NbtAccounter.unlimitedHeap`, an unbounded byte budget and
  depth 512. The 2 MiB quota of a chunk packet's block entity data is a network limit and does not
  cap saved state.
- Registration reports `PartStateRefusal::Sync` for a synced key, `CopyOnDeath` for a key that
  copies on death, or `StaleKey` if the key was not issued in the active registration pass.
  The refusal names the Part, the key, the type and the reason.
- Values without a codec last for the session. Part state is server-side for now. A client reads the
  default, because nothing is sent to it yet.
- Part registration and state errors are local values, not `Wire` records.
  The attachment codec persists the state value, not its declaration or errors.

## Saved data

Implement `BlockEntityPart::block_entity_save_additional` to serialize live mod state when the server
captures this type's data. `event.tag` starts with the stored payload. Change that copy to write the
value that goes into a chunk, a command read, or a copy into an item, loot or a structure. For a
holder without declared inventory slots, this copy includes any saved `Items` as mod-owned NBT.
Even canonical-looking item rows keep their saved list order instead of being reconstructed in
slot order. The listener can replace or remove that field. For a declared container, the game
writes its inventory instead. Neither path changes the live entity or the separate client update tag.
Malformed payloads, including names or strings too long for NBT, are refused before any copy uses
that output. If the region writer cannot encode or admit the complete chunk
record, it retries with the stored block-entity payloads from the same snapshot. Other changes in
the snapshot are kept. The read-only level host cannot write to the world or request another save.
During unload or a removed block's loot roll, the entity
may no longer be readable through the host; the event still carries its payload, identity and its
position and level (`event.pos`, `event.level`), so a mod that keeps live state per dimension and
position can look it up and write it into `event.tag`.
`block_drops` and `player_destroyed_block` read the original entity's stored data, not the loot copy.
`block_drops` supplies a resident `block_entity` handle while the original is still in the level,
or lends `block_entity_data` if the original was removed or stored only in the chunk.
Changing output does not mark a chunk dirty: mark the entity changed when the live state changes.

`block_entity_on_load` is the separate once-per-instance attachment notice. It runs after the entity
has its initial data and before its first tick; it does not run on serialization.

`block_entity.replace_data(host, data)` replaces its saved payload, clearing omitted fields while retaining
identity and position. Like `merge_data`, it loads transactionally and synchronizes the change; it returns
`Err(BlockEntityError::Stale)` for an invalidated handle, or `Ok(false)` for data the entity cannot
load or an unchanged result.


### Reading data loaded by the server

Implement `BlockEntityPart::block_entity_load_additional` to refresh mod-owned state after the server
applies a saved payload, including `/data merge` and `/data modify`, cloning, structure and item
placement, falling-block landing, and chunk load. `event.tag` is the applied mod payload, and
`event.subject` is a `BlockEntityLevelReadScope<'_>`: `with_view` lends a read-only
`BlockEntityReadView` carrying the instance's handle, position, level, block state and updated data,
and `host()` borrows the read-only `LevelHost`. On a slotless holder, `event.tag` and
`BlockEntityRef::data` include any saved `Items` field in its original list order, even if its entries
look like inventory rows. `block_entity_on_load` follows when that
instance is new; it also fires for a fresh placement that had no saved data.

Direct Mod API `merge_data` and `replace_data` writes and transaction restores do not fire the
notice. Client sync instead uses `block_entity_data_received`.

## Transform saved data with a structure

Add an optional `.transform(...)` step to the declaration when the saved payload contains a facing
or relative positions that must turn with a structure:

```rust,ignore
use mod_api::{Compound, StructureMirror, StructureRotation, StructureTransform, Tag, TransformRefusal};

let press = r.block_entity_type("press")
    .blocks(&["mymod:press"])
    .transform(|data: &mut Compound, transform: StructureTransform| {
        let facing = match data.get(&"facing".into()) {
            Some(Tag::String(value)) => value.to_string()
                .map_err(|_| TransformRefusal::new("invalid facing text"))?,
            _ => return Err(TransformRefusal::new("missing facing")),
        };
        let mirrored = match (transform.mirror, facing.as_str()) {
            (StructureMirror::LeftRight, "north") => "south",
            (StructureMirror::LeftRight, "south") => "north",
            (StructureMirror::FrontBack, "east") => "west",
            (StructureMirror::FrontBack, "west") => "east",
            (_, facing) => facing,
        };
        let directions = ["north", "east", "south", "west"];
        let index = directions.iter().position(|facing| *facing == mirrored)
            .ok_or_else(|| TransformRefusal::new("unknown facing"))?;
        let turns = match transform.rotation {
            StructureRotation::None => 0,
            StructureRotation::Clockwise90 => 1,
            StructureRotation::Clockwise180 => 2,
            StructureRotation::Counterclockwise90 => 3,
            _ => return Err(TransformRefusal::new("unknown rotation")),
        };
        data.insert("facing".into(), Tag::String(directions[(index + turns) % 4].into()));
        Ok(())
    })
    .build();
```

The step runs on the authoritative server during structure-template placement, possibly on a
worldgen worker thread. It borrows this placement's working compound after the block state is
placed and any loot seed is set, before the block entity loads the data. Each placement starts
from the reusable, unedited template. Mirror applies before rotation. The only inputs are the
data, rotation and mirror: there is no `Host` or world access. Mods must ensure determinism,
including when using a `Send + Sync + 'static` closure; avoid randomness, clocks and mutable
external state when calculating the result.

Edit only the payload fields you need to change. The engine preserves the original `id` and any
original `x`, `y`, `z` fields. `TransformRefusal` logs the mod, block entity type and position each
time, without disabling the callback. Validate before editing if refusal should leave the data
unchanged: edits already made are not undone. A panic is logged and sent to the error toast and
disables that callback for the session; its partial edits likewise are not undone. Limits are
checked when data is actually serialized, not by the placement callback.

Callbacks are registered at startup and frozen with the block registry snapshot. A data-pack
`/reload` keeps that registry and its existing callbacks; it does not register replacement code.
A newly prepared registry receives its own callback snapshot.

## Load callback

Use `block_entity_on_load` to initialize a mod block entity after attachment and
saved-data application, before it ticks. The hook fires for fresh placement,
saved chunk loads and client chunk reception. It corresponds to NeoForge's
`IBlockEntityExtension.onLoad`.

The event supplies `event.subject: BlockEntitySideScope<'_>`: match
`BlockEntitySideScope::Server(scope)` for a mutable `BlockEntityView` and `&mut dyn ServerHost`
sequentially, or `BlockEntitySideScope::Client(scope)` for a read-only `BlockEntityReadView`
and `&mut dyn ClientHost` sequentially. Both views already see the applied saved data. Dispatch
is keyed by the block entity type ID.

- The hook fires once per attached identity, on each logical side. Writing a
  payload to an existing identity does not fire it again.
- Unloading and loading a chunk creates new identities and delivers new notices.
  Vanilla block entities do not receive this hook.
- Notices are drained before block entity ticking. Each drain handles only the
  notices pending when it begins; a listener's replacement block queues a notice
  for a later drain.
- If a pre-tick drain replaces the entity about to tick, that tick waits. The
  replacement receives its own notice before ticking.
- Worlds without subscribers collect no notices.

## Mod container openers

A declared mod block entity with a container menu uses the server's
`ContainerOpenersCounter`. Non-spectator players increment it when their menu
opens, and decrement it when the menu closes, including disconnects. Copper
golems count too. Each block entity owns its own counter. Native menus do not
count simply because they carry a position; a combined container counts both
block entities.

`mod_container_viewers` version 2 reports the previous and current counts
synchronously at these moments. The counter rechecks users in range five ticks
after the first open, then every five ticks while open; these rechecks report
counts even when unchanged. Open and close callbacks can therefore occur in the
same tick. The player is in place during their open or close callback; a recheck
has no acting player. Removing or replacing the block entity drops its pending
recheck without a viewer notice or a game event. A guest's open menu belongs to
that block entity instance, not its position: closing or rechecking the old menu
does not affect a replacement's count or viewer list.

A zero-to-positive transition posts `minecraft:container_open`; a
positive-to-zero transition posts `minecraft:container_close`. The opening or
closing entity is the source. A recheck has no entity source. These game events
reach vibration listeners just as vanilla container events do.

A mod can update its block state or play its own sound from the viewer callback.
Use `previous == 0 && viewers > 0` for an open edge and
`previous > 0 && viewers == 0` for a close edge.
