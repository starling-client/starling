# Runtime levels

A mod opens a level of its own while the server runs and closes it again: an instanced dungeon, a
minigame arena, a pocket dimension per player. `host.runtime_levels()` has `open`, `unload`, `delete`
and `is_open`.

In `Hooks::register_commands`, bind an operator-only [server command](../entities/server-commands.md).
Return `Ok(1)` after `open` accepts the request, or a failure message if it refuses:

```rust,ignore
event.commands.push(Command::op("openarena", 2).executes(|host, _, _| {
    let level = RuntimeLevel::new("mymod:pocket", RuntimeGenerator::Mod("mymod:arena".into()), 4242);
    // On success the level opens after this tick's levels have ticked.
    host.runtime_levels().open("mymod:instance/ab12", level)
        .map_err(|_| Text::translate("mymod.commands.arena_open_failed", vec![]))?;
    Ok(1)
}));
```

## What a level is made of

- **The id** is `namespace:path` with a folder-safe namespace and path. The folder is
  `dimensions/<namespace>/<path>/` of the world, derived from the id and nothing else, so an id
  that is not a folder name (`mymod:../x`, an empty segment, a Windows device name) is refused
  with `InvalidId`. Requests are not restricted to the calling mod's namespace. `open` and
  `delete` refuse a vanilla, startup or data-pack level with `ReservedId`.
- **`dimension_type`** is a type known since startup: a vanilla type (`minecraft:overworld`) or the
  type of a level a mod registered with [`register_dimension`](dimension-shape.md#registering-a-dimension)
  (`mymod:pocket`). Nothing new crosses the network, so every client already has it. Any other
  name is `UnknownDimensionType`; a data pack's own dimension types are not offered yet.
- **`generator`** is `RuntimeGenerator::Flat` (bedrock, two dirt, a grass block, over plains),
  `Void` (air), or `Mod(name)`, a chunk generator a native mod registered
  ([`register_chunk_generator`](dimension-shape.md#chunk-generators)). Use the generator's full
  `namespace:path` id. A malformed, unqualified or unregistered name returns `UnknownGenerator`.
- **`biome_source`** (`RuntimeLevel::with_biome_source`) pairs a [`RuntimeBiomeSource`] with
  `generator`, matching how vanilla's `LevelStem` pairs a `ChunkGenerator` and `BiomeSource`:
  `Fixed`, `Checkerboard`, `MultiNoise`, `End`, `Points`, or `Registered(id)` (a biome source
  registered with `register_biome_source`). `Flat` and `Void` follow vanilla's `FlatLevelSource`
  and accept only `Fixed` (or `None` for `minecraft:plains` / `minecraft:the_void`), refusing
  non-fixed sources with `IncompatibleBiomeSource`. `Mod(name)` accepts any valid biome source.
  `None` takes the dimension type's registered stem source, or plains if there is no registered
  stem. A custom generator without vanilla stages uses the non-noise sampling context, not
  the Overworld's climate router. A checkerboard takes either a single `#namespace:tag` reference
  or a list of biome ids, matching vanilla `HolderSetCodec`; mixing `#tag` references with other
  entries is refused as `InvalidBiomeSource`. A missing biome, tag, preset or registered source
  returns `UnknownBiomeSource`; an empty checkerboard holder set, mixed list, invalid identifier
  or invalid scale returns `InvalidBiomeSource`.
- **`seed`** is the level's own. It reaches the mod generator (`ChunkAccess::seed`), the level's
  block-tick and level randoms and the hashed seed clients are sent. `ChunkAccess::level` tells a
  generator which level it fills, so one generator serves every instance and looks its plan up by
  id.

## When it happens

`open`, `unload` and `delete` check the request when they are called and return the refusal at once. The
change itself waits: the server applies the queue after its levels have ticked in the server
tick, never inside a level's tick, so no hook sees a level appear or vanish under it. Until then
`is_open` is false after an `open` and true after an `unload` or a `delete`. A request made while the queue is
applied waits for the next tick. `level_load` and `level_unload` fire as for any level the game
opens or closes, once each.

`open` returns a `LevelRef` that names this open of the level. A handle from before an `unload`
never names a later open of the same id: it reads as stale, like a handle from an earlier world.
A folder that exists under the id is reopened as it was saved, a persistent level after an
unload or a restart. Nothing reopens a runtime level by itself at the next start: the mod
reopens what it keeps, and a player saved in a level that was not reopened keeps the usual
fallback. At server startup every configured level (including datapack dimensions) fires
`level_load` (v4) before the next is available. The Overworld opens first and, if needed, selects the
initial world spawn before the other levels open, waiting for the spawn search's chunks to reach
FULL. The owner's player spawn is prepared separately. `create_spawn_position` runs during the
world spawn search for normal fresh worlds; the Debug preset uses its fixed spawn without that hook. The remaining levels follow
the `LEVEL_STEM` registry traversal; mods must not assume a particular order among them.
`host.levels().visit_levels(...)`, ticks of eligible levels, saves and shutdown use that creation order.
A player's saved dimension does not add to the registry. The server then validates and activates
saved tickets and waits for the ticketed chunks before `server_started` (v2), even if no player visits
those levels. In `Hooks::validate_tickets`, a controller's owning mod can remove saved tickets
through `event.validation`. The callback sees its removals immediately. As with
[staged registration](../../concepts/load-phases.md#registration-and-setup), a panic or phase
refusal discards those edits before the saved tickets take hold. Other controllers still validate.

On a full server save, `player_save_to_file` notifications for successfully saved placed players
come before `level_save`, then `world_saved` follows the world metadata write. During shutdown,
a failed player file write emits no `player_save_to_file` for that player; it does not prevent the
world metadata save. Startup's integrated
save completes before `server_starting` and `server_started`. It writes levels and world metadata,
naming a known profile for a new world without creating or overwriting the owner's player file.
The initial save is outside the `server_started` listener's transaction journal.

Lifecycle hooks follow the [shared failure rules](../../concepts/hooks-and-the-host.md#failures-and-panics).
A panicking listener does not fail a successful save or skip another listener's level close
notice. On the server, the existing transaction journal discards that callback's journaled
writes, including edits committed by a nested `host.transaction`. Effects outside the journal,
such as a mod writing its own file or editing client state, are not undone.

On the client, resolve the level in `client_level_load` with `host.level(event.dimension)` before it
is installed. Its chunks arrive later, but it can already read the datapack registries synchronized
by its server for that configuration. On dimension replacement, `client_level_unload` follows
for the retiring level: its handle, loaded chunks and available synchronized registries still
resolve until that callback finishes. On reconfiguration, Unload reads the retiring configuration
and the next Load reads the new one, even if the new registry set arrived before Unload ran.
The new level's handle remains valid when that level is installed.

Success means queue admission, not apply completion. Apply failures are logged with the
requesting mod and level id. `Unavailable` means there is no open logical server world or its
worldgen registries could not be loaded for a registered generator; it does not require a mod
callback to be active. A failed open rolls back publication and frees the id for retry;
a failed unload leaves the level open for another unload request. Observe `is_open` after
the queue runs rather than treating the returned result as completion.

An explicitly opened runtime level fires `level_load` while parked: `event.dimension` names
the opened level, whereas the Host still names the current level. Use
`host.levels().visit_levels(&mut visit)` to walk resident levels, including parked
runtime levels; a level that is open but idle-unloaded is not resident.

## Validating saved chunk tickets

In `Hooks::register`, pass `Some(Arc::new(|host, level, tickets| { /* prune stale tickets */ }))`
to `registrar.register_ticket_controller("quarry", ...)` to validate that controller's saved
chunk tickets. Pass `None` to restore them as saved. Retain the returned `TicketController` to
hold chunks independently of validation.

A validator runs only when its controller has saved tickets in the opening level. `level` is
the actual opened `LevelRef`, including when the level is parked; `level.dimension().id(host)`
returns its id. `tickets` shows only that controller's saved block and entity tickets, not live
tickets added in `level_load`. `tickets.block_tickets()` and `tickets.entity_tickets()` borrow
read-only `BTreeMap` views of the original saved tickets for the whole call and stay unchanged
after removals. Later removal calls skip rows the validator has already selected. The server
activates surviving tickets after all validators run.
If a validator panics, the server reports the mod, discards that validator's removals, and skips
only that validator for the rest of the session. Other controllers still validate their tickets.

## Closing

`unload(id)` saves the level and drops it, keeping the folder. It refuses with `PlayersInside`
while a player, or an arrival that has not landed, is in the level, both when it is called and
when it is applied: move the players out first with `entity.teleport_to_level`. `NotOpen` is the
answer for a level that is not open (or still waiting to open). A parked non-Overworld level that
has no players, views, tickets, or other work keeping it active can evict its chunks and entities
without closing: `is_open` stays true, neither `level_unload` nor `level_save` fires, and revisiting
it does not fire `level_load` again. An unload listener that activates another level keeps it
resident before its turn. The sweep leaves a column and its live entities resident if a required
record cannot be saved; other eligible columns can still evict. Only `unload`, `delete`, or server
shutdown closes the level and fires `level_unload`.

## Deleting

`delete(id)` removes a level for good: its folder `dimensions/<namespace>/<path>/` and the
structure references the game keeps beside the world for it
(`data/sunrise_structure_references_<namespace>_<path>.json`, kept if another id would share that
file and its level exists: `a_b:c` and `a:b_c` do), then the folders those leave empty. It refuses
invalid or reserved ids (`InvalidId`, `ReservedId`), and the folder it removes comes from the
validated id and nothing else.

- A level that is **open** is unloaded first, as by `unload` (saved, `level_unload` fires), and
  its files are removed in the same pass. It refuses with `PlayersInside` while a player or an
  arrival that has not landed is in the level, when called and again when applied: move the
  players out first. `is_open` stays true until the delete is applied, which closes it.
- A level that is **not open** is only files, the leftovers of a crash or of an earlier
  `unload`. They are removed, and a level with no files is not an error. Nothing fires, since
  nothing was open.
- A level still **waiting to open** refuses with `AlreadyOpen`: its `open` comes first.
- A level with a **level below it** refuses with `LevelsBelow`, when called and again when
  applied. Folders nest: `mymod:instance/ab` lives inside the folder of `mymod:instance`, so
  removing the parent would remove the child, which may be open, with players or a save waiting.
  A level is below when its id continues the id with `/` and it is open, waiting, a level of the
  world, or has a folder under the level's own. A folder under it is any folder other than
  `region`, `entities`, `poi` and `data`, one of the first three holding a folder, or a folder in
  `data` with a level's shape somewhere below it (a folder holding `region`, `entities` or `poi`,
  or a link); the other files a mod keeps in `data` are not in the way. A folder that cannot be
  read counts as a level below. Delete the level below first.

The removal happens on the server thread, in the pass that applies the queue, after the level
has been saved and dropped, so no save is running against the files. A request in the queue is
applied in the order it was made. A reopen therefore waits for the removal: an `open` of a level
that is open and being deleted is `AlreadyOpen` until the delete is applied; an `open` of a level
that was not open, made after the `delete`, is applied after it and starts from nothing. A save a
mod asked for with `level_request_save` is dropped with the level.

A link (symbolic link, junction) on the way to the folder under `dimensions`, or to the
references file under `data`, is never followed: the removal fails before anything is removed.

If the removal fails (a link, a file another program holds), the failure is logged with the mod
and the id, and what happened to the level depends on how far the delete got. A level that could
not be closed, or has a level below it, stays open and `is_open` stays true; delete it again.
A level that closed but whose files could not all be removed stays **closed**: `is_open` is
false, its handle is stale, `open` of the id answers `Removing`, and an `open` already waiting
behind the delete is dropped with a log line instead of starting from the files that are left.
A second `delete` of the id finishes the removal, and only then can it be opened.

A handle from before the delete reads as stale, as after an `unload`.

## Players and clients

A player goes into a runtime level and out of it as into any level, with
`entity.teleport_to_level`. Its client, vanilla or Sunrise, learns the level on the way in: the
Respawn names it and its dimension type, which the client has had since it joined. On the way out
a Sunrise client forgets a level the server opened after it logged in, so a long session through
many instances keeps none of them. A level opened again under the same id after a `delete`
reaches the client afresh, with nothing of the old one.

A runtime level opened after a player logged in is viewed through [chunk areas](../entities/chunk-areas.md)
like any other level, by a Sunrise client of this version. A client of an older version names the
levels it views by their place in the list it got at login, where such a level is not: the areas
there are kept, but nothing of the level is sent to it, and it sees the level only by going in.
When a runtime level closes, by `unload` or `delete`, every client that viewed it drops its world
of it; the areas stay, and show the level again once it is open again.

Every refusal is also logged with the name of the mod that asked.
