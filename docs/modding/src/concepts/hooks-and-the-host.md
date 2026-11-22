# Hooks and the Host

A **hook** is one moment in the game: a tick, a block broken, a player joining. Your mod implements the hooks it
needs as methods of `mod_api::Hooks`, and the game calls each subscribed mod with the moment's **event** and a
**host**, its typed view of the game. A moment that belongs to one object is a method of a [Part](parts.md)
instead, called only for the objects your mod attached it to.

![The game calls every mod that implements a hook, and a Part only for its own objects. Both receive an event and
reach the game through the Host, its handles, capabilities and attachments.](../images/hooks-parts.svg)

## Dispatch

Mods run in order of their listener priority, highest first, then in dependency order: a mod after the mods it
depends on. When a mod cancels an event, the mods after it are skipped, unless they receive cancelled events
([Listener priority and cancellation](#listener-priority-and-cancellation)).

What a hook's return value means depends on its kind:

- **Listen** hooks are notices: every subscribed mod is called.
- **Cancellable** hooks return `Flow`: `Flow::Cancel` stops the game's action and skips later listeners unless they receive cancelled events.
- **Replacement** hooks return an `Option`: the first mod to answer `Some` wins.
- **Collection** hooks call every mod, each adding to a list in the event.

A hook with no receiver gets no `Host`.

## Failures and panics

The dispatcher catches panics in hook callbacks, including chunk-disk callbacks and startup
`Hooks::register`. A panic disables only that mod's callback for that hook for the session. The game
names the mod and hook in the error log and in a [client diagnostic toast](../how-to/client/toast-notifications.md#admission-rules)
that mods cannot cancel or rewrite. A dedicated server logs the error without showing a toast.
Ordinary refusals, such as an unavailable phase operation or a rejected output, are reported every
time they occur and do not disable the callback.

A failed proposal is not a cancel or replacement. Its changes to the event, including the cancel
flag, do not publish. Later eligible listeners start from the last accepted result, not from the
original request. An accepted cancellation still skips listeners that don't receive cancelled
events; a successful replacement still ends a first-answer-wins hook.

[Staged registration](load-phases.md#registration-and-setup) discards the failed callback's pending
entries and replacements. For a journaled action such as `farmland_trample`, a failed callback's
writes do not join the action. A successful veto or stale landing state still aborts the whole
trample action, and fall damage still runs. For [chunk-disk jobs](../how-to/chunk-data-load.md), later
listeners can run after a panic or codec failure, but the failed job's changes cannot publish.

## Player tick hooks

`player_tick_pre` and `player_tick_post` bracket the player's body tick on both
logical sides. On the client, both fire through death tick 20, but neither fires
for the removed local player while the death screen remains open. On the server,
neither hook fires for a spectator whose bounds touch an unloaded chunk: that
player's body tick is skipped. Connection and
level upkeep and outgoing state updates still run. The server resolves whether
the body runs before the pre hook; a listener changing the player's position
does not change that decision for the current tick.

## Entity tick hooks

`entity_tick_pre` and `entity_tick_post` version 3 observe additional entity families, including
server players, named projectiles, displays and other entities the server keeps without simulation.
Whether a call represents one entity or a family depends on the tick route and the types named in
registration. Keep cancellations within the current Pre call. The player connection body remains
separate from the server-side entity tick, and the local client avatar is not observed by these hooks.

Removing a general entity during its own Pre skips its tick and Post. A passenger dismounted by that
removal can still tick as a root if it is eligible, has not ticked, and its join-order slot is still ahead.
The pass does not revisit an earlier slot. Canceling an entity's Pre does not by itself skip its passengers'
ticks, though a listener can separately change the riding graph.

`general_entity_tick` and `on_entity_item_update` lend the live server subject
through `event.subject.with_view`. Use `event.subject.host()` between view
borrows for other objects; a later view borrow returns `None` if the subject is
no longer resident. Use `event.subject.transaction(|scope, tx| ...)` to commit
subject writes together with other journaled Host writes. Returning `Err`
restores those writes. This covers velocity, rotation and fall distance on a
general entity, and the stack, pickup delay, pickup target and velocity on a
dropped item. Client scopes have no `transaction` method.

`on_entity_item_update` dispatches to the parts for the item ID held when
that hook starts. Read the current stack with
`event.subject.with_view(|item| item.with_stack(|stack| stack.count())).flatten()`.
Call `stack.project()` inside that loan only when you need components or an
owned copy.

## Explosion membership

`explosion_detonate` version 2 exposes mutable `blocks` and `entities` lists
before an explosion applies its effects. The initial entity query excludes the
exploding source. Remove entries to spare them, or add live handles to make them
eligible; additions still obey normal distance, exposure and immunity checks.
Stale or invalid handles are dropped and repeated IDs are visited once. A mod
can explicitly add a valid source handle back; per-kind immunity still applies.

The final list controls membership, not global effect ordering. Its order is
used for ordinary storage targets. Crystals/dragon and players retain their
earlier separate passes, while frames and other decorations run last in storage
order. Block positions are shuffled before destruction. An empty entity list
spares direct entity effects, not indirect consequences of destroying support
blocks. With no subscriber the game does not build a target list.

## Liquid containers

A `BlockPart` can implement `can_place_liquid` to let its block accept fluid without inheriting a
waterloggable template. Return `Some(true)` to accept as a container, `Some(false)` to refuse as a
container, or `None` to keep the template's rule. A bucket can still replace a replaceable block or
try the neighbouring cell. The read-only query hears both sides of bucket use, and server
spreading and dispensers. A player's bucket lends that player as `user`; the fluid utility lends
its optional player. Spreading and dispensers don't lend a user.

`place_liquid` is the server write: return `Some(true)` after storing the fluid and scheduling any
needed tick, `Some(false)` if it was not placed, or `None` to pass to the next part, then the template.
The first `Some` wins. Buckets supply a source state; spreading supplies the arriving state. Live
structure templates restore the displaced fluid or a neighbouring source, without calling
`can_place_liquid`. A normal bucket still empties if the container returns false; a validated
fluid-container operation fails instead.

Player buckets and the fluid utility journal each part's writes separately: a false answer or
fallthrough discards its writes before the next part or the template can see them. Root commit
applies only the final state at each changed position, not the intermediate states. If any committed
write there requested a neighbour, client or immediate update, that request survives; shape
updates, drops and other effects are suppressed only if all writes there suppressed them. Aborted
writes do not contribute. Dispensers, spreading and live structure restores use a direct host;
parts on these paths must not write before returning `None` or false. If a mod transaction is open,
these paths skip the part and log the refusal, leaving the template's answer. Both hosts calculate
`container_signal` from stored slots without generating deferred loot.

## Saving and game events

To save from inside a hook, use `level.request_save(host)` so the save runs at the next settled
start-of-tick boundary. A synchronous `save-all` while an entity pass or held-out row is borrowed
fails with vanilla's `commands.save.failed` feedback and a failed command result, without saving.

To emit a vibration-producing game event, use `level.game_event(host, event, at, context)`.
`GameEventContext` carries an optional source entity and affected block state; see
[game events with context](../how-to/world/game-events.md) for listener semantics and refusals.

## Listener priority and cancellation

`#[listener]` on a hook method sets both, as NeoForge's `EventPriority` and `receiveCanceled` do. Two mods, each
its own crate:

```rust,ignore
#[mod_api::sunrise_mod]
impl Hooks for Protection {
    // Runs before mods at the default priority, 0; higher runs first.
    #[listener(priority = 100)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if self.protected(host, event.pos) { Flow::Cancel } else { Flow::Continue }
    }
}

#[mod_api::sunrise_mod]
impl Hooks for Override {
    // Receives cancelled events and may clear cancellation.
    #[listener(priority = -100, receive_cancelled)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if event.cancelled && self.allowed(host, event.pos) {
            event.cancelled = false;
        }
        Flow::Continue
    }
}
```

`priority` is an `i32`, 0 when omitted; mods of equal priority keep dependency order. Each hook's order is
fixed once, when the mods are installed. Every cancellable hook's event has a `cancelled` field: a mod without
`receive_cancelled` always sees `false`, since it is not called once the event is cancelled. A mod that receives cancelled events and sets it back to
`false` lets the game's behaviour run, and the mods after it are told again. An event the game hands over already
cancelled reaches only mods that receive cancelled events. `receive_cancelled` on a hook without a cancel does not
compile, and `#[listener]` is for a mod's hooks only: a part is called for its own thing, in no order.

### Canceling or consuming a projectile

In `projectile_impact`, use `host.entity(event.projectile)` to resolve the live projectile,
even if the captured hit would normally consume it. Canceling alone suppresses the hit and
lets the projectile continue according to its type. Call `EntityRef::discard(host)` to consume
it without applying the hit. A discard makes the handle stale immediately; another listener
clearing cancellation cannot bring that projectile back. If a listener changes the projectile
or its target so the captured hit no longer applies, the game skips that hit instead of tracing
it again.

## Events and shared state

Each hook method takes its host, where it has one, and one event:

```rust,ignore
fn living_incoming_damage(
    &self,
    _level: &mut dyn HitHost,
    event: &mut LivingIncomingDamageEvent<'_>,
) -> Flow {
    if event.kind == "minecraft:pig" {
        let halved = event.container.new_damage() * 0.5;
        let _ = event.container.set_new_damage(halved); // refused writes leave the hit unchanged
    }
    Flow::Continue
}
```

Each accepted hit has its own `living_damage_post` after its hurt sounds and any death or
protection that hit caused, before the hurt caller resumes. This is not a next-tick or turn-exit
notification. The next hazard, poison application or rider's hit observes the preceding hit
completed; a later hit never shares the first hit's `DamageContainerView`. A mob's queued effect
hurt finishes before the next action, and a dolphin's dry-out hurt finishes before its hop on land.
A bee finishes drowning damage before checking stinger expiry. The bee uses the stung state
it read before drowning, but the expiry hit uses its current health. The expiry hit finishes
before the bee updates nectar and anger and runs its movement controls.
A server player's landing hurt settles death or protection and Post on the landing body before
`HIT_GROUND`, the fall-distance reset, and the next accepted move. In a mob's own tick, landing
hurt settles before the fall-distance reset, the rest of movement, a gliding collision hurt or
another goal. This includes `GoalSpec::BreathAir` installed with either `MobRef::add_goal` or
`MobRef::add_target_goal`, and a dolphin's move to follow a rider. The target selector finishes
before the action selector ticks. Trampling farmland is still decided before the landing hurt.
A pufferfish's sting on a mob finishes the hurt, including any death and `living_damage_post`,
before applying that sting's poison or starting the next sting. The fish's own `entity_tick_post`
follows all of its stings.
Direct projectile hits finish before the on-hit follow-up: for example, shulker bullet levitation,
trident Channeling, or the damage from a large fireball's explosion. A mounted fall settles each
rider's hurt before continuing to the next hit. A horse's own hurt finishes before the fall is
passed to its passengers.

For incoming damage, `event.weapon` is the direct entity's weapon, including a projectile's
saved firing weapon. `event.causing_weapon` is instead the causing living entity's current
main-hand weapon (`DamageSource.getEntity().getWeaponItem()`): an off-hand crossbow shot can
name the saved crossbow in `weapon` and the shooter's main-hand sword in `causing_weapon`.
An absent, unknown or non-living cause has no `causing_weapon`.

The event is a generated struct named after the catalog hook, in CamelCase plus `Event`.
The game constructs it and passes it to the listener. It is `#[non_exhaustive]`: a mod cannot build it
with a struct literal, and a destructuring pattern must include `..`. Listeners still read and write
the public fields as the hook allows. The hook point's arguments are values or references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. To exercise dispatch behavior, use the hook's dispatch function
([Testing a mod](testing.md)).

The generator validates edited stack-list outputs inside each listener's owning scope. A refused stack
output retains the immediately preceding accepted value, not the event's initial value. Scalar
outputs and cancellation still commit; later listeners see the accepted stacks and run independently
under the usual cancellation rules. Each refused output logs one warning naming the mod, hook,
output and conversion error.

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `LevelRef::set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.

## Which thread runs a hook

Most hooks run on the thread that ticks their side: the server thread for a server hook, the window's thread for a
client hook. Single player has both in one process, so one mod instance is called from two threads. Some hooks and
Part methods run elsewhere, and each one's own documentation says where:

- **Worldgen workers.** While chunks generate, the methods of a structure type, feature type, biome source or chunk
  generator that place or sample the world, and the hooks fired while a feature generates, run on the worldgen
  worker threads, several chunks at once. Feature placement also runs on the server thread when the game places a
  feature there: a sapling growing, a command, `LevelRef::place_feature`. A query asked once at start, such as
  `BiomeSource::possible_biomes`, is not a worker call. They get a `WorldgenHost` or no host at all, never a `Host`.
- **Meshing workers.** `add_section_geometry` itself runs once at client start, to register renderers. A panic or
  phase refusal in registration discards only that callback's renderers. Each renderer's `SectionGeometry::add` then runs on the renderer's meshing workers every time a section is meshed. A
  block tint source is asked there too, and also outside meshing, for example for terrain particles and blocks a
  piston moves. Both get no host.
- **Chunk workers.** `chunk_data_load` and `chunk_data_save` run where a chunk's record is decoded or encoded, with a
  restricted host. `host.job()` borrows the captured `ChunkDiskContext`; the callback edits only its job-local
  [attachment store](../how-to/chunk-data-save.md), never the live chunk.
- **Loading threads.** Data-load hooks run on whichever thread loads the data.

What follows for a mod's state:

- A `thread_local!` holds one value per thread, not per game or per world. A value written on the server thread is
  not there on a worldgen worker, and the window's thread has its own. Keep shared state in the mod's own fields
  behind a lock or an atomic, or on the object it belongs to as an [attachment](attachments.md).
- Do not hold a lock across a host call. A write through the host can fire another hook of the same mod on the same
  thread (`LevelRef::set_block` fires `block_changed`), and that hook then waits for the lock its caller holds.
  Copy what you need out, release the lock, then call the host.
- A worker's call can run at the same moment as a tick hook. Data a worker reads belongs in an immutable value
  swapped as a whole, or behind a read-write lock. A lock makes the access safe, not the result right: worldgen
  must give the same chunk for the same seed whatever the order of calls, and a section geometry renderer or tint
  source must answer from its arguments and immutable data fixed at registration, as their own documentation requires. Mutable state does not belong in them.

`Hooks` contains global hooks. Scoped hooks belong to `BlockPart`, `BlockEntityPart`,
`ItemPart`, `MenuPart` or `FluidPart` and run for the registered object.

## Item tooltip context

The item's `tooltip` hook (`Item.appendHoverText`) receives `event.context`, a
borrowed `TooltipAppendContext`. Component `TooltipAppender::append` callbacks
receive the same context: local player identity, `advanced` (F3+H), `creative`
(creative-tab slot), `nested` (a loaded crossbow projectile), and `tick_rate`.
The rate follows vanilla 26.3's `Item.TooltipContext.tickRate()`: the current
client level's tick-rate manager, or 20 when no level is present. It is not
measured TPS or the frame rate. The item hook still does not fire for nested
projectiles or hidden tooltips; adding context does not change line ordering.

## Level hosts

Level hooks use access-specific hosts rather than `Host`. `LevelHost` provides
`block_state` through `LevelAccess::block_state`, client-side detection and
`state_with_property`. `state_string(state)` serializes the full name and properties, or returns `None`
for an unknown state; `min_y()` and `height()` read the level's build range. It also reads light and loaded state, through the game's own queries:
`brightness(LightLayer::Block | Sky, pos)` (`getBrightness`, the stored 0 to 15),
`raw_brightness(pos, sky_darken)` (`getRawBrightness`, which `CropBlock.randomTick` asks with 0)
and `is_loaded(pos)` (`Level.isLoaded`: inside the build height and in a loaded chunk),
`is_raining_at(pos)` (`Level.isRainingAt`, false where `pos` is not loaded) and `enabled_features()`
(`LevelReader.enabledFeatures`, a shared `FeatureFlags` with the world's effective flags). An omitted
flag list in the save means `minecraft:vanilla`, not an empty set. These queries do not allocate.
None of them loads a chunk. Light can be available outside the build height or where no chunk is
resident, so check `is_loaded` first when you need a resident block. `LevelHost` also answers
tag and fluid questions without a `Host`: `block_in_tag(block_key, tag)`, `fluid_state(pos)`
(`getFluidState`, `None` where the block read is),
`state_fluid_state(state)` and `fluid_in_tag(fluid_name, tag)`. Fluid reads return `FluidState` with a
`FluidKey`, amount, falling flag and engine-derived source flag; `is_empty()` and `is_source()` report
those states without a registry lookup. `block_state_id(name)` parses a block and properties, and `block_state_default(block)` gives a block's default state, as on `Host`; both return `None` for invalid input or unavailable context. For tag reads, use `Host::tags()`: resolve a `TagKey` with `Tags::key`, then call `Tags::is_in_tag` for item membership or `Tags::items`, `Tags::blocks` or `Tags::fluids` for ordered member lists. The list methods return `TagMembers::Missing` for an absent tag and `TagMembers::Members(keys)` for a loaded tag, even if `keys` is empty. They return `TagMembers::Unavailable` until that registry's authoritative tags have been applied or if a member key cannot be resolved. On clients `tags_updated` announces the applied server tags; the query never substitutes local resource-pack tags.
`default_components_bound` precedes `tags_updated` after item defaults are published and readable.
Both notices carry `TagsUpdateCause`: `ServerDataLoad { reload }` for the server's initial data
load or reload, and `ClientPacketReceived { integrated_server }` for a client's applied tags.
`should_update_static_data()` is true on the server and a remote client, but false on an
integrated client, whose server already updated the shared static data. Installing client tags
announces the bound defaults; it does not rebind those defaults.

`block_face_sturdy(pos, face, support)` reads `BlockState.isFaceSturdy` through the same engine
query as the level handle. It uses the server level's or current client level's local shapes,
including dynamic and published shapes, and returns `None` for unloaded or out-of-height
positions. `canSupportRigidBlock` is `(pos, Up, Rigid)`. `canSupportCenter` is `(pos, face, Center)`,
except that vanilla also refuses `Down` on blocks tagged `minecraft:unstable_bottom_center`.

`can_see_sky(pos)` tests the published sky light for 15, not the heightmap or block light. It
returns false everywhere in a dimension without skylight. Padding sections one section above and
below build height can carry stored light. Where sky light is not stored, it repeats the bottom row
of the nearest stored sky layer above, or returns 15 if there is none. Missing block light is zero.
An unloaded column can still retain light where a loaded neighbour supports its light sections.

Sunrise does not yet compute new light for the padding sections ([#3494](https://github.com/sunrise-org/sunrise/issues/3494))
or refresh unloaded borders when a block edit changes whether a section is empty ([#3495](https://github.com/sunrise-org/sunrise/issues/3495)).

Both reads return `None` in callbacks without the corresponding view. A `LevelRef` must still
name an open server level or the current client level. Neither read loads chunks or opens loot
containers. Both require the owning game thread, not a worker-thread hook body.

`block_entity(pos)` reads this level's block entity without loading a chunk; it returns `None`
when absent, unloaded or out for its tick. On a client it returns only the local block entity
handle. `container_size(at)` and `container_stack(at, slot)` are the same side-local capability
snapshots as `ItemsHost` supplies; neither generates unopened loot, and both refuse clients and
unloaded positions.

`BlockEntityPartState::state_in(level, entity)` and `state_existing_in` read the same attachment
snapshots as their `Host` equivalents, without inserting a default, generating loot or loading
chunks. A level view in an open transaction sees tentative Part-state writes in that frame.
Unloaded or stale holders fail; clients have no server Part state. All of these reads require the
owning game thread and are never available in worker hook bodies.

A placement part can ask its `LevelHost` whether a resident neighbour offers `ITEMS`, using
`has_item_capability(pos, None)` for an unsided answer. It sees the same state and block entities as
other reads on that view, including staged changes. A client asks its own providers about its
local data; placement still follows the server's result. See [Capabilities](capabilities.md).

`LevelRandomHost::container_signal(pos)` is the engine comparator container read. Unlike the immutable slot snapshot, it generates unopened loot once, as vanilla's `getItem` does, then calculates 0 to 15 using each stack's maximum size. It returns `None` on clients and unloaded positions, never loads a chunk, and runs only on the owning game thread. `analog_output` version 2 receives this mutable host so the loot generation and random draws happen in vanilla order; it permits no other world edit.

Redstone and shape reads on `LevelHost`, for placement and survival: `signal(pos, direction)`
(`SignalGetter.getSignal`), `direct_signal`, `direct_signal_to`, `best_neighbor_signal(pos)`,
`has_neighbor_signal(pos)` (what a lamp's or a door's placement reads) and `collision_shape(pos)` (the block's
collision boxes at `pos`, block-local, a block entity's published shape included, empty for no collision, `None`
for an unloaded position). The signal reads are the game's own, `block_signal` and `should_check_weak_power`
listeners included, and are the same ones `LevelRef` and the tick hooks give. `state_for_placement` and
`can_survive` receive a level that answers them, so a block can place or survive by what powers it or what shape
sits beside it. The levels of `state_for_placement`, `can_survive` and the level tick and shape-update hooks answer all of them;
the levels other callbacks get (a damage hit's, a part's read view) may return `None` for any of them, signal reads
and `collision_shape` alike.
A signal read never asks `state_for_placement` or `can_survive`, so it cannot re-enter the hook it was read in.
On a client (its placement prediction and its own shape updates ask `state_for_placement`, `can_survive`,
`may_place_on` and `can_sustain_plant` too) these reads answer from the client's loaded copy of the level, and the
reads above that refuse clients return `None`, as do `block_in_tag` and `fluid_in_tag`. `tags()` still returns a
service: `Tags::key` can resolve a key, but `is_in_tag` returns `None` and `items`, `fluids` and `blocks` return
`TagMembers::Unavailable` in this view. `BlockEntityPartState::state_in` gives its default there. Never answer from those as if
they were the server's: a client's own shape update that refuses a block removes it on that client.
`LevelTicksHost`'s signal reads moved here and now return `Option`.

Before a block's placement state is selected, `ItemPart::placement_context` replaces the item's
`BlockItem.updatePlacementContext` rewrite. It is keyed by the real pre-use `stack`, including its
components. `placer` gives the player handle and the eyes at the time of the click via `eye_position()`
(the acting player on the server, the predicting player on the client). It is `None` for a dispenser.
`Some(Some(context))` rewrites the target and click, even if the player's original destination is
occupied. The new destination must be loaded, in bounds and replaceable. The placing item cannot
change and click and view values must be finite. `Some(None)` refuses. `None` keeps vanilla, which
checks the original destination before its own rewrite (such as scaffolding's column walk). A
blocked dispenser still fails before this hook. `BlockPart::state_for_placement` then selects the
candidate, which may be a registered state of another block. The game uses that
candidate for survival, obstruction, and the written-block check that gates item data and placed-by
callbacks. The held item does not change, and its double-height pre-clear still runs if applicable.
Selecting a different block does not run that block's `getStateForPlacement`. If that block places a
companion, the mod must check its footprint: `setPlacedBy` can overwrite the companion cell without
rechecking it, and failing to write the companion does not refund the item. Sunrise additionally
requires creative-operator permission when a mod selects a game-master block. At the final `BlockItem.canPlace`
check, `ItemPart::must_survive` decides whether placement must pass `can_survive`: `Some(false)`
skips it, `Some(true)` requires it, and `None` keeps vanilla (scaffolding false, other block items
true). Standing/wall items retain their own unconditional candidate-survival checks. Entity
obstruction is still checked. Both hooks run on both sides with a read-only `LevelSide`.
`must_survive` still takes only the item id. Player and dispenser placement offer the resolved decision
once to `entity_place`; a veto stops all writes, including a double-height item's pre-clear, and
keeps the stack. If a listener changes the placement inputs, it is refused rather than retargeted.
The veto's placement neighbour and the post-placement notice use the resolved face,
and the notice uses the resolved click location. Neither changes the sides of the block placement or
survival hooks.

`level.state_for_placement(host, block, &context)` takes `&mut dyn ServerHost` and queries only
the level in play. It runs the block's `Block.getStateForPlacement`, including its
`state_for_placement` parts, without resolving the click again or placing anything. The caller
owns the `ServerHost`; the Part still sees only its restricted `LevelHost` through `LevelSide`.
The context's item is a registry key, not a stack or its components. The method returns
`Result<Option<BlockState>, LevelReadError>`. `Ok(Some(state))` gives the candidate before item
placement's additional survival, obstruction and permission checks. `Ok(None)` means there is no
candidate or a placement input was refused. A stale handle or closed level returns `Err(LevelReadError::Stale)`; a parked level,
an unavailable host or load phase, or a state the API cannot represent returns
`Err(LevelReadError::Unavailable)`. A camouflage block can use this read to store a held block's
placement state, such as a stair's facing or a log's axis.

A neighbour's `BlockPart::fence_gate_in_wall` answers whether a fence gate sits at wall height
beside that state. It runs on both sides at placement and shape updates, with a read-only `LevelSide`:
`Some(true)` counts as a wall, `Some(false)` does not, and `None` keeps the `minecraft:walls` tag
check. It changes only the gate's `in_wall`, not wall connections or pathfinding.

`BlockPart::is_possible_to_respawn_in` answers the forced respawn column's space check on the
server, with a read-only host. If no bed or anchor rule handles the spawn, the server asks for
both the saved point and the block above. This includes a bed or anchor at which spawn cannot be
set. `Some(true)` accepts that state, `Some(false)` rejects it, and `None` keeps vanilla's predicate.
It does not select a bed or anchor's stand-up position; `respawn_position` handles those.

`LevelHost` has no random access: a draw from a read-only hook inside a level write would shift
the level's random sequence away from vanilla's order. A hook that may draw is given a
`LevelRandomHost`. Use `host.with_random(|rng| rng.next_int_bound(bound))` to borrow its level's
live stream for that closure only. With a mutable `ServerHost`, use
`level.with_random(host, |rng| rng.next_int_bound(bound))` to select a resident server level,
even a parked one. If the stream can't be lent, the call returns `None` without running the closure.
Draws advance the live stream directly and are not rolled back on panic/unwind or transaction
abort, matching vanilla `Level.getRandom()` ([Randomness](randomness.md#borrow-the-live-level-stream)).

`LevelTicksHost` adds scheduling through `schedule_block_tick` and `schedule_fluid_tick`, while
`LevelWriteHost` adds random access, `set_block`, `set_block_property`, `play_sound` and
`send_particles` and `level_event` (`levelEvent`, queued in order with the write's own events).
A damage hit's listeners (`living_incoming_damage`, `living_damage_pre`, `living_damage_post`,
`living_shield_block`, `living_use_totem`) are given a `HitHost`: a `LevelRandomHost` that also plays level
sounds and draws from an entity's own random, and writes no block. `host.play_sound_at(sound, at, source,
volume, pitch)` is `Level.playSound(null, x, y, z, ...)` and `host.play_sound_from(entity, ...)` is
`Level.playSound(null, entity, ...)`: every player in range hears it, and a cancelled hit still plays what was
queued. The seed is drawn at the call from the level's sound-seed stream, not from the level's random.
`entity.next_float(host)`, `next_double`, `next_bool` and `next_int(host, bound)` draw from the entity's actual
random stream, in listener order. A hit lends its attacker where the game applies a melee hit (the attacking player or mob, for player → mob, mob → mob and mob → player) and no entity at any other damage site (environment, projectiles, explosions, potions and the like); the attacking mob's own random is the one its AI draws from, and a hit built inside a mob's own tick lends none. Any other entity, a stale handle, a client and an out-of-range `bound` answer `None` (`false` for a sound).
Outside a write, a `LevelRef` plays level events, and their global form for every player on the server
([Playing level events](../how-to/world/level-events.md)).
`set_block(pos, state, flags)` is `Level.setBlock` on a whole state, as `LevelRef::set_block` takes it: it
removes the old block's block entity and creates the new block's in the write (`LevelChunk.setBlockState`), runs
the neighbour and shape updates `flags` ask for, and queues the write's level events in order. A mod block entity
it creates starts with the data the `block_entity` hook gives, which is fired in the write and receives the level
(`LevelSide`, read-only: vanilla's `newBlockEntity` gets no level), as the `Host`'s `native_set_block` fires it. It returns true when the block
changed, and false off an authoritative server, for a position that is not loaded or outside the build height,
an invalid state, or the state the block already has. It never loads a chunk. The game owns
the tick queue; mods do not pass it back to these helpers.

## Drawing

Frame drawing through `DrawContext`, `BlockEntityDrawContext`, `HandDrawContext` and
`WorldDrawContext` stays in `mod_api`. In `render_hand`, use `draw.render_arm(MainHand::Left)`
or `MainHand::Right` to submit the local avatar at the current draw pose, independent of
the logical hand being rendered. See [Local player skins](../how-to/client/model-drawing.md#local-player-skins).
Raw GPU access is native-only, unstable and outside that API, in
`mod_render::render` (`sunrise-mod-render`). Replace its frame functions with native HEAD mixins compiled against the prebuilt game.
`render_needs` collects requests; `world_drawn` runs before the hand and HUD, and
`hand_drawn` runs after the hand, on the render thread. Use API draw contexts for API-typed drawing.

## After a hook that can change the world

A listener of a hook that gets a mutable host may remove, move or change objects. After such a hook
returns without cancelling, the game does not use the list position or the state it worked out before
the hook. It finds the object again by its entity id, checks the facts the action was decided on, and
abandons the action if the object is gone or a fact changed; it never works the action out again. The
hook still fires at its usual moment, and the check runs straight after it returns, before anything
moves. Stack checks are shallow: they compare the item id and the count, not components. Each hook's
doc names what it checks (`armor_stand_manipulate`, `entity_mount`, `cauldron_level_changed`,
`hanging_break`, `player_shear_entity`, `living_change_target`). The effect writes on a living handle
treat a mob that is marked removed but still listed as absent: they return false and change nothing.
