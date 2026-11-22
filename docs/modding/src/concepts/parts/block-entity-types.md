# Block entity types

Declare a block entity type and its valid blocks during `Hooks::register`, then attach its part:

```rust,ignore
let press = r.block_entity_type("press")
    .blocks(&["press", "mymod:big_press"])
    .build();
r.block_entity_part(&press, Press).expect("native block entity type");
```

The builder qualifies the type ID and each bare block name with the registering mod's namespace, so
`"press"` above is `mymod:press`; the blocks may be declared later in `blocks`. It owns the save ID, wire type
and valid blocks; `block_entity` returns only initial `Compound` data (or none for an empty compound). It is fired
inside the write that creates the entity (`LevelChunk.setBlockState`) and receives the read-only level, a `LevelSide`,
on a server and on a client of one.
Unknown blocks, duplicate types, overlapping membership and attempts to own a `minecraft:` block are
refused at load. Refused native block, item or block entity declarations stop startup, as does unreadable
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
native types and returns `BlockEntityPartError::VanillaType { kind }` immediately for a vanilla key. Native
Parts are expanded after every mod has registered, so a Part can target a type declared later.

## Attaching to another package's type

A Part attaches by type id, whoever declared the type: `r.block_entity_type_key("owner:press")` then
`r.block_entity_part(&press, MyPart)`. Only a package that lists the declaring package in its
`[dependencies]` (required or optional) may do so, and that row must apply on the current
installation side and survive instance overrides. Otherwise load is refused by name,
even when that package is installed:
`mod b attached <Part> to block entity type owner:press, declared by package owner; declare owner as a dependency`.
A type that no package declares keeps the refusal `mod b attached <Part> to undeclared native block entity type owner:press`;
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

The Part's state (`block_entity_part_with_state`) is a key the attaching package declared with
`register_attachment_type`, which accepts any namespace that package owns (its manifest namespaces
or a content package it serves via `served_by`). It is saved in the block entity's
attachment compound under that package's id, not under the owner's. A Part reaches the owner's state only through an
attachment the owner declared (the public attachment API); a key nobody declared is refused at load, naming the
package. When the attaching package is removed, what it saved stays in the block entity, unread (the saved
attachment compound keeps entries it does not know on save), and is read again if the package comes back.

## Removal side effects

`pre_remove_side_effects` runs on the authoritative server while the block entity and its
attachments are still readable, before removal's neighbor effects. The level host permits
reads and random draws, not world writes. Flag 256 skips the call and its outputs.
Contract version 4 includes removal when a menu callback removes the backing block.
Do not drop those contents manually in the button callback.

The engine applies the answers after the level write, in this order:

1. Unless `keep_contents` is true, scatter the base container slots.
2. Scatter `drops`, a list of `Option<ItemStack>` in slot order. `None` consumes the
   three position draws of an empty `Containers.dropItemStack` slot but spawns nothing.
3. Create `spawns`, each `(ItemStack, [f64; 3])`, at that exact position with the default
   pickup delay (10 ticks), as a lectern or jukebox does.
4. Award `experience`, each `(u32, [f64; 3])`, through the furnace's experience-orb path.

Each list accepts at most 1024 entries; positions must be finite. Invalid list sizes or
positions refuse the outputs. Stack writes use the usual checked item boundary. The same
fields go to the Part. A part that answers nothing consumes no random draws.
The optional drops represent empty slots; wrap a filled slot in `Some`.

## Part state

A Part that keeps a value on each block entity of its type (a machine's progress, the last
position of a moving head) declares it with `BlockEntityPartState` instead of keeping a side
table. The game owns the values: one per block entity, in its attachment map, saved with the
chunk through the attachment codec and kept by transactions.

```rust,ignore
static PROGRESS: AttachmentType<u32> = AttachmentType {
    namespace: "mymod", name: "press_progress", default: || 0,
    codec: Some(PROGRESS_CODEC), copy_on_death: false, sync: false,
};
impl BlockEntityPartState for Press {
    type Data = u32;
    const DATA: &'static AttachmentType<u32> = &PROGRESS;
}
r.block_entity_part_with_state(&press, Press)?;

// In its tick (or a menu hook), get the live handle from the view, then edit through the Host:
let Some(entity) = event.subject.with_view(|view| view.handle()) else {
    return;
};
let done = Press::edit_state(event.subject.host(), entity, |_, progress| {
    *progress += 1;
    *progress >= 200
});
```

- `Press::state(host, entity)` and `state_existing` read a snapshot through `&dyn Host` and never
  insert the default. `edit_state` takes `&mut dyn ServerHost` and runs in a nested transaction
  of its own. The closure gets a copy and the host, and no store borrow is held while it runs. The
  result is stored when the closure returns.
- Every call checks the handle first. A handle from another level gives `PartStateError::OtherLevel`.
  A removed, replaced or unloaded block entity gives `Stale`. After a reload the saved value is read
  under the new handle.
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
  `Stale`. Removed block entities retire only when the outermost transaction commits. No save runs
  while a value may still be tentative, and a chunk unload waits until no transaction is open.
- No size or depth limit of its own applies to a value. A saved value follows the chunk's NBT rules:
  vanilla 26.3 reads region files with `NbtAccounter.unlimitedHeap`, an unbounded byte budget and
  depth 512. The 2 MiB quota of a chunk packet's block entity data is a network limit and does not
  cap saved state.
- Registration refuses a declaration that syncs, copies on death, lies outside the mod's namespaces
  or reuses a key with a type of another Rust type name. The refusal names the Part, the key,
  the type and the reason.
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
    .transform(|data: &Compound, transform: StructureTransform| {
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
        let mut result = data.clone();
        result.insert("facing".into(), Tag::String(directions[(index + turns) % 4].into()));
        Ok(result)
    })
    .build();
```

The step runs on the authoritative server during structure-template placement and worldgen,
possibly on worldgen worker threads. The block state has already been mirrored and rotated; the
step returns the saved data to store. Mirror applies before rotation. Its only inputs are the data,
rotation and mirror: it gets no `Host` or world access. Determinism is the mod's obligation, not
something the engine enforces; any matching `Send + Sync + 'static` closure is accepted. Avoid
randomness, clocks and mutable external state when calculating the result.

The engine restores the original `id` and any original `x`, `y`, `z` fields over the result, so a
payload transform cannot change the saved entity's identity or original coordinates. With no step,
data is unchanged. A `TransformRefusal` logs the type and position and keeps the original data;
placement succeeds. Invalid output exceeding standard NBT limits (64 MiB, one million nodes,
depth 512), or unwinding panics also keep the original data.

Callbacks are registered at startup and frozen with the block registry snapshot. A data-pack
`/reload` keeps that registry and its existing callbacks; it does not register replacement code.
A newly prepared registry receives its own callback snapshot.
