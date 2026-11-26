# Entity hooks

Implement catalog hooks as methods of `mod_api::Hooks`, with `#[mod_api::sunrise_mod]` on the `impl`.
The attribute derives subscriptions from the implemented methods. See
[Writing hooks](../../start/writing-hooks.md).

Native rendering callbacks such as `sunrise_mod_render::render::world_drawn`,
`hand_drawn` and `world_views` use a HEAD mixin. The launcher compiles the patched
function against the prebuilt game. Callbacks borrow the frame's GPU resources;
`world_views` supplies additional camera views. The renderer refuses invalid,
duplicate and oversized view requests by name, as `push` returns them.

Use the geometry types re-exported as `sunrise_mod_render::render::{Vertex, EntitySpan}`.
Implement the `render_needs` callback to set `RenderNeeds` requests for world depth
and entity geometry; these requests feed the engine's frame requirements so it
retains the resources your drawing callbacks borrow.

## Reading a hand or equipment slot

Resolve an entity with `host.living(entity)`, then use `living.with_equipment(host, slot, read)`
for one equipment slot or `living.with_hand_item(host, hand, read)` for the selected main-hand
stack or off hand. Both return the callback's result in `Some`; an empty or unavailable slot
returns `None` without running the callback. On the client only the local player's slots are
available.

```rust,ignore
let count = living.with_hand_item(host, Hand::Main, |stack| stack.count());
let helmet = living
    .with_equipment(host, EquipmentSlot::Head, |stack| stack.project())
    .flatten();
```

Use `project()` when you need components or an owned stack after the callback. For the
server-side attacker's live weapon, use `with_weapon_item`; for the weapon recorded by a
projectile, see [Death sources and weapons](../items/death-loot-context.md).

## Checking an entity's tags

In a server hook that gives you an entity and a host, use
`entity.tags(host).is_some_and(|tags| tags.contains("mymod:marked"))` to match a
scoreboard tag without allocating. To read all tags, the
[borrowed view](../../concepts/parts/entity-parts-and-mob-goals.md#entity-tags)
yields UTF-16 slices in saved order. Choose checked or lossy conversion explicitly
when you need UTF-8.

## Opening a container

`player_container_open` runs synchronously after a successful open. Failed opens and opening the player's
own inventory do not notify. Replacing an open menu closes it before announcing the new one.

## Leaving a bed

`player_bed_leave` is a server-side, listen-only notice. It runs before the
sleeping position and pose are cleared, so a listener can still read the player
as sleeping at `pos`; it cannot cancel the wake. For example, a listener recording
why a player left a bed should retain both flags rather than treating every wake
as a manual leave:

| Wake reason | `(wake_immediately, update_level)` |
| --- | --- |
| Morning after skipping the night | `(false, false)` |
| Explicit leave or bed-rule refusal | `(false, true)` |
| Generic stop-sleeping, missing bed or death | `(true, true)` |

`wake_immediately` resets the sleep counter to **0** when true, skipping the
fade-out; false sets it to **100** for the normal fade-out. `update_level` is false
for the morning wake because the sleeping list has already been handled, and
true for the individual wakes above. The same pairs apply to the acting player
and stored guests. Waking that same player again inside the callback does not
emit a second notice.

See the [generated API reference](../../reference/api.md) and the hook's
[caller notes](../../reference/hook-notes.md#player-bed-leave-callers).

## Dropped-item expiry

`item_expire` runs on the server when a dropped item reaches its lifespan (normally 6000 ticks),
before the game decides whether to discard it. Read the stack from `event.stack` and the drop's
position with `host.entity(event.entity).and_then(|entity| entity.position(host))`.

To extend its life, add to `event.extra_life` with `wrapping_add` so contributions combine like
NeoForge's signed 32-bit integers. The game adds that amount to the lifespan with the same
arithmetic, clamps the result to `0..=32766` (`Short.MAX_VALUE - 1` in NeoForge's `ItemEntity.tick`),
and leaves the current age unchanged. The drop survives only if its age is below its new lifespan
and it hasn't otherwise been removed.

To observe actual removals instead, listen to `entity_leave_level` and check for
`event.kind == "minecraft:item"`. It reports departures from any cause, not just expiry. A drop
discarded at expiry leaves before the next drop ticks; extending its life does not fire a leave
notice.

## Physical movement decisions

`entity_move` runs on both sides (movement phase 3, `Entity.move`) before one
physical `Entity.move` invocation, including its no-physics and stuck-multiplier
handling. It fires on every side that simulates the body (vanilla
`Entity.canSimulateMovement`), and also for piston, shulker and other
non-`SelfMovement` pushes (`MoverType.isServerAndClientSimulated`) on bodies the
client only observes. This hook receives the loop context, not a `Host`: it
decides only the movement delta, as a pure function of its inputs and what both
sides know that tick (`context.side()` tells the sides apart; the server keeps
vanilla's movement checks and its position wins). A `side = "server"` mod's
listener is not called for a body a client controls (a player, or a vehicle a
player is driving) and runs normally for every other body. `MoveCause`
distinguishes self movement, player packets, pistons, shulker boxes, shulkers
and a moving collider carrying or shoving a body (`Carried`); boats and
minecarts use self movement.

A replacement must have finite components; the hook imposes no per-axis magnitude
limit. Each invalid proposal is reported with its mod and discarded whole, including
its cancellation, without disabling the listener. Later listeners see the preceding
accepted delta. An unchanged game delta is not revalidated. Cancellation
skips the invocation: no position, velocity, collision-flag or fall
change, and no displacement notice. An overridden player or vehicle packet move
sends a correction even below the normal distance threshold, for creative players
and from obstructed starts too. A replacement equal to the request retains the
normal vanilla distance rules.

If a listener panics or incurs a phase refusal, its proposed delta and any
change to the cancel flag are discarded. Later eligible listeners see the last
accepted result; a failed callback neither vetoes movement nor clears an earlier
veto. Only a panic disables that mod's callback for this hook for the session.

A mod moves an entity through the same moment with `EntityRef::move_by(host, delta, cause)`
(`Entity.move(cause, delta)`, server only): the collision scan stops it at blocks, the world border and
entities, it steps up, and its ground flag follows. This is not a teleport or a velocity setter, but
collision response can change the body's velocity through block restitution. If the base fall handling
removes the body, no restitution query runs. The `entity_move` hook fires once with `cause`, and a
cancelled move returns `[0.0; 3]`. See the generated reference for which bodies the handle can move
and for the relative-teleport semantics of player moves.

`EntityRef::on_block(host)` uses the block retained by the last support check for players, mobs,
armor stands and nonliving bodies that keep ground support, including items, boats and minecarts.
It does not rerun collision detection. If there is no retained support, it reads the block at
vanilla's feet offset.

`living_travel` runs on both peers (movement phase 2, `LivingEntity.travel`
branch pick), after the player's swim steering or at a mob's travel entry: the
client's own movement fires it too, so a mod installed on both sides predicts
the edit or veto the server applies, and the server keeps vanilla's movement
checks and its position wins. A `side = "server"` mod's listener is not called
for a body a client controls (a player, or a vehicle a player is driving) and
runs normally for every other body. It receives the loop context, not a `Host`,
and lets a listener change the input and velocity or cancel. `TravelMode` names
the branch about to run, including species overrides: an allay or vex reports
`Flying`, and a squid reports `VelocityOnly` in or out of water. A replacement
does not change the branch. A body that does not travel, such as a shulker,
fires no hook. Simulated (replayed) player ticks fire the hook as real ticks do.

Edits to `input`, edits to `velocity` and cancellation are independent: there is
no per-listener output validator or synthetic magnitude cap. Velocity writes
follow vanilla `Entity.setDeltaMovement`: any finite vector is applied, while a
non-finite vector leaves the previous velocity intact without reverting valid
input edits or cancellation. Input flows through vanilla `LivingEntity.travel`
(`Entity.moveRelative` and `Entity.getInputVector`): vectors with squared length
above `1.0` are normalized, and a non-finite input adds no acceleration without
reverting valid velocity edits or cancellation.

Cancellation skips the branch, its nested moves and flying vertical damping,
without undoing earlier work such as swim steering. Listeners registered with
`#[listener(receive_cancelled)]` still run and may set `cancelled` back to
`false`. If the branch runs and calls `entity_move`, that move can still be
vetoed. A listener panic or phase refusal leaves earlier edits standing and
later eligible listeners running. Only a panic disables the failed callback for
this hook for the session.

The server does not send a separate position correction for a listener's edit or
clamp later movement reports on a veto: vanilla's ordinary movement checks
decide.

`living_travel_moved` is the moment after the branch's own move (movement phase 4):
the collision has happened and the branch's drag and gravity have not. A listener
replaces `velocity` with a finite vector and the drag and gravity then act on it;
`moved`, `on_ground_before`, `on_ground` and `jumping` say what the move did. It
fires once per branch that moves, with a zero `moved` when `entity_move` vetoed the
move, takes the loop context and no `Host`, and runs on every side that simulates the
body. A `side = "server"` mod's listener is not called for a body a client controls.

On an observing client, `EntityRef::interpolate_to(host, position, rotation)`
updates client display state for a body the server owns (`Entity.moveOrInterpolateTo`),
routing to the family's own interpolation handler or setting the pose directly
for `NO_OP` families; the next server update wins, and bodies the client
simulates are refused (`false`).

## Turning hanging entities

On the server, use these `EntityRef` methods with `&mut dyn ServerHost`:

- `set_hanging_direction(host, direction)` changes a painting or frame's facing as
  `HangingEntity.setDirection` does, updating yaw, pitch and bounds. Frames accept all six
  directions; paintings accept only the four horizontal directions and return `false`
  without changing for up or down. The next survival check decides whether it can hang there.
- `set_item_frame_rotation(host, rotation)` turns the item in an item frame or glow item
  frame as `ItemFrame.setRotation` does: eighths of a turn, wrapping modulo 8.
  Comparators update afterwards.

Both methods return `false` for a stale handle or the wrong entity type. Entity
synchronization shows the change to players, including a painting or frame turned south.

## Swimming eligibility

`Hooks::can_start_swimming` asks for a Boolean from the local player and the
server's acting player. Return `Some(false)` to refuse or `Some(true)` to allow;
the first answer wins. Return `None` to leave vanilla's answer in place. The
player and `LogicalSide` are in the event, and the shared `Host` is read-only.
Declare `side = "both"` and install the mod on both sides. Base the answer on facts
available to both sides that tick; the server remains authoritative for position.
For example, read the feet slot with `host.living(event.player.entity())` and
return `Some(false)` on `SwimmingCheck::Entry` while a certain pair of boots is
worn. Return `None` for callers your rule does not affect.

`Entry` runs after fluid sampling for a sprinting player who isn't swimming or
flying. The query runs before the passenger check: allowing it still cannot
start swimming while riding. `ShallowWater` runs in local sprint eligibility
when vanilla would not treat the water as shallow but the body is still in a
swimmable fluid. In that case denying the predicate counts as shallow and can
prevent or stop sprinting. An existing swim does not ask the entry question.

## Teleport hooks

`ender_pearl_teleport` covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`, and read what the pearl hit
(`hit_entity`, or `hit_block` and `hit_face`, and the point `hit_at`). Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision. A consumed stack the API can't carry, such as a saved item
whose id isn't registered, isn't heard: the listeners aren't called and the teleport
goes on as vanilla's.

## Fishing medium

`fishing_medium` lets a mod say that a bobber outside water floats and fishes where it is: lava,
the void under an island, a cloud. The first mod in order to answer decides; later mods are not
asked. The answer is a `FishingMedium` whose `surface` is the world Y the bobber floats at. The
bobber is in the medium in the block of `surface` or below (its y under `floor(surface) + 1`),
so a surface of -64.0 holds y -63.5 but not y -62.5, and negative heights work the same as
positive ones. A `surface` that is not finite is no medium.

On the server the answer also carries a `loot_table` and `lure_ticks`. The catch rolls the named
table, and the wait for a fish is `min + nextInt(max - min + 1)` less the rod's Lure, with `min`
raised to at least 1 and `max` to at least `min`. A table id that is malformed or names no table
the server has rolls vanilla's `minecraft:gameplay/fishing`, with one warning naming the mod and
the id. A client reads only the `surface`: it draws the bobber floating and ignores the table and
the wait.

#### Item use lifecycle

`living_entity_use_item_tick` runs after the may-still-use check. Its `duration`
replaces the remaining-ticks counter. Next come per-tick effects only above zero,
the decrement, and server completion at zero or below.

On the server, canceling Tick or setting duration to zero or below **finishes**
the use: food is eaten or a potion applied, Finish fires, and use state clears.
Setting one runs the effects and finishes in that same tick. This never calls
release-using, so a bow fires nothing. A use-on-release item such as a crossbow
does not finish here: its counter goes negative until release.

On a client, cancellation sets the counter to -1 before decrement and skips effects. The negative countdown continues;
the use remains active until server completion or release, so releasing still
notifies the server.

The neighbouring events differ: canceling Start prevents the use beginning,
but duration zero does begin it. Canceling Stop skips only release-using, not
clearing the use. Finish is not cancellable; its result stack can be replaced.

## Entity size on both sides

`entity_size` resolves dimensions through a read-only `Host`, on each side that
refreshes an entity's size. A client asks for its copy of a server entity when the
copy is added, when the server sends a new pose (even one with the same size), and
when `minecraft:scale` or another synched value from the server changes its vanilla
size; the copy's box, picking and hitbox outline then use the answer.
The server asks for its own entity separately: a mob before and after its tick,
and every other entity (items, minecarts, boats, projectiles, armor stands, displays,
interactions and the rest) at the same points, the first time once it exists. Its
collisions, block contacts and picking then use the answer. Answer from the event
(the entity's type, the pose and the sizes) the same way on both sides, or the
client's box and the server's disagree.

## Player size and pose fit

`entity_size` resolves dimensions through a read-only `Host`.
Before player motion, standing, crouching and swimming candidates are resolved;
an unchanged current-pose answer is reused. The fit test and pose application share
the resolved answer. Answer using the event's pose, not the entity's current pose:
a candidate need not become the actual pose. With subscribers, candidate resolution
dispatches 2–3 times per player tick; keep the listener cheap.

Cached answers are keyed by pose and vanilla dimensions. A scale change after
resolution or a replay with another key uses vanilla for that fit decision until
the next pre-motion resolution. Uncached fall-flying, sleeping and spin-attack
candidates also use vanilla dimensions.

## Living knockback entry

`living_knockback` fires once per full-entry call, before resistance and motion changes.
The read-only `entry` is `KnockbackEntry::Short` when the short overload delegated the push,
otherwise `KnockbackEntry::Full`. To model doubling strength at both entries, double it
once, then double it again only on `Short`: an input of 0.4 becomes 1.6 for `Short`
and 0.8 for `Full`. There is no second hook dispatch. Two non-commuting listeners run
in dispatch order, each completing its rewrites before the next; this does not simulate
two separate passes over the listener list. Cancellation still leaves motion unchanged.

On the server, `living.knockback(host, strength, [x, z])` enters the short overload. The
entity moves away from `[x, z]`, and a player's own client gets the resulting velocity.
It returns `false` if the handle is unavailable, the input isn't finite, the hook cancels,
or no push remains after resistance. This call deals no damage and adds no kill credit:
its source is unattributed `minecraft:generic`, so any attacker fallback and kill credit
come only from the victim's existing hurt memory.

`kill_credit` identifies who receives kill credit for the pushed living entity, and `kill_credit_main`
is that credited entity's current main-hand stack (if any), both read after accepted-hit memories are
committed and before the push. A remembered player takes precedence over the last hurting mob,
including a tame wolf's owner. An unresolved remembered player yields 0 and no stack, not a fallback
to the mob. After the remembered player expires, the recent mob may receive credit. This context is
distinct from `attacker` and its captured source weapon, and the credited entity can differ from both
the victim and the immediate attacker. NeoForge's `LivingKnockBackEvent` doesn't carry either field.

## Living death

`living_death` (see its rustdoc and [Hook notes](../../reference/hook-notes.md)) supplies
the accepted killing source and its captured weapon stack. Cancellation keeps the
health a listener leaves, suppresses loot, experience and death messages, and does
not heal the body. An unhealed cancelled body is removed at tick 20 without loot
or another death dispatch.

`LivingRef::kill_credit` can still resolve a mob's remembered player after the
lethal hit while the mob remains in the level. As in `LivingEntity.baseTick`,
the player reference survives the tick that decrements its timer to zero and
expires on the next tick. Death loot and experience still use the positive timer
to determine whether a player hit the mob recently.

### Last attacker and last hurt target

On the server, `LivingRef::last_attacker(host)` reads who last hurt the entity,
within its recent-attacker window. `LivingRef::last_hurt_mob(host)` instead
reads the living target the entity last attacked, as `(EntityRef, i32)`. The
number is the holder's `tick_count` at the hit, not level time. Melee records
a successful hit; arrow and wind-charge impacts record the target before
damage, even if damage is refused. There is no expiry window on this read.
It returns `None` for a stale holder, an empty record, or a dead or gone target.

## Item pickup notices

Listen to the server hook `item_picked_up_post` (NeoForge's `ItemEntityPickupEvent.Post`) to track
items a player receives from the ground. It fires once per pickup that put anything into the
inventory: when the whole stack went in, and when only part of it did and the rest stayed on the
ground, as when 8 apples lie there and only 5 fit.

It does not fire when `item_picked_up` refused the pickup or the inventory took nothing. A creative
player always takes the whole stack.

It carries the player's UUID, the item entity, `original`, a copy of the stack as it lay, and
`remaining`, the item entity's live remainder: `None` after a whole pickup, the part that did not
fit after a partial one. When the notice arrives, the amount the inventory took is `original`'s
count less `remaining`'s. Set `remaining` to change what stays on the ground, or `None` to empty the
entity; that changes the ground, not what went into the inventory, so read the amount before editing
`remaining`. Removing the entity in the listener keeps it removed, for the host's player and LAN
guests alike: there is no take animation, the stat and criterion run.

For a quest counter or custom pickup sound, count the amount as it stood when the notice arrived:
after any listener edits `remaining`, `original` less `remaining` need not equal what the inventory
took.

A client of a server never runs the server's pickup hooks. There, listen to the client hook
`client_take_item_entity` (vanilla's `ClientPacketListener.handleTakeItemEntity`): the server told
this client that `collector` took `amount` of `collected`, an item, orb, arrow or trident the client
tracks. It runs before the client shrinks or removes `collected`, so the host still reads it.
`collector` is the local player's entity for its own pickups, another player or a mob for theirs,
and None when the client doesn't track the collector.
