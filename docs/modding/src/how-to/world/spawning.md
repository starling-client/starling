# Controlling spawns

Use spawn hooks to accept or reject a spawn, edit a mob before finalization, edit pending breeding or split children, or control despawning. The four spawn-check and despawn hooks below run on the server with a read-only host and fire on replayed ticks, so their listeners must be pure functions of their inputs (`finalize_spawn` changes only `event.mob` and `event.group`). `baby_entity_spawn` and `mob_split` run on the server with `&mut dyn ServerHost`.

| Moment | Hook | Answer |
|---|---|---|
| The spawn predicate for a type and position | `mob_spawn_placement_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| The mob's own position and obstruction checks | `mob_spawn_position_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| Before `finalizeSpawn`, or after loading a spawner's configured mob | `finalize_spawn` | `Flow::Cancel`, `spawn_cancelled`, a new `difficulty`, edits to `mob`, a new `group` |
| Each tick, before a mob's `checkDespawn` | `mob_despawn` | `Despawn::{Default, Allow, Deny}` |
| After breeding constructs a baby, before insertion | `baby_entity_spawn` | `Flow::Cancel`, edits/replacement/removal on `event.child` |
| After a cube splits on death, before insertion | `mob_split` | `Flow::Cancel`, edits/additions/removals on `event.children` |

## Registering a spawn predicate

Use `Registrar::register_spawn_placement` in `Hooks::register` (NeoForge's
`RegisterSpawnPlacementsEvent.register`). Request the entity's fully qualified registry key
with `r.entity_type_key`. Entries are keyed by registered entity type, including the Ender Dragon
and non-mob types such as arrows. Registering a predicate does not create a new spawn path.
This example registers a ghoul that spawns on blocks in a ground tag, in biomes in a biome tag,
and in the dark (except at a trial spawner):

```rust,ignore
use mod_api::{
    BlockPos, EntityTypeKey, Heightmap, SpawnPlacementContext, SpawnPlacementOperation,
    SpawnPlacementType, SpawnReason,
};
use std::sync::Arc;

fn predicate(
    _: EntityTypeKey,
    ctx: &mut dyn SpawnPlacementContext,
    reason: SpawnReason,
    pos: BlockPos,
) -> bool {
    ctx.level().block_in_tag(pos.offset(0, -1, 0), "mymod:ghoul_ground")
        && ctx.level().biome_in_tag(pos, "mymod:haunted")
        && (reason == SpawnReason::TrialSpawner || ctx.is_dark_enough_to_spawn(pos))
}

// In Hooks::register:
let key = r.entity_type_key("mymod:ghoul");
let placement = SpawnPlacementType::OnGround;
let heightmap = Heightmap::MotionBlockingNoLeaves;
let _ = r.register_spawn_placement(
    key,
    Some(placement),
    Some(heightmap),
    Arc::new(predicate),
    SpawnPlacementOperation::Replace,
);
```

If the type has no vanilla spawn placement, its first registration must supply both metadata
fields: the placement checks where the type can stand, and the heightmap selects the surface
for chunk-generation spawns. The natural spawn pass starts from `WorldSurface` regardless of the
type. The first predicate becomes the original, whatever operation you give it. For an existing
entry, only `Replace` can set either field; `None` keeps
that field's current value, including any change by an earlier replacement.

If an existing entry was replaced, only the last replacement runs, even after later `Or` or
`And` registrations. Otherwise the original (vanilla or the first predicate for a new entry)
runs first. If it fails, `Or` predicates run in registration order until one passes. If neither
the original nor any `Or` passes, the result is false and no `And` runs. Otherwise `And`
predicates run in registration order until one fails; the result is true only if all pass.
Use `None` for both metadata fields when adding `Or` or `And` to an existing entry.

`SpawnPlacementContext` lends a read-only level and the caller's live `RandomSource` for one call.
Level reads never load chunks or write to the world. Use namespaced block and biome tags without
`#`. The light helpers don't check the reason: for vanilla's trial-spawner bypass, test
`reason == SpawnReason::TrialSpawner` first, as above. For a brightness condition, use
`ctx.level().is_bright_enough_to_spawn(pos)` in place of the darkness test. To allow any light
level, omit the light test. Draw any additional randomness from `ctx.random()`. The darkness
helper also draws from that stream. Short-circuited predicates don't draw. Returning `false`
or refusing the spawn later does not rewind any draws. Each predicate runs as the mod that
registered it. If a predicate panics, the game rolls back only that call's draws, disables that
predicate for the session, and names its mod in the log and client error toast. A disabled or
refused replacement supplies `false` to the placement check hook without rerunning the original.

Registered predicates run only where vanilla uses `SpawnPlacements.checkSpawnRules`;
patrols and village sieges call their built-in predicates directly instead. Spawners and trial
spawners check the resolved type, including non-mobs, before attempting to create an entity.
Ordinary spawners with custom spawn rules skip the placement check; trial spawners check
their custom rules afterward.

An unavailable key both returns `SpawnPlacementError::InvalidEntityType` and records a startup
refusal, so ignoring the returned error does not hide it. A type missing at resource load or
invalid placement/heightmap options refuse resource loading with the mod and type named. Sunrise
resolves entity keys from the accepted gameplay resources, not a provisional pack load.
Recovery from a failed client pack load clears the optional pack selection without removing
registered mod entity types or changing their spawn facts. Each prepared type must have a
category and dimensions in the accepted gameplay resources. Missing or malformed facts fail
loading and name the affected type instead of treating it as unknown when a spawner ticks.
If mods register predicates, the prepared resources must include vanilla's spawn placement data.

## Saved display and utility entities

`entity_join_level` fires for block, item and text displays, markers, interactions,
and summoned or chunk-restored eyes of ender and explicit leash knots.
It does not fire for gameplay-thrown eyes, naturally tied leash knots, or saved data
kept for a missing mod. A new entity fires the hook once with `loaded_from_disk = false`;
restoring it from a saved chunk fires the hook once with `loaded_from_disk = true`.
The normal join cancellation and transaction rules apply.

`entity_leave_level` reports when the same entities leave. On the server,
`LevelRef::visit_entities_in` also visits markers and interactions, and on both sides
block, item and text displays (see below). It does not visit eyes of ender, leash knots
or other unmodeled kinds. The query uses physical boxes, not rendering bounds: markers
have zero-sized boxes at their positions; interactions use their saved width and height.
A handle expires when its entity unloads, so find it again when the saved entity rejoins.

### Reading display settings

Narrow a server entity with `host.display(entity)`. The resulting `DisplayRef` reads
its current server settings, not the last save or the client's interpolated pose.
`text(host)` projects just the text component of a text display; it returns `Ok(None)`
for other display kinds. For an item display, `with_item` lends its stack only when
nonempty. For example, read the count without copying the stack's components:

```rust,ignore
let count = host.display(entity)?.with_item(host, |stack| stack.count());
```

Use `display.with_item(host, |stack| stack.project()).flatten()` if you need an owned
stack or need to inspect its components. `set_item(host, None)` still empties the display.

`set_transformation` stores the translation, scale and two `[x, y, z, w]` quaternions
exactly as provided. It does not normalize rotations or use their length to reject a
write. Provide unit quaternions for ordinary rotations. Vanilla's saved transformation
codec is separate and normalizes quaternion component lists on load.

## Visiting entities in a box

`level.visit_entities_in(host, min, max, &mut visit)` visits readable `EntityRef`s
whose boxes intersect the given box. Face-only contact does not intersect, and
inverted bounds are swapped on each axis. Ordinary entities are visited in level
section order, then in entry order within each section. The client sees tracked
entities and its own player if not spectating; the server includes fishing hooks
and placed non-spectator guests. Intersecting dragon parts come last on both sides.

Block, item and text displays have zero-sized physical boxes on both sides: a display
only matches when its position is strictly inside the query on every axis. On the client,
this is its current tick position after movement interpolation, not the packet's destination
or the pose drawn between ticks. Overlap with just the render culling box does not count,
and ordinary crosshair picking still ignores displays.

The visitor can read the entity or run another shared query through the same host:
no residency guard is held during the callback. Return `ControlFlow::Continue(())`
to keep walking or `ControlFlow::Break(())` to stop before checking later matches.
For example, find the first entity of a type without building a list:

```rust,ignore
use core::ops::ControlFlow::{Break, Continue};

let mut first = None;
let _ = level.visit_entities_in(host, min, max, &mut |entity| {
    if entity.kind(host) == Some(wanted_kind) {
        first = Some(entity);
        Break(())
    } else {
        Continue(())
    }
})?;
```

The call returns `Ok(Continue(()))` on completion, including a valid empty query,
or `Ok(Break(()))` if the visitor stopped it. `Err(LevelReadError::Stale)` means
the level is stale; `Err(LevelReadError::Unavailable)` means the bounds are not
finite, the level is parked or undisplayed, or world access is unavailable.
Neither refusal runs the visitor. If you need to mutate multiple entities, push
just the handles you need into your own list and use the mutable host after the
visit returns.

## Finding the nearest entity hit

Use `level.ray_trace_nearest_entity(host, from, to, margin, exclude, &mut accept)`
when you need just one hit. It visits the same kinds of entities as the box query
within the segment's bounds padded by `margin.abs()`. It skips `exclude` (such as
the shooter) before calling `accept`.

Matching vanilla's `ProjectileUtil.getEntityHitResult`, `accept` receives each
non-excluded candidate `EntityRef` in spatial traversal order (not nearest-first
order) **before** its bounding box is read and clipped, and can make shared reads
through the same host. Accepted candidates have their box inflated by `margin`
(`AABB.inflate`: positive grows, negative shrinks; any finite value is accepted)
and clipped against the segment, skipping boxes the ray starts inside. Among
accepted hits, the smallest squared distance from `from` wins; exact
squared-distance ties keep the first visited hit. The query doesn't allocate a hit
list. It returns `Ok(Some(hit))` on success and `Ok(None)` if no accepted
candidate is hit. A stale level returns `Err(LevelReadError::Stale)`; non-finite
ray geometry or unavailable world access returns `Err(LevelReadError::Unavailable)`.
Neither refusal runs the predicate.

Keep using `level.ray_trace_entities(host, from, to, margin, exclude)` when you need
all hits, depend on processing them nearest first, or must filter on the clipped
hit location or distance. That call returns `Some` with an empty list on a valid
miss and `None` if refused.

## Walking every entity in a level

`level.visit_all_entities(host, &mut visit)` visits the authoritative level's
`EntityRef`s without an AABB filter, including spectator players. Vehicles being
removed are skipped. Unlike `level.visit_entities_in`, it does not append the dragon's
parts: vanilla's `ServerLevel.getAllEntities` iterates the entity lookup, not
multipart hit boxes. The visitor can read the entity through the same host.

Return `ControlFlow::Continue(())` to visit the next entity or
`ControlFlow::Break(())` to stop. The walk resolves each eligible lookup member
lazily in admission order, skipping any member that cannot resolve cleanly, and
stops immediately on `ControlFlow::Break(())` without touching later members. The
call returns `Ok(ControlFlow::Continue(()))` on completion (including an empty
level), or `Ok(ControlFlow::Break(()))` if the visitor stopped it. A stale handle
returns `LevelReadError::Stale`. A remote client, a parked level, or unavailable
world access returns `LevelReadError::Unavailable` before the visitor runs.

The walk preserves lookup admission order across storage families, including
removal and re-admission at the end. An entity spawned during play, such as a
fired arrow or primed TNT, joins the lookup as it spawns, as vanilla's
`addFreshEntity` adds it, even when the engine gives it its id later. A chunk's
entities join in the order the chunk was stored: vanilla's `storeChunkSections`
order, section by section from section 0 upward and then the negative sections,
whatever storage family holds each entity. Each section keeps its entities in
the order they entered it: an entity that crosses into another section goes to
the end of that section, as vanilla's `onMove` puts it there, while its place in
the lookup stays the same. Each crossing is recorded as the entity moves, in
the order the moves happen: an entity that steps out of a section and back is
at the end of it again, and a passenger enters after its vehicle. A vehicle
comes before its passengers.

Section entry order does not depend on a listener of `entity_entering_section`:
the engine records each server crossing whether or not a mod hears it. When a mob
write is told with the owning Host, crossings queued by those listeners (for example,
from a vehicle teleport) are also told with the owning Host, in write order, before
the next mob ticks.

Some differences remain. Vanilla notes a section change at every position write;
here minecarts, boats, projectiles, dropped items, primed TNT, falling blocks,
fireworks, experience orbs and the other server entities that report after their
own update count several moves within one tick as one crossing, from where the
entity was to where it ended (mobs and players are told per write). A block
piston's push is recorded only while a mod listens to `entity_entering_section`.
Entities that the engine keeps outside the residency families, such as lingering
potion clouds, lightning bolts and eyes of ender, are not lookup members at all.
The walk uses the existing admission order without allocating a result list. If
you need a list after the call, collect the handles you need in your visitor.

## Checking a spawn

`mob_spawn_placement_check` and `mob_spawn_position_check` both get the `level` the mob would join (a `LevelRef`, so a
listener can read the blocks around `pos` through the host). Set `*event.result` to `SpawnCheck::Succeed` or `Fail` to
override that check; `Default` keeps the check's own answer.

In `mob_spawn_placement_check`, `event.default_result` is the effective predicate result. It can differ
from vanilla even for a vanilla type, because registrations can add to or replace its predicate. The hook fires
after the predicates at `SpawnPlacements.checkSpawnRules`. Like NeoForge's `SpawnPlacementCheck`, it has no spawner
field; inspect `event.reason` instead. For a probabilistic decision, draw from `event.random`,
the caller's live `&mut dyn RandomSource` lent to each listener in turn. Reassigning `event.random` only changes that
listener's local reference; draws from the caller's stream and writes through `event.result` carry to the next
listener. Keeping `Default` or refusing a spawn does not undo any draws, and this hook does not decide later checks.
Ordinary spawners with custom spawn rules bypass this placement check; trial spawners check their custom rules
afterward. Patrols and village sieges bypass it too, and commands and spawn eggs still don't run it.

```rust,ignore
fn mob_spawn_placement_check(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>) {
    // Veto half of the natural zombie placements that the predicate accepted.
    if event.kind == "minecraft:zombie"
        && event.reason == mod_api::SpawnReason::Natural
        && event.default_result
        && event.random.next_boolean()
    {
        *event.result = mod_api::SpawnCheck::Fail;
    }
}
```

In `mob_spawn_position_check` (hook version 3, NeoForge's `MobSpawnEvent.PositionCheck`), `event.spawner` is
`Some(Spawner::Block { pos })` for a monster spawner block, `Some(Spawner::Minecart { entity })` for a spawner
minecart, and `None` for natural spawning and chunk generation. A minecart's `entity.kind(host)` and
`entity.position(host)` read the live cart at this spawn attempt. Like NeoForge, trial spawners do not fire
`mob_spawn_position_check`. The position check also gets `event.mob`, the mob about to spawn, as a scoped read-only
`UnspawnedMobView<'_>` with the same reads as `finalize_spawn`'s (read-only: the hook does not edit the mob). Vanilla
creates the mob before this check. The spawn goes on with the very mob the listener read, not a second construction. A refused check
drops the mob: it never joins the level. Creating it draws nothing from the level's random, so a listener that answers
`Default` changes no draw.

```rust,ignore
fn mob_spawn_position_check(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPositionCheckEvent<'_>) {
    if event.mob.custom_name().is_ok_and(|name| name.is_some()) {
        *event.result = mod_api::SpawnCheck::Succeed;
    }
}
```

Both configured ordinary spawners and chunk-generation packs lend the real candidate. The ordinary spawner
loads its configured entity data before the position check. Trial spawners post placement checks, but run
obstruction checks directly without posting a position check.

To keep a monster spawner running with no player near, or idle with one beside it, answer [`spawner_active`](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/spawner_active.rs).

## Stopping a spawn or leaving a mob plain

`finalize_spawn` has two cancels. Returning `Flow::Cancel` skips finalization: the mob spawns, but with no equipment, attributes or class setup. Setting `event.spawn_cancelled = true` vetoes admission but still runs initialization and its draws. Set both to skip initialization and veto admission. `event.spawner` carries the shared
`Option<Spawner>` identity (`Spawner::Block { pos }`, `Spawner::Trial { pos }`, or `Spawner::Minecart { entity }`,
matching NeoForge's `FinalizeSpawnEvent.getSpawner`). The minecart is readable through the same host as in the position check.

```rust,ignore
fn finalize_spawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.kind == "minecraft:zombie" && event.reason == mod_api::SpawnReason::Natural {
        event.spawn_cancelled = true;
    }
    mod_api::Flow::Continue
}
```

`finalize_spawn` also fires when a monster spawner block, spawner minecart, or trial spawner uses
configured mob NBT. Those already skip vanilla initialization; a listener can still read, edit or veto the
loaded mob. On spawners, vanilla finalization (when unconfigured and not cancelled with `Flow::Cancel`),
trial-spawner persistence, and any configured equipment table roll still run before `spawn_cancelled` vetoes
admission. Chunk-generation packs lend their actual mob and carry spawn group data between members. Outside
spawners, the hook fires only where vanilla calls `finalizeSpawn`: breeding, an infested block's silverfish
and a thrown egg's chick do not fire it.

For `/summon minecraft:ender_dragon` without NBT, command placement starts at zero
rotation and consumes no random yaw draw. Skipping finalization also skips its
follow-range and handedness draws. Spawn eggs retain their random placement yaw;
an explicit NBT compound follows the load route without finalization.

## Changing the mob before it is finalized

In `finalize_spawn` (hook version 3), `event.mob` is a scoped `UnspawnedMobEditor<'_>` borrowing the mob directly: it is
not in a level, so it is not an `EntityRef`, and the borrow lasts only for this hook call. Read it directly without
`host` (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`, or `as_view` for a
read-only `UnspawnedMobView<'_>`) and edit it directly: equipment (an API-owned `ItemStack`), an attribute's base or a
permanent modifier, tags and the custom name. An edit applies at once, so later reads and the next listener see it.

```rust,ignore
fn finalize_spawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.mob.kind().is_ok_and(|kind| kind.path() == "zombie") {
        let health = mod_api::Identifier::parse("minecraft:max_health").unwrap();
        let _ = event.mob.set_attribute_base(&health, 40.0);
        let _ = event.mob.add_tag("mymod:tough");
    }
    mod_api::Flow::Continue
}
```

Listeners run before finalization. Finalization's own rolls follow them: equipment it rolls for a
slot replaces what a listener put there, and the attribute bonuses it adds apply on top of a listener's base. Return
`Flow::Cancel` to keep exactly what the listener left, since no finalization runs. Attribute edits reach the
attributes a mob keeps itself (max health, movement speed, follow range, attack damage, knockback resistance,
spawn reinforcements, jump strength, burning time, flying speed); the others answer `Unsupported` until the mob has
joined, when a `LivingRef` writes them. The ender dragon's loan answers its kind and position, and `Unsupported`
for the rest. A mob that finalization creates (a chicken jockey's chicken, a zombie horse's rider) fires the hook
again with a scoped loan of its own.

The hook fires on replayed ticks with the read-only host, so `event.mob` and `group` are the only things a
listener changes, and it must change them the same way every time: a pure function of the event.

`event.group` is the pack's `SpawnGroupData` as plain data (`SpawnGroup`, variants named as vanilla saves them):
read it, change it, set it or clear it, and finalization reads what is left. Each hook call receives `None`; group data is not passed between natural pack members.
Chunk generation shares group data inside the generator, outside this hook. A group you set affects
only this mob's finalization.

See [the `finalize_spawn` contract](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/finalize_spawn.rs) for the spawn sites that fire the hook.

## Changing how large a spawn group grows

A natural spawn adds mobs of one type to a group until its count reaches the type's limit
(`Mob.getMaxSpawnClusterSize`). `EntityPart::spawn_cluster_size` runs at that comparison in
`NaturalSpawner.spawnCategoryForPosition`, like NeoForge's `SpawnClusterSizeEvent`: after the mob was
finalized, added to the level with its passengers and counted by the spawn state. The part gets the read-only host and the
event: the mob (`event.mob`), the vanilla limit (`event.vanilla`, an `i32`) and one signed
`event.size: i32` that starts at the vanilla limit. Every part of the type edits it in dispatch
order and sees what the parts before it left, so two parts compose (one doubles, the next adds one). The
group stops growing once its count reaches the final size, so a size of 0 or less ends it with the mob just
spawned.

```rust,ignore
struct ZombiePacks;
#[mod_api::sunrise_part]
impl mod_api::EntityPart for ZombiePacks {
    fn spawn_cluster_size(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::SpawnClusterSizeEvent,
    ) {
        event.size = event.vanilla * 2;
    }
}
```

Only natural spawning's packs ask it: vanilla has no group limit anywhere else. A type with no part keeps vanilla's limit and costs nothing. The mob (`event.mob`) is a `MobRef` to a mob already in the level, so it reads and changes like any resident mob, and the call runs on replayed ticks like `finalize_spawn`, so an answer must be a pure function of its inputs. Attach the part with `r.entity_part("minecraft:zombie", ZombiePacks)` as [entity parts](../../concepts/parts/entity-parts-and-mob-goals.md) describe.

## Editing bred babies and split children

The baby hook's `player` and `player_entity_id` name the same player. The game resolves
who fed the initiating parent first, then who fed the partner if the first player is no
longer in the level. Both fields are `None` if neither feeder resolves.

`baby_entity_spawn` (hook version 4, NeoForge's `BabyEntitySpawnEvent`) fires after animal breeding has attempted to construct the baby (`Animal.spawnChildFromBreeding` / `Fox.FoxBreedGoal.breed`) and before breeding snaps the baby, finalizes breeding, and inserts it. `event.child` is a scoped `BabyEntitySpawnChild<'_>` lending the single optional child slot: inspect it with `get()` (`Option<UnspawnedMobView<'_>>`), edit it in place with `get_mut()` (`Option<PendingChildEditor<'_>>` — equipment, attributes, tags, custom name, position, yaw, baby state, size), construct a replacement detached `UnspawnedMob` with `construct(host, PendingMobSpec::new(...))?` and swap it in with `replace(baby)?`, or clear the slot with `take()` / `clear()`. A replacement must be an `AgeableMob` (`minecraft:fox` for fox breeding), but it can start as an adult. The event runs inside the breeding goal or brain behavior, before either parent's cooldown and before the initiating mob's movement and age/love countdown. Returning `Flow::Cancel` breeds no baby and resets both parents to age 6000 and love 0; returning `Flow::Continue` with an empty slot skips spawning and finalization without resetting the parents or consuming credit. An accepted child is made a baby and snapped to the initiating parent's live position with zero yaw and pitch before joining. Ordinary animals award breeding credit after those updates and admit their experience orb before the baby; the baby's state stays unpublished until its own admission. Foxes grant trust and award credit against the child's pre-snap age and position, then reset the parents before updating the child. Any detached `UnspawnedMob` not left in the slot is discarded cleanly on drop.

```rust,ignore
fn baby_entity_spawn(
    &self,
    _: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::BabyEntitySpawnEvent<'_>,
) -> mod_api::Flow {
    if let Some(mut baby) = event.child.get_mut() {
        let _ = baby.add_tag("mymod:bred_baby");
    }
    mod_api::Flow::Continue
}
```

`mob_split` (hook version 4, NeoForge's `MobSplitEvent`) fires when a slime, magma cube or sulfur cube above size 1 dies and splits (`AbstractCubeMob.remove`), after vanilla has constructed its children (a primed sulfur cube above size 1 dispatches with an empty list). `event.children` is a scoped `MobSplitChildren<'_>` lending an unbounded mutable child list: inspect with `len()`, `is_empty()` and `get(i)`, edit in place with `get_mut(i)`, construct a detached `UnspawnedMob` with `construct(host, spec)?`, and mutate the list with `push(child)?`, `insert(i, child)?`, `replace(i, child)?`, `remove(i)` and `clear()`. An invalid spec or out-of-bounds `insert` or `replace` returns `UnspawnedMobError::Invalid`; `get`, `get_mut`, and `remove` return `None` for an out-of-bounds index. The original proposals fire `entity_constructing` before this event, even for children later removed or canceled. Returning `Flow::Cancel` (or clearing the list) spawns no children but keeps the construction draws and unrelated Host writes; the parent still dies. Otherwise the remaining children join once each in list order. The parent’s removal effects run afterwards against its current state: removing Oozing during the split, for example, prevents the extra slimes from that effect.

In either hook, `construct(host, spec)` fires `entity_constructing` before returning the detached child, even if you later discard it. Constructor attachments and identity remain unpublished until admission. The original split proposals are all constructed before any admission, even without a `mob_split` listener.

For either pending-child hook, a panicking callback is disabled for that hook for the session. The shared dispatcher reports the mod and hook in the error log and, when a local client is present, in an error toast. The callback's cancellation flag is restored, but direct edits to a child, slot or list are not journaled and remain visible to later listeners. This does not change rollback for writes covered by an existing Host transaction.

```rust,ignore
fn mob_split(
    &self,
    _: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::MobSplitEvent<'_>,
) -> mod_api::Flow {
    if let Some(mut first) = event.children.get_mut(0) {
        let _ = first.add_tag("mymod:first_split");
    }
    mod_api::Flow::Continue
}
```

## Keeping or removing a mob

`mob_despawn` fires once per tick for every mob whose chunk is entity-ticking, just before vanilla's `Mob.checkDespawn`, persistent mobs and mobs in peaceful mode included. Player tickets make chunks entity-ticking across the whole view distance, so a mob beyond the simulation distance is still checked, though it isn't ticked. A mob in a loaded chunk that is not entity-ticking is neither checked nor told. `Allow` discards the mob, `Deny` keeps it, `Default` leaves vanilla's rules to decide. An answer other than `Default` skips all of vanilla's checks, so a `Deny` keeps a hostile mob in peaceful mode and neither answer takes the idle-despawn `nextInt(800)` draw.

```rust,ignore
fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
    if event.kind == "minecraft:zombie" {
        *event.result = mod_api::Despawn::Deny;
    }
}
```

The hook's scoped read host resolves the ticking mob directly and peers through the tick snapshot. An already-ticked peer exposes its post-turn state; a not-yet-ticked peer exposes its pre-turn state. A mob spawned during the pass does not resolve through this scoped view until the next pass. Removed mobs do not resolve, but a dead mob still present in the level is not removed and can still resolve. `living_change_target` uses the same view when fired inside this tick pass; outside it, that hook reads the entities available in the game host, not this snapshot.

The wither and the ender dragon override `checkDespawn` in vanilla, so the event is not fired for them.

## Adding or removing a custom spawner

The level's custom spawners (for the Overworld: wandering trader, patrols, cats, village sieges, and phantoms; other dimensions start empty as in vanilla `MinecraftServer#createLevels`) are a list a mod edits once per server level when it opens, in `modify_custom_spawners`. The listener gets the level (`event.level`) and an editor over the ordered list (`event.spawners`): read every entry, built-in or a mod's, with `entries`, `get` and `position`; take one out with `remove(SpawnerKind::...)` or `remove_at`; put an owned `CustomSpawner` (a struct implementing `CustomSpawner` or a `FnMut(&mut dyn ServerHost, LevelRef, bool) + Send + 'static` closure) at any place with `insert` (0 is before the built-ins) or at the end with `add`; reorder with `move_entry`. Edits are staged and applied after every listener ran, and a listener that panics has its edits undone, so another mod's instance it removed is still in the level. The level owns each registered spawner for its lifetime, ticks every entry in one ordered walk (including `Phantom` before mod-appended spawners) once per eligible server tick (outside debug worlds, with `spawn_mobs` enabled) inside a server transaction, and drops it when the level unloads or the world closes.

The callback's `spawn_enemies` is the chunk source's stored setting. The server samples `spawn_mobs && spawn_monsters` at startup, on difficulty updates, and when `spawn_monsters` is set (`MinecraftServer#updateMobSpawningFlags`). Changing only `spawn_mobs` changes whether the pass runs, not the stored setting. Peaceful is not a separate filter on that setting.

```rust,ignore
struct RatSwarmSpawner {
    cooldown: u32,
}

impl mod_api::CustomSpawner for RatSwarmSpawner {
    fn tick(&mut self, host: &mut dyn mod_api::ServerHost, level: mod_api::LevelRef, spawn_enemies: bool) {
        if !spawn_enemies {
            return;
        }
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return;
        }
        self.cooldown = 200;
        // spawn on `level` through `host`
    }
}

fn modify_custom_spawners(&self, _: &dyn mod_api::Host, event: &mut mod_api::ModifyCustomSpawnersEvent<'_>) {
    // no phantoms in this world
    event.spawners.remove(mod_api::SpawnerKind::Phantom);
    event.spawners.add(RatSwarmSpawner { cooldown: 0 });
}
```

See [the custom spawner contracts](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/custom_spawners.rs).

## Spawning from a level

`LevelRef::spawn_mob(host, kind, pos, reason)` is `EntityType.spawn` for a mob at rest, the same living route `spawn_entity` takes with `SpawnBody::Living`: the mob is made as any of its type is, Parts, attachments and `entity_constructing` included, then `finalize_spawn` and `finalizeSpawn` run with `reason`, and it joins the level (`entity_join_level`; inside a transaction, at its root commit), not persistent. It returns the mob's handle, or `None` with nothing spawned: a kind that isn't a mob, a position that isn't finite, outside `Level.isInSpawnableBounds` or in an unloaded chunk, a level not in place, a client, or a mod that cancelled the spawn or kept the mob out.

`LevelRef::spawn_entity(host, kind, pos, &spawn)` spawns one entity of `kind` at `pos` and returns its handle, or `None` with nothing spawned. `EntitySpawn::new(body)` starts the spawn at rest, unturned and with no owner; set its `velocity` (blocks per tick, `Entity.setDeltaMovement`, set before it joins), `rotation` (`[yaw, pitch]`) and `owner` (the entity that fired it) as needed. Its `SpawnBody` names the family, and the call goes the way that family's own spawns go:

- `SpawnBody::Thrown { item }`: a thrown item projectile (vanilla's snowball, egg, ender pearl, experience bottle, wind charge, splash or lingering potion, or a content mod's thrown type) through `Projectile.spawnProjectile`, the route a thrown snowball takes. Its owner is set before it joins (`Projectile.setOwner`: hit credit, saved as `Owner`, and the shot keeps clear of its owner until it has left the owner's box), `entity_join_level` decides whether it joins, and it carries one of `item`. With no `rotation` it faces along its velocity, as `Projectile.shoot` leaves it. Its owner is a player, a mob, a boat or a minecart: the shot needs the owner's collision identity to keep clear of it, so any other owner refuses the spawn.
- `SpawnBody::Living { reason }`: a mob (vanilla's or a content living type) through `EntityType.spawn`, the route `/summon` takes: made as any of its type is, Parts, attachments and `entity_constructing` included, then `finalize_spawn` and `finalizeSpawn` run with `reason`, and it joins the level (`entity_join_level`), not persistent. With no `rotation` it takes `EntityType.spawn`'s yaw. Both calls refuse a type that is disallowed in Peaceful when the level is Peaceful, before the mob is built or any spawn hooks run. A mob takes no owner. `kind` resolves as `/summon`'s does (`cow` is `minecraft:cow`). The ender dragon takes its own creation, facing `rotation`'s yaw; it has no velocity or pitch here, so a dragon given either is refused.
- `SpawnBody::General`: a row of a general type a mod declared (`declare_general_entity`), with a fresh UUID and the type's declared payload version, through the insertion `/summon` and chunk loading share. It is admitted as one record (no `entities_load`: no save was read) and joins as a fresh spawn. Every declared `FeatureFlag` must be enabled in the world. For example, check `level.enabled_features(host).is_some_and(|flags| flags.contains_flag(FeatureFlag::RedstoneExperiments))`, or use `flags.contains_all(&required)` for a slice of requirements. Its owner is kept on the row and saved as `Owner`. Its state is its attachments: write them on the returned handle inside the same transaction, and `entity_join_level` and the row's first tracking see them.

A general row can move to another position in its level through `EntityRef::teleport_to`. A row teleported on its own dismounts; a teleported vehicle takes its riders with it. Removing the vehicle dismounts its riders and leaves them in the level. `EntityRef::set_rotation` turns a general row without moving or dismounting it; the new angles reach clients through ordinary tracking.

In `declare_general_entity`, pass `pickable` after `summonable`. Pass `false` for
vanilla's plain `Entity.isPickable` default. With `true`, the crosshair can hit
the declared box as soon as the row spawns on the client. The ordinary use and
attack packets name that row, and the existing `entity_interact` and
`attack_entity` hooks receive its id and declared kind. Without a listener's
response, use on the row passes and the row declines damage. This does not make
the box solid or give it a middle-click item. The next argument, `fire_immune`,
is vanilla's `EntityType.fireImmune()`: with `true`, the row never reads as
burning and its fire ticks are put out each tick. The argument after it declares how the
row reacts to pistons: pass `None` for `PushReaction::PushPull`, or
`Some(PushReaction::IgnoreEntity)` to leave the row in place. The declared
reaction is readable on either side.

`LevelRef::spawn_falling_block(host, state, pos, velocity, falling)` spawns a falling block
(`FallingBlockEntity`) of `state` at `pos` (its bottom centre) in the named open server level, even
when that level is parked. It leaves the block at `pos` alone. `Falling` sets `drop_item` (default
`true`) and `hurts` (default `false`, which keeps the carried block's default damage). Enabling
`hurts` sets the damage from `AnvilBlock.falling`: 2 per block fallen up to 40. Live velocity stays
exact in blocks per tick (`Entity.setDeltaMovement`); packets quantize only on send (`LpVec3`).
Any valid block state is accepted, including air, which vanilla discards on its first tick.
Admission follows `ServerLevel.addFreshEntity`: no per-level count cap or `/summon` position
check, and the call neither requires nor loads the destination chunk. Section status governs
entity visibility and ticking. Keep the returned `EntityRef` to work with the entity
in its destination level while its column is hidden; ID and UUID lookup, entity visits,
and ray traces do not discover it until the section is accessible.

The spawn runs `entity_constructing` then `entity_join_level`, just like a natural fall. A natural
fall asks for admission before the next scheduled block tick; if vetoed, the source block's
replacement still stands. `spawn_falling_block` returns `Ok(Some(EntityRef))` when admitted,
`Ok(None)` for non-finite coordinates or velocity, exhausted entity ids or a join veto, and
`Err(LevelWriteError)` for a stale or closed level, a client, or an unknown block state.
Only a transaction in the destination level delays the join until its root commit; the
returned handle is provisional and becomes stale on abort or a veto at commit.

`EntityRef::owner(host)` reads the owner back (`TraceableEntity.getOwner`) for a projectile or a general row (`ProjectileRef::owner` answers the same for a projectile): a general type's own tick uses it to skip its shooter and to name the attacker of the damage it deals. A spawn is refused for a kind of another family than its body, a position, velocity or rotation that isn't finite, a position outside `Level.isInSpawnableBounds` or in an unloaded chunk, an owner that is stale, in another level, or without a collision identity for a thrown spawn, a level not in place, a client, or a mod that cancelled the spawn or kept the entity out.
Inside a transaction the join waits for the root commit, so a spawn a mod then keeps out still returns a handle, which reads stale from the commit on. The level's spawn limits, flags and spawn rules don't apply, as they don't to `/summon`; nor does a general type's `summonable`, which is `/summon`'s alone. Items, experience and block drops have their own calls (`spawn_item`, `award_experience`, `pop_resource`), and any other entity `LevelRef::summon(host, kind, pos, nbt)` (which also routes parked levels outside a transaction and returns `Result<Option<EntityRef>, LevelWriteError>`).

A level's natural spawning (`NaturalSpawner.spawnForChunk`) also has a policy a mod can change:

- `spawn_limits(host)` and `set_spawn_limits(host, limits)`: a `SpawnLimits` with each spawning category's `getMaxInstancesPerChunk`, `SpawnLimits::VANILLA` (70 monsters, 10 creatures, 15 ambient, 5 axolotls, underground water and water creatures each, 20 water ambient) until a mod sets others. A limit caps the category twice, as vanilla's does: over the level, under `limit * spawnable chunks / 289`, and around each player near the chunk, under `limit`. A limit of 0 stops the category.
- `spawn_flags(host)` and `set_spawn_flags(host, flags)`: a `SpawnFlags` with `monsters` and `friendlies`, both on until a mod turns one off. Off keeps that side out of the level's natural spawning; neither turns monsters on against the `spawn_monsters` rule or anything on with `spawn_mobs` off.
- `can_spawn_entities_in_chunk(host, [x, z])`: `ServerLevel.canSpawnEntitiesInChunk`, the question natural spawning asks of a spawn position outside the chunk it started in. Sunrise's spawner answers it as "the chunk is loaded", and so does this call.

```rust,ignore
fn level_load(&self, host: &mut dyn mod_api::ServerHost, event: &mut mod_api::LevelLoadEvent<'_>) {
    let Some(level) = host.level(event.dimension) else { return };
    // twice the monsters, no bats
    level.set_spawn_limits(host, mod_api::SpawnLimits { monster: 140, ambient: 0, ..mod_api::SpawnLimits::VANILLA });
}
```

Only natural spawning reads the policy: spawners, trial spawners, patrols, sieges, phantoms, a new chunk's creatures and custom spawners keep their own rules. A change counts from the level's next spawn pass, and mobs already there stay. The policy is not saved. It survives parking and reopening a level within the same world session, but closing the world resets it. Set it in `level_load`. Reads work on any host and answer `None` on a client or for a stale handle; the writes need a `ServerHost` and return `false` there.

## Raid handles

Use `LevelRef::raid`, `raid_at` and `raids` to obtain raid handles. They resolve their
own loaded server level even while another dimension is in place. Raid reads,
omen-level writes and `stop` do not swap levels. Raider lists and living health
come from that level's mobs; raid ids are local to a level. A closed level, an
earlier world epoch or a client refuses the handle. Entity handles in a parked
raid's member list keep their identity; their ordinary entity operations
require the entity's level to be in place.
