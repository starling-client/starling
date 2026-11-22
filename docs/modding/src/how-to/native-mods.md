# Native mods: building and shipping

A native mod is Rust source in a `.smod`. The launcher compiles it on the player's machine, with its own pinned Rust
toolchain, against the prebuilt game of the exact Sunrise build the mod pins in `[native]`. The game itself is never
rebuilt for a mod: the mods are compiled and linked against that prebuilt game, and the instance then plays the
result.

Native code runs with the player's rights, outside any sandbox. The first install shows a trust notice, and the
launcher builds native mods only on an install that allows native code (`--trust-native-code`, revocable in
**Settings > Storage**).

The pages in this section follow a mod from its source to a player's game and a server:

<!-- section-pages -->

## Living knockback entry

`living_knockback` fires once per full-entry call, before resistance and motion changes.
The read-only `entry` is `KnockbackEntry::Short` when the short overload delegated the push,
otherwise `KnockbackEntry::Full`. To model doubling strength at both entries, double it
once, then double it again only on `Short`: an input of 0.4 becomes 1.6 for `Short`
and 0.8 for `Full`. There is no second hook dispatch. Two non-commuting listeners run
in dispatch order, each completing its rewrites before the next; this does not simulate
two separate passes over the listener list. Cancellation still leaves motion unchanged.

Version 2 also supplies `kill_credit` and `kill_credit_main`: the pushed living entity's
kill credit and that entity's current main-hand stack, read after accepted-hit memories
are committed and before the push. A remembered player takes precedence over the last
hurting mob, including a tame wolf's owner. An unresolved remembered player yields 0 and
no stack, not a fallback to the mob. After the remembered player expires, the recent mob
may receive credit. This context is distinct from `attacker` and its captured source weapon. Both fields are additions: NeoForge's
`LivingKnockBackEvent` doesn't carry them.

## Living death contract version 2

[`living_death` version 2](https://github.com/sunrise-org/sunrise/blob/main/docs/native-mods.md#living-death-version-2) supplies
the accepted killing source and its captured weapon stack. Cancellation keeps the
health a listener leaves, suppresses loot, experience and death messages, and does
not heal the body. An unhealed cancelled body is removed at tick 20 without loot
or another death dispatch.

`LivingRef::kill_credit` can still resolve a mob's remembered player after the
lethal hit while the mob remains in the level. As in `LivingEntity.baseTick`,
the player reference survives the tick that decrements its timer to zero and
expires on the next tick. Death loot and experience still use the positive timer
to determine whether a player hit the mob recently.

### Custom statistics

Declare custom statistic identities in `Hooks::register`, using `Registrar`:

```rust,ignore
fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
    registrar.register_custom_stat("steps").unwrap();
}
```

For a mod named `rewrite`, this declares `rewrite:steps` in
`minecraft:custom_stat`. An explicit `rewrite:steps`, another manifest-owned
namespace, or a namespace delegated to the mod through `served_by` (see
[Declare content in code](content-mods.md#declare-content-in-code)) is also accepted; `minecraft`, foreign namespaces, malformed IDs and
duplicates are refused. Ignoring the returned `Result<(), DeclarationError>` does not
bypass installation failure. A mod's explicit registration refusal rolls back
its declarations, and the installed inventory freezes before worlds open.

These declarations populate the integrated command/statistic inventory and
append after vanilla's IDs in LAN and client built-in registry reads, preserving
vanilla IDs. Servers declaring custom statistics require Sunrise clients with
the same ordered statistic inventory; configuration rejects mismatches before
play. Register the target before rewriting an award to it; an arbitrary
identifier alone does not register a statistic.

`/scoreboard objectives add steps minecraft.custom:rewrite.steps` creates an
objective for this registered counter.

## Moving colliders

A native mod gives an entity boxes that every other entity collides with:

```rust
entity.set_collider(host, Some(Collider {
    boxes: vec![ColliderBox { min: [-1.5, -1.0, -10.0], max: [1.5, 0.0, 10.0] }],
    rotation: [0.0, 0.0, 0.0, 1.0],
}))?;
entity.set_collider_transform(host, turned)?; // the rotation alone, as the entity turns
```

The boxes are in the entity's own frame (its position is the origin), turned by `rotation`, a unit
quaternion with any yaw, pitch and roll. Other entities collide with them as solid ground and walls:
the entity's box is swept against each turned box (a separating-axis test with the time of impact), it
steps up onto a top within its step height, and standing on a tilted top follows the surface. The
collider is solid to every mover except spectators, entities with no physics and the entities in its
own vehicle stack (the root vehicle and every passenger, at any depth, the collider's entity among them).
**Contact effects are not applied yet:** a box is solid ground and walls and nothing more, so standing or walking on one has no block's step sound, sprint particles, friction or bounce ([#1909](https://github.com/sunrise-org/sunrise/issues/1909)).

The game checks a collider when it is set and refuses by name (`ColliderError`), keeping the old one:
every value finite, the rotation a unit quaternion within 1e-3, at most `Collider::MAX_BOXES` (32,768)
boxes, and every box within `Collider::MAX_REACH` (512) blocks of the entity on each axis. Boxes are kept
as `f32`; read back, they are the rounded values both sides collide with. A box whose minimum is above
its maximum is read with the corners swapped, and a box with no volume (zero thickness on an axis)
collides with nothing.

A collider is saved with the entity. The server sends it only to Sunrise clients that track the entity:
the whole collider when they start to (in messages of at most 64 KiB), then only what changed, by box
index, and the rotation alone for `set_collider_transform`. **A vanilla client does not see a collider**:
it sees the entity and walks through it. A mod that needs players to stand on one requires the Sunrise
client at join.

A shape edit does not immediately move a body it overlaps. On its movement tick, the Sunrise local
player's ordinary wall-escape nudge also checks solid collider boxes, except those in the same
vehicle stack. It checks the sampled pose, not the space a rotating box swept between poses.

### Carrying

A collider that moves carries what stands on it. Once a tick, after the level's entities ticked
(so whatever order they ticked in), every body standing on a box of a collider that moved or turned
this tick moves by the displacement of its contact point, the point of the box under its feet: the
translation and the rotation of the collider's change, as an ordinary collided `Entity.move` with the
cause `MoveCause::Carried`. An `entity_move` listener sees it and may cancel or replace it like any other
move. It stops at walls and steps up like any move, and yaw is never turned. A body's support is the box
under its feet with the highest top (a tie goes to the lower entity id). A collider standing on another
collider is carried first, and the bodies on it take its whole displacement. A body that rides something is not carried itself; it
follows its vehicle. The server carries the entities it simulates, and each Sunrise client its own player.

After carrying, a collider that translated without turning shoves bodies in the path of its boxes,
in its direction of travel. This also uses `MoveCause::Carried`. Changing a box's bounds does not
sweep that box between the old and new shape. Turning colliders still carry, but do not shove yet.
A shove does not cancel velocity into the collider; the ordinary entity move handles velocity when
the body hits an obstacle. The client predicts a shove for its own player, and ordinary server movement
checks still apply.

Neither carrying nor shoving pushes through a wall, and the collider is never stopped by a body in
its way. A squeezed body stays where the collision leaves it. A living body still takes `in_wall`
damage while a box covers its eyes, as in a block. The shove itself does not deal impact damage.

### Players on a moving collider

A client moves its player with the collider as it last heard of it, so the position it reports is in a
frame that is some ticks old: the round trip, plus how stale its copy was. The server therefore keeps
each collider's recent transforms (about a second of them: about 2.8 KiB of frame data per collider that
moves, plus the container and index around it, and one entry for one that stands still) and judges a report against the collider as it was at some age
around the one it expects, the player's round-trip time plus two ticks, give or take four. A report
fits an age when the player stands on the top of a box (within 0.05 blocks) or is above one, is over
it, and is not inside any box deeper than that. Of the ages that fit, the one nearest the expected age
wins. The move from the last accepted position, measured in the collider's frame (so the speed test sees how far the
player walked on the collider, not how far the collider carried it), then goes through the
ordinary move check of a walking player, with the colliders placed where they stood at that age, so
walls, blocks beside the track and the collider itself stop a carried player as they stop any other,
and a player already touching one box of a collider may not walk into another box it did not touch.
The position the server keeps for an accepted report is the same place on the collider under its
transform now, not the position in the older frame the client reported in. A player on a collider's box
is held up by it, so standing on a collider is never counted as floating. A
Sunrise client also says how old its copy of the collider was before each report
(`sunrise:carry_age`); the server only starts its search there, and the report is still checked
against the same window.

A report that fits no age in the window is not carried: if the player was on the collider, the ordinary
rules judge the move, and if they refuse it the player is sent back to where the server last had it
on the collider, placed under the collider's transform now. Such a correction is counted per player and
never counts toward the floating-player kick. A teleport (a portal, a command) and a collider
that is gone let go of the player.

What it holds for: Create's speeds, 1.4 blocks a tick on straight track (2.0 powered), and 0.7 and 1.0
on a curve of 4 degrees a tick, at round trips from 50 to 200 ms and a tick of jitter either way, never
refuse an honest player, and the daily tests run curves up to 2.0 blocks a tick the same way. **The
limit is the age:** the window is four ticks either side of the age the server expects, and on a curve a
report held to a wrong age moves the far end of a long collider sideways, so a round-trip estimate that
is off by more than the window, or a client that is more than four ticks stale, is refused. A collider
that goes faster or turns harder than 2.0 blocks and 4 degrees a tick is not covered by any test.
Past a second of round trip the window stops growing: a player whose copy is
older than it reaches is judged as a walker in the frame the cap allows, not carried. A vanilla client, which sends no age, gets the
window around the expected age.

## What a callback can access

Registration, setup, initial config loading and data/asset reloads lend only the
context in the hook signature, not a full game host. An operation that is not on
the context cannot be called there. Config reload and unload lend the polling
or stopping game's host; world and session lookups on a live host return
`None` when no world or session is open. See [load callbacks](../concepts/load-phases.md)
for the callback order and types.

| Phase | Open |
|---|---|
| Setup | settings reads; registry and metadata reads; the client setup calls; the published resource queries |
| Config | settings reads; the mod's own config files |
| Reloading | settings reads; registry and metadata reads; the published resource queries |

The client setup calls of Setup can have presentation effects (a sound, a narrator line, a HUD mode) that stand
even if the hook is refused afterwards. A native mod that calls a method in a phase that does not admit it is
**refused, not crashed**:

- The call ends the mod's hook call (it unwinds out of the listener; the game makes no panic report). The game logs
  `native refusal: <mod> hook <hook>: mod API <method> is unavailable during <phase>` on each refused callback.
- The latch: after one refusal, every later Host call that hook call makes is refused too, whatever it asks, and the
  refusal counts even if the mod caught the unwind itself and returned normally.
- The listener is dropped, and the hook goes on with the next one. The event outputs the listener touched are put
  back to what they were before its call, and the answers of earlier listeners stand. What its earlier allowed calls
  did (a sound, a narrator line, a HUD mode) stands too.
- The hook uses its default return value: `Flow::Continue` for a cancel hook, `None` for a replace
  hook that answers an `Option`, and the type's default (`false`, an empty list, `()`) for any other. A
  refused listener is not disabled for the session; a panicking listener is.
- A method with a typed phase refusal (for example, one returning `Result<_, LevelReadError>`,
  `Result<_, LevelWriteError>`, or `Result<_, TeamMembersReadError>`) returns its `Err(Unavailable)`
  instead of unwinding, and the mod goes on.
  `LevelRef::with_random` returns `None` without calling your closure when world access is unavailable.

A panic is logged and shown in a client error toast naming the mod and hook. Only that callback is
disabled for the session. Its staged outputs are discarded, and writes covered by an existing
transaction are undone. Effects outside a transaction, such as a dynamic texture already created
by `client_tick_pre`, are not undone.

**The limit:** if the thread is already unwinding when the call is refused (a mod's `Drop` that calls the Host while
another panic unwinds), the game logs the same line and panics, and a panic during an unwind aborts the process.
Do not call the Host from a `Drop`. A native mod built with `panic = "abort"`, a library that brings its own
`#[global_allocator]`, and a library that does not use the game's `libstd` are refused at load, by name.
