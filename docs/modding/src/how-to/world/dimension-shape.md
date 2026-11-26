# Dimension shape

## Dimension keys

`LevelRef::dimension` returns a `DimensionKey`, a small `Copy` value that names the dimension
for as long as the game runs, custom dimensions included. Compare it with another key or with
`DimensionKey::OVERWORLD`, `NETHER` and `END`; `level.dimension().id(host)` borrows the name
as `Option<&str>` (for example, `Some("minecraft:the_nether")`). It returns `None`
without a backend or for an unissued slot. Copy the name only if you need to retain it
beyond the host borrow. On a client, `host.level(name)` gives the level it shows under
the name the server calls it, whether or not that is a vanilla dimension.

## Dimension shape

`LevelRef::dimension_shape` reads the loaded server level's build range, logical height,
skylight and ceiling from its dimension shape. It accepts `&dyn Host`, including when the
level is parked, and returns `None` for a stale or unavailable level or on a client. It does
not load a level or allocate.

## Dimension type

`level.dimension_type(host)` reads a level's dimension type (`Level.dimensionType()` and its
gameplay attributes) as a `LevelDimensionType`: the build range, `coordinate_scale`, `ambient_light`,
`has_fixed_time`, `infiniburn`, the monster spawn light range, the sky and the gameplay switches. A
level on a vanilla type returns that type's values, a registered dimension returns what its
`DimensionSettings` set, and a data pack's dimension type counts the same way. It returns `None` for
a stale handle. On a client, it reads the displayed level's received dimension type,
including custom types sent by another server. Text fields and the timeline slice borrow
the type while scalar settings are values. Copy a string or use `timelines.to_vec()`
only if you need the data after the host borrow ends.

## Reading saved maps

`level.visit_map_data(host, id, &mut visit)` reads a world map through any current server
level, including a parked one. On `Ok(Some(()))` the callback ran once with a
`MapState<'_>`; its dimension ID and colour bytes are borrowed for that call. Read them
in place, or use `map.dimension.to_owned()` and `map.colors.to_vec()` if retaining them.
A missing map returns `Ok(None)`. A stale handle or unavailable side, phase or backend
returns `LevelReadError`. Neither a missing map nor a refusal calls the visitor.

## Registering a dimension

`register_dimension` gives every world a level of the mod's own. Call it once, at startup, in
`Hooks::register`:

```rust,ignore
#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_dimension(
            "pocket",
            DimensionSettings { min_y: -128, height: 448, logical_height: 448, ..Default::default() },
        );
    }
}
```

The level is `mymod:pocket`. `DimensionSettings` sets the build height, `has_skylight`,
`has_ceiling`, the `generator` (`minecraft:noise` by default), its `noise_settings` and the level's
one `biome`; the default is the Overworld's shape and terrain over plains. It also sets the rest of
the dimension type the game reads: `has_fixed_time`, `has_ender_dragon_fight`, `coordinate_scale`,
`ambient_light`, `infiniburn`, the monster spawn light range and block light limit, the `skybox`,
the `cardinal_light` and the gameplay switches (`water_evaporates`, `fast_lava`,
`respawn_anchor_works`, `nether_portal_spawns_piglin`, `piglins_zombify`, `snow_golem_melts`,
`can_start_raid`, `sky_light_level`) and the bed rule (`bed_can_sleep`, `bed_can_set_spawn`,
`bed_destroy_on_use`, `bed_destroy_on_leave`: the first two accept `always`, `when_dark` or `never`;
the last two are boolean flags controlling destruction on use or leave; the same four with `straw_bed_` for the straw bed). A field left at its default is the Overworld's, and the game
refuses a value out of range (a `coordinate_scale` outside 0.00001 to 30000000, an `ambient_light` that is not a number, an unknown
`skybox`, a bed rule that is none of the three) at startup with a message naming the mod. The
`coordinate_scale` is what `execute in mymod:pocket` scales a position by, and what a portal
between two levels scales by (`DimensionType.getTeleportationScale`); a position given to a
teleport call is taken as it is. The level opens, ticks, saves and closes as the
Nether and the End do, in `dimensions/mymod/pocket/` of the world, and clients receive its
dimension type when they join. A client of a server takes the whole type, not only the build
range: the level's fluids, mobs, beds and time follow the type the server sent, as on the host,
whether or not the client has the mod that registered the level.

`sky_color`, `fog_color` (`#rrggbb`) and `cloud_height` set the sky overhead, the fog at the
sunrise and the clouds' height; left as None they are the Overworld's. A client of a server draws a
level with the sky the server's type gives it, as the host does: a level of a mod or a pack, and a
built-in level whose type a data pack changed. Resource packs' Polytone dimension modifiers apply
over that type only when it is one of the three vanilla types (`minecraft:overworld`,
`minecraft:the_nether`, `minecraft:the_end`); a mod's dimension type is drawn without them, even in a
built-in level.

`day_cycle` picks the timelines and the clock the dimension type names: `overworld` (the day and moon
cycles, the villagers' schedule and the Overworld's clock, the default), `nether` (the schedule only,
no clock), `end` or `none`. It sets how the level's sky and light change over the day in the host's
world, whether monsters burn in daylight, and which clock `/time` reads there. `level.dimension_type(host)`
gives the timelines and `default_clock` back. Use `level.clock_time(host)` for the dimension's default clock and `level.day_time(host)` for the Overworld clock.

The level's biomes are the one `biome` everywhere unless `biome_source` says otherwise:
`BiomeSourceKind::Checkerboard` alternates the `biomes` in cells `2^biome_scale` chunks wide,
`MultiNoise` uses the Overworld's or the Nether's biomes (`biome_preset`) and `End` the End's islands.
`Points` places the mod's own biomes: `biome_points` lists a `BiomePoint` per biome, with an interval
(`(min, max)`, each from -2 to 2, a single value is `(v, v)`) for temperature, humidity, continentalness,
erosion, depth and weirdness, and an `offset` from 0 to 1. The biome whose point is nearest to the level's
climate at a position wins, as in a data pack's `minecraft:multi_noise` source with an inline `biomes` list;
the biomes can be the mod's own or vanilla's. A mod's points shape its own level only: placing a mod's biomes
among vanilla's in the Overworld or the Nether is a [biome region](../content-mods/biome-regions.md).
Any valid `biome_source` (`Fixed`, `Checkerboard`, `MultiNoise`, `End`, `Points`) or a registered
`mod_biome_source` ([biome sources](#biome-sources)) composes with either `minecraft:noise` or a
chunk generator of a mod's own.

A bare name uses the registering mod's namespace; an explicit valid `namespace:path` keeps its
namespace. An empty or malformed name, a name that is not a folder of the world
(lowercase letters, digits and `_ . -` only, with `/` between segments that are not empty, `.` or
`..`), a vanilla level's id, an id registered twice, a build height the dimension type codec refuses
(`min_y` and `height` are multiples of 16, and the level stays between -2032 and 2031) and a
generator nobody registered stop the game at startup with a message naming the mod. The generator
may come from a mod that loads later.

A registered dimension is the level stem and dimension type a data pack would give, below every
data pack: a pack with `data/mymod/dimension/pocket.json` or `data/mymod/dimension_type/pocket.json`
replaces that half of it, in the worlds that have the pack. A `minecraft:flat` stem uses the
resolved dimension's build range and the world pack's biome registry. Its `features`, `lakes` and
`structure_overrides` use the same generator as a flat Overworld. Invalid layer heights fail
world loading with the dimension id in the error.

## Sending a player to a dimension

There is no portal to a registered level. A mod sends an entity there with
`entity.teleport_to_level(host, dimension, pos, rotation)` from a server hook, the call vanilla has
as `Entity.teleport` with a portal's transition. It works for a player, a mob, a boat, a minecart
(a vehicle takes its passengers along), an item, a primed TNT, an experience orb, an arrow, a
trident, a firework and the thrown items and fireballs a chunk saves. The entity keeps its motion;
set it first with `entity.set_velocity` to stop or turn it.

Bind a [server command](../entities/server-commands.md) in `Hooks::register_commands`.
This executor accepts a player source and returns `Ok(1)` when the move is queued:

```rust,ignore
event.commands.push(Command::new("pocket").executes(|host, source, _| {
    let CommandSource::Player { player, .. } = source else {
        return Err(Text::translate("mymod.commands.player_required", vec![]).into());
    };
    player.entity().teleport_to_level(host, "mymod:pocket", [8.5, 100.0, 8.5], None)
        .then_some(1)
        .ok_or_else(|| Text::translate("mymod.commands.teleport_failed", vec![]).into())
}));
```

`pos` is in the destination's own coordinates: nothing scales it by the level's
`coordinate_scale`. A level nobody has entered opens first, and a player arrives after the current
tick, when `player_teleported` and `player_changed_dimension` report the move. The call
answers whether the move was taken, not whether the player arrived: a level that won't open, such
as one naming a generator nobody registered, logs an error and leaves the player where they were.
It is false for an unregistered dimension and for an entity that cannot change level (an item, an
orb, a projectile); a fake player stays where it is. Nothing builds a platform at the destination, so look for the
ground first, or place one.

## Sea level

`level.sea_level(host)` reads `Level.getSeaLevel`: 63 in the Overworld, 32 in the Nether, 0 in the End. For a level of
the noise generator it is the `sea_level` of its noise settings, a data pack's too when it replaces them. A level
of a chunk generator the mod wrote has no noise settings to read: set `DimensionSettings::sea_level` to give it one
(a level of the noise generator keeps its noise settings'), else it is 63 (-63 for a superflat generator). It is the sea
level the game uses for the level: mob spawning and behaviour (drowned, turtles, phantoms), rain and snow, and maps.
Terrain and aquifers are built with the generator's own sea level, which for a noise level is the noise settings' and
which `DimensionSettings::sea_level` never changes: it only gives `level.sea_level(host)` a value for a generator that has
none, so it does not move where the water is.

## Day clocks

Each level goes by its dimension type's `default_clock`: the Overworld by `minecraft:overworld`, the End by
`minecraft:the_end`, a level without one (the Nether) by the Overworld's. Every clock the world has met runs at its
own rate and keeps its own total, saved in `world_clocks.dat`, so a level with a clock of its own has its own day:
its sky, its spawning and its beds follow that clock, and `/time` in the level sets it.
`level.clock_time(host)` reads the level's own clock in ticks (`None` on a client, for a stale handle or for a level without a default clock);
`level.day_time(host)` reads the Overworld's clock, whichever level you ask.
`level.clock_marker_time(host, &marker)` gives the next time the level's own clock reaches a time marker, as
`/time set <marker>` moves it. Only the Overworld's clock defines markers today, so any other clock answers None.

To give a dimension a clock of its own, register a `WorldClock` with `r.register_world_clock("dungeon")` in
`register` and name it in the dimension's settings, `DimensionSettings { clock: Some("mymod:dungeon".into()), .. }`:
the dimension type takes it as its `default_clock` in place of `day_cycle`'s, and every level of that type follows it.
A registered clock is an entry of the world's `minecraft:world_clock` registry, sent to players, and runs from the
world's first tick like the vanilla clocks. `level.clock(host)` gives the level's clock as a `WorldClockRef`
(`None` for a level without one), whose writes are `ServerClockManager`'s: `set_total_ticks(host, ticks)` sets its
time, `set_rate(host, rate)` the ticks it counts each game tick, and `set_paused(host, paused)` stops or restarts it,
so a paused clock gives the level a fixed time.

When enough players have slept, `sleep_finished_time` hands its listeners a `ClockAdjustment` for the level's own
clock: `Absolute(time)`, `Relative(ticks)` from the time now, or `Marker(id)`. It starts as vanilla's
`Marker(minecraft:wake_up_from_sleep)`, a later mod reads what an earlier one set, and the game resolves it once
after all of them. A time before now or past `i64::MAX`, or a marker the clock does not define, is refused and the
clock moves to the morning instead; `new_time` is that morning, read only.

Before any of that, `sleep_skip_night` is asked once each tick that enough players have slept, with the
`players_sleeping_percentage` rule and how many players sleep and count. Canceling it keeps the night for that tick:
the clock does not move, nobody is woken and the rain goes on, and `sleep_finished_time` is not asked.

How many is enough is `sleepers_needed`'s: it arrives with vanilla's count (the rule's share of the players that
count, rounded up, at least 1) and a listener may set its own for the level and tick. The night-skip check and the
"n/m players sleeping" line both use the answer; below 1 counts as 1.

`sleep_status_announce` is asked when the game is about to show everyone the action-bar line "n/m players sleeping" or "Sleeping through this night". It sees the counts and the needed number, may replace `line`, and canceling it sends nobody the line; who sleeps and when the night skips do not change.

## Which level a hook is in

The block-behaviour hooks (`neighbor_changed`, `random_tick`, `crop_grow`, `block_changed` and the rest) name their
level with a `LevelDimension`: `Overworld`, `Nether`, `End`, or `Custom(key)` for any other level, a mod's or a data
pack's. `LevelDimension::key()` gives the level's key (`key.id()` its `namespace:path`) and `LevelDimension::of_level`
goes back. Match it with a final `Custom(_)` arm. The game's own rules read the level's own dimension type for these
too (water that evaporates, fast lava, infiniburn, piglin portals), and `player_respawn_position` may send a player to
a `Custom` level.

`crop_grow_post` version 2 always reports the growing plant's pre-growth state as
`original_state`, including sugar cane and kelp or vine heads. This diverges from
NeoForge. `state` is the current block at the reported position: the new cane or
head for those plants. Age-only sugar cane steps emit no Post; a full-age stem
still emits Post if no fruit was placed. Cactus and chorus report the plant's
position, not the position above it used by Pre.

## Players, structures and chunk watchers

`level.visit_online_players(host, &mut visit)` visits the real online players in that server
level without building a list, even if the level is parked. In the active level the local
or acting player comes first, then the placed guests in server order; a parked level has
only its placed guests. Fake players and players displaced by a nested guest turn are skipped.
The callback takes a `PlayerRef` and can make shared reads through the same host. Return
`core::ops::ControlFlow::Break(())` to stop or `Continue(())` to keep going. The call returns
`Ok(Break(()))` if you stopped it or `Ok(Continue(()))` if it finished, including when
the level had no players. A stale level gives `Err(LevelReadError::Stale)`; a client or
unavailable world access gives `Err(LevelReadError::Unavailable)`. Neither calls the visitor.

If you need mutable host access, collect only the handles that will receive the effect.
For example, select players above a world y-coordinate (`min_y: f64`) with a shared
read, then send each a message. This example is from a context that can propagate
both visit and send errors:

```rust,ignore
use core::ops::ControlFlow::Continue;

let mut recipients = Vec::new();
let _ = level.visit_online_players(host, &mut |player| {
    if player.entity().position(host).is_some_and(|pos| pos[1] > min_y) {
        recipients.push(player);
    }
    Continue(())
})?;
for player in recipients {
    let _ = player.tell(host, message.clone())?;
}
```

For typed payloads, `ToClient::send_in_level` and `ToClient::send_near` already handle
collecting recipients before sending.

Obtain a chunk with `level.chunk_ref(host, pos)`, then call
`chunk.visit_watchers(host, &mut visit)` to visit the players it was sent to and whose view
still contains it, in server order. Fake players watch nothing. A valid parked level uses
its own tracking rows; a valid unwatched chunk completes without calling the visitor.

`level.visit_loaded_chunks(host, &mut visit)` visits resident chunks in x-then-z order,
without loading anything. `level.visit_forced_chunks(host, &mut visit)` visits commanded
chunk positions in `/forceload query` order. `level.visit_mod_forced_chunks(host, controller,
&mut visit)` visits the issued controller's tickets in ticket order, passing the owner,
position and natural-spawning flag separately. These visits use the same `ControlFlow` and
`LevelReadError` convention as the online-player visit. The handle authorizes the read
regardless of the calling mod; an invalid handle or one from another registration table
returns `Unavailable`.

`level.find_nearest_structure(host, structure, StructureSearch::new(origin, radius))` searches
for a structure id or `#tag` and returns its structure and locate position. This is a
synchronous, `/locate`-cost search, not a cheap loaded-chunk lookup: cache results and avoid
per-tick use. The radius counts placement-cell rings (nonnegative; `/locate` uses 100);
stronghold rings are checked regardless of radius. It does not load or generate chunks.

## Chunk tickets

Register a named ticket controller once at startup in `Hooks::register` with
`r.register_ticket_controller("quarry", validator)`. Keep the returned `TicketController`
handle (for example in a `OnceLock`) and pass it to `chunk.set_mod_forced` and
`level.mod_forced_chunks`. Active reads and writes authorize by the registry-issued handle
itself rather than the current caller, so helpers or shared subsystems that hold the handle
can manage that controller's tickets directly while separate controllers stay isolated.

Pass `Some(Arc::new(...))` to bind a `LoadingValidationCallback` that prunes stale saved
tickets when a level opens, or `None` to restore saved tickets without validation:

```rust,ignore
use std::sync::{Arc, OnceLock};
use mod_api::{
    Hooks, LevelRef, Registrar, RegistrationContext, ServerHost, TicketController,
    TicketOwner, TicketValidation,
};

pub static QUARRY_TICKETS: OnceLock<TicketController> = OnceLock::new();

#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let quarry_block = r.block_key("mymod:quarry");
        let controller = r.register_ticket_controller(
            "quarry",
            Some(Arc::new(
                move |host: &mut dyn ServerHost,
                      level: LevelRef,
                      tickets: &mut dyn TicketValidation| {
                    let stale: Vec<[i32; 3]> = tickets
                        .block_tickets()
                        .keys()
                        .copied()
                        .filter(|&pos| level.block(host, pos) != Some(quarry_block))
                        .collect();
                    for pos in stale {
                        tickets.remove_block_tickets(pos);
                    }
                },
            )),
        );
        let _ = QUARRY_TICKETS.set(controller);
    }
}

pub fn hold_quarry_chunk(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) -> bool {
    let Some(&controller) = QUARRY_TICKETS.get() else {
        return false;
    };
    let Some(chunk) = level.chunk_ref(host, [pos[0] >> 4, pos[2] >> 4]) else {
        return false;
    };
    chunk.set_mod_forced(host, controller, TicketOwner::Block(pos), true, false)
}
```

`tickets.block_tickets()` and `tickets.entity_tickets()` borrow read-only `BTreeMap` views
of the controller's original saved tickets for the whole callback even after removals, while
`remove_block_tickets`, `remove_block_ticket`, `remove_entity_tickets` and
`remove_entity_ticket` stage removals that publish only when the validator completes successfully. Use `level.dimension().id(host)`
when the validator needs the opening level's dimension name. See
[Validating saved chunk tickets](runtime-levels.md#validating-saved-chunk-tickets) for
startup and runtime-level validation timing and panic handling.

## Watered-area tickets

On the server, `level.add_water_ticket(host, min, max)` returns an `Option<WaterTicket>`
for a sprinkler's watered area. Bounds are integer and half-open on **all three axes**:
`min` is included, `max` is excluded. For 5×1×5 farmland beginning at `[4, 64, 4]`,
use `[4, 64, 4]` and `[9, 65, 9]`; `[8, 64, 8]` is inside, `[9, 64, 4]` is outside.
Empty or reversed boxes are refused; any nonempty box is accepted without a column cap.

`WaterTicket` is an in-process handle and does not implement `Wire` — never serialize or
persist it. Retain the handle in memory, then call `ticket.remove(host)` when the sprinkler
stops or breaks. Removal returns whether that ticket was removed; overlapping tickets remain
independent. Any holder with the `WaterTicket` handle can remove it without consulting the
current caller. `level.has_water_ticket(host, pos)` tests whether any ticket covers a block.
Both calls reach active or parked server levels without loading chunks; unavailable/stale
handles and clients answer `false` (adding answers `None`).

Tickets are indexed by enclosing x/z chunk tile in memory only, never force chunks to load,
and queries test matching tiles without calling mod predicates. Each ticket attaches to the
covered chunk closest to its horizontal center (its master chunk, matching NeoForge's
`FarmlandWaterManager.addAABBTicket`): unloading that chunk on the server invalidates the
ticket (`FarmlandWaterManager.removeTickets`), while unloading other covered chunks leaves it
active. Tickets are also removed on explicit removal or level unload; when added during an
enabled mod call, the ticket is attributed to that mod and cleared if that mod is disabled
(calling from a disabled installed mod is refused, and re-enabling does not restore old
tickets). A reopened world starts empty: re-add from `level_load` or a block entity's load
listener. Several boxes can describe a shape without a per-position mod predicate.
Farmland checks the ticket before its vanilla water scan, so dry farmland inside moistens
to 7. Removing the last covering ticket restores normal drying, while nearby water and
rain continue working normally and the moisture-change veto still applies. Empty levels
pay only the empty-index guard, not a mod scan or callback.

## Saving a level

`level.request_save(host)` asks the server to save a level, as the autosave and `/save-all` do. The
save runs at the start of the next server tick, not inside the call; multiple requests before
then save it once. The call answers whether the request was taken, not whether the save succeeded.
`level_save` fires with the level's dimension after the save pass, even if an individual record
could not be written. As in vanilla, those failures are logged but do not fail the pass. A failed
awaited storage flush fails the pass and does not fire `level_save`.

Inside a hook, use `level.request_save(host)`, not a synchronous `save-all` command.
While an entity pass or held-out row is borrowed, synchronous saving refuses with vanilla's
`commands.save.failed` feedback and a failed command result, without writing a partial snapshot.
Requested saves wait for a settled start-of-tick boundary.

## Resolve an entity by UUID

In a server hook, wrap a saved `u128` in `EntityUuid` and ask the level that
should hold the entity with `LevelRef::entity_by_uuid`. The level may be the one
in place or a parked one; the lookup returns `None` if the entity is unloaded or
in another level. To search every level, do as vanilla's
`ServerLevel.getEntityInAnyDimension` does: ask the hook's own level first, then
each other level `host.levels()` lists.

```rust,ignore
use mod_api::prelude::*;
use mod_api::{EntityRef, EntityUuid, LevelRef, ServerHost};

fn find_saved_entity(host: &dyn ServerHost, level: LevelRef, saved_uuid: u128) -> Option<EntityRef> {
    level.entity_by_uuid(host, EntityUuid(saved_uuid))
}

fn find_in_any_level(host: &dyn ServerHost, own: LevelRef, saved_uuid: u128) -> Option<EntityRef> {
    let uuid = EntityUuid(saved_uuid);
    own.entity_by_uuid(host, uuid).or_else(|| {
        let levels = host.levels().list_levels();
        levels
            .into_iter()
            .filter(|level| *level != own)
            .find_map(|level| level.entity_by_uuid(host, uuid))
    })
}

fn entity_identity(host: &mut dyn ServerHost, entity: EntityRef) -> Option<EntityUuid> {
    entity.uuid(host).map(EntityUuid)
}
```

`EntityRef::uuid` returns the raw `u128`. Use `EntityUuid(raw_uuid)` to pass
that value back into the lookup.

## Chunk generators


A chunk generator is a mod's code for a level's terrain: vanilla's `ChunkGenerator`, and
`build_terrain` its `buildTerrain`. Register it once, at startup, and name it from a dimension,
or from a data pack's level stem as `"generator": {"type": "mymod:void"}`:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:terrain}}
```

There is no `Host` in world generation: chunks fill on the worker threads, so a generator sees only
the `ChunkAccess` of the chunk it fills, is `Send + Sync`, and keeps no mutable state, because two
workers can fill neighbouring chunks at once. The same level id, seed and chunk give the same blocks.
`ChunkAccess` has the chunk's `pos`, the id of its level (`level()`, such as `mymod:pocket`, so
one generator can serve several levels and tell them apart), the level's `seed` (the world's seed),
the level's `min_y` and `height`, and borrowed immutable `metadata()`. Resolve `BlockKey`s at registration, then use `state` or
`state_with` to construct API-owned `BlockState`s before the terrain-writing loop, never once per
placed block. `set_block` skips a position outside the chunk or build height and answers whether
it wrote. The chunk starts as air; invalid state ordinals are refused rather than written.

`set_block_entity_nbt(nbt)` is vanilla's `ChunkAccess.setBlockEntityNbt`: a block entity's data
from generation on, such as a chest's `LootTable` and `LootTableSeed`, at the block the tag's `x`,
`y` and `z` name. It waits as the chunk's pending data and becomes the block's block entity before
the chunk first loads, as a structure's chest does; data with no `id`, or for a block that has no
block entity by then, is dropped.

`create_biomes(chunk)` is vanilla's `ChunkGenerator.createBiomes`, run at the biome step before the
terrain: `chunk.fill_biomes_from_noise(&mut |[qx, qy, qz]| ...)` gives each 4×4×4 cell the
`BiomeKey` its quart position resolves to, as `ChunkAccess.fillBiomesFromNoise` does. The chunk
keeps those biomes; structure placement and carvers still ask the level stem's biome source, as in
vanilla. A generator that does not answer keeps the source's biomes.

By default the generator gives terrain only: the level has no surface, carvers, structures or
features, and its biomes come from the level stem's biome source. It does not need
`noise_settings`. As in vanilla's `ChunkMap` for non-noise generators, any climate or erosion
that the source samples is zero (`NoiseRouterData.none()`).

`vanilla_stages` asks for vanilla's stages around the terrain, as `VanillaStages` with the fields
`surface`, `carvers`, `structures` and `features`; it is asked once, at registration. They run in
vanilla's order: the structure starts before the terrain, then the level's material rule
(`surface`: grass, sand, deepslate, bedrock and ore veins over every solid block) and its biomes'
carvers, as vanilla's noise generator runs them after its noise fill, then the structure pieces
and the biomes' placed features. The material rule, the carvers' aquifer and the height a structure
starts at come from the level's `noise_settings`, not from the generator's terrain, and so does its
sea level. With any stage, `ChunkAccess`'s `min_y` and `height` are the noise settings' range and
`set_block` writes only inside it, as vanilla's noise fill does. Turning off structures for the
world skips structure generation without changing those terrain bounds. A level whose stem names no noise settings or ones the world lacks, one that asks for
structures in a world with no structure sets or for structures or features in one with no placed
features, and one that asks for features the game cannot place yet, refuses to open with a message
naming the generator and what is missing; the surface and the carvers alone need neither. A level that asks for none of them costs
what it did before; one that asks for any runs on vanilla's generator, with the mod's terrain in
place of the noise.

A bare name uses the registering mod's namespace; an explicit valid `namespace:path` keeps its
namespace. An empty or malformed name, a name that is not a folder of the world (as
for a dimension), an id registered twice and built-in chunk generators (`minecraft:noise`,
`minecraft:flat`, `minecraft:debug`) stop the game at startup with a
message naming the mod.

A level stem naming a generator nobody registered, say after its mod was removed, is an error for
whoever opens that level, not for the world: a teleport there fails and is logged, and a player
saved there joins a server in the Overworld. Single player refuses to open a world whose player is
saved in such a level.

### Biome sources

A biome source is a mod's code for where a level's biomes go: vanilla's `BiomeSource`, with
`possible_biomes` its `possibleBiomes` and `noise_biome` its `getNoiseBiome`. Register it once, at
startup, and name it from a dimension's `mod_biome_source`, or from a data pack's level stem as
`"biome_source": {"type": "mymod:chaos"}`:

```rust,ignore
{{#include ../../../examples/src/worldgen.rs:biome_source}}
```

`possible_biomes` lists the registered `BiomeKey`s the source places, vanilla's, a data pack's
or the mod's own; it is asked once, at registration. `noise_biome` answers for one quart cell
(a block position >> 2,
`[x, y, z]`) of a world with `seed`, with an index into that list; an index past the end wraps
around it. It runs on the worldgen worker threads for every 4x4x4 cell of every chunk generated, with
no `Host`, so it is `Send + Sync`, keeps no mutable state and does not allocate. The same seed and
cell give the same biome.

The source serves the game's noise generator, whose structures and decoration read its possible
biomes as they read a vanilla source's, and a chunk generator of a mod's own, whose chunks then
get its biomes cell by cell instead of one biome. The biomes are saved with the chunks, by name, as
any other.

A bare name uses the registering mod's namespace; an explicit valid `namespace:path` keeps its
namespace. An empty or malformed name, a name that is not a folder of the world (as
for a dimension), built-in biome sources (`minecraft:fixed`, `minecraft:multi_noise`,
`minecraft:checkerboard`, `minecraft:the_end`), an id registered twice and an empty `possible_biomes`
stop the game at startup with a message naming the mod, and so does a dimension whose
`mod_biome_source` nobody registered.
The source may come from a mod that loads later. A level whose world lacks one of the possible
biomes refuses to open, naming the source and the biome, as does a level stem naming a source that
is gone.
