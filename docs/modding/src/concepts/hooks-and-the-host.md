# Hooks and the Host

A **hook** is a moment in the game, such as a tick, a block break or a player joining. Implement
that moment on `mod_api::Hooks` to receive its **event** and, where the signature provides one,
a **host**: the game's typed interface for that call. Use a [Part](parts.md) instead when the
behaviour belongs to particular registered objects rather than all matching objects.

![The game calls every mod that implements a hook, and a Part only for its own objects. Both receive an event and
reach the game through the Host, its handles, capabilities and attachments.](../images/hooks-parts.svg)

## Dispatch

| Hook kind | What the listener supplies |
| --- | --- |
| Listen | A reaction to a notice; every subscribed mod is called. |
| Cancellable | `Flow::Cancel` stops the action; on hooks whose contract permits it, a later `receive_cancelled` listener can clear cancellation (`entity_move` and `living_travel` keep an accepted veto sticky). |
| Replacement | The first accepted `Some` wins; `None` passes to the next listener. |
| Collection | Each listener can contribute to the event's list. |

Read the signature before choosing a hook: it tells you which side runs it, what you can change,
and whether you are observing a decision or participating in it. For exact signatures and return
rules, see the [generated API reference](../reference/api.md); [Hook notes](../reference/hook-notes.md)
explains how related hooks fit together. For a concrete tour, see [Entity hooks](../how-to/entities/entity-hooks.md).

## Failures and panics

A panic disables only that mod's callback for that hook for the session. The game names the mod
and hook in the log and, on a client, in a [diagnostic toast](../how-to/client/toast-notifications.md#admission-rules)
that mods cannot cancel or rewrite. Ordinary refusals are reported without disabling the callback.

A failed proposal is not a cancellation or replacement. Later eligible listeners see the last
accepted event state, not the original request. [Staged registration](load-phases.md#registration-and-setup)
discards the failed callback's declarations. On a journaled action its failed writes do not join
the action; this is not a rollback of arbitrary external or mod-owned side effects. A failed
[chunk-disk job](../how-to/world/chunk-data.md) cannot publish that job's edits, while the other
listeners' accepted edits still can. An attachment codec refusal rejects only that operation and
does not disable the listener; an error encoding or writing the disk record itself is separate,
and can keep that record from being saved.

## Listener priority and cancellation

`#[listener(priority = 100)]` runs before default-priority listeners. Higher `i32` priorities run
first; the default is zero and ties keep dependency order. This order is fixed at installation.

Generated declaration order only orders the generated `Hooks` items and local `HookId` discriminants; it does not
control which mod runs first.

A listener normally skips an already-cancelled event. Add `receive_cancelled` to inspect it; on
hooks whose contract permits clearing cancellation, setting `event.cancelled = false` and returning
`Flow::Continue` lets later listeners and the vanilla action proceed. Some hooks keep an accepted
veto sticky (`entity_move` and `living_travel`) and preserve the earlier veto even if a later
listener clears the field. `receive_cancelled` is only valid on cancellable hooks. These attributes
order global hook listeners, not Parts; see [Parts](parts.md) for their dispatch rules.

Cancellation controls the current action, not all its side effects. For example, cancelling a
projectile impact suppresses the hit without itself discarding the projectile. If a listener
also discards the projectile, a later listener clearing cancellation does not bring it back.

## Events and shared state

The game constructs the generated event and lends it to each listener. The public fields describe
what you may read or edit; the struct is `#[non_exhaustive]`, so use `..` when destructuring. Mutable
scalar fields are written back after dispatch, even after cancellation; mutable lists and other
compound fields are borrowed. Later listeners see accepted edits. When an edited stack-list output
is refused, that field keeps its preceding accepted value; valid scalar edits and cancellation still apply.
 A refused replacement does not win a first-answer hook.

Some events lend the live subject instead of handing out a snapshot. Borrow the view only while
using it; a later borrow can fail if the subject is no longer resident. For journaled writes through
both the subject and the host, use the scope's [transaction](transactions.md). For example, a damage
container belongs to one hit: its Post runs after that hit resolves, before the caller resumes. Copy
facts you need later rather than trying to keep the view. To exercise dispatch, see [Testing](testing.md).

Hook and Part methods use `&self` and require `Send + Sync`. The same mod instance serves both
logical sides in single player. A host write may also call another hook of the same mod before it
returns. Release your locks before calling the host: otherwise a re-entrant listener can deadlock.
Use an [attachment](attachments.md) for data that belongs to one game object.

## Weather parameters and strengths

`LevelRef::set_weather(host, clear_ticks, weather_ticks, raining, thundering)` takes signed
timers. Any resident server level can name the origin, even when parked, but the write applies
to the whole server. `Hooks::weather_change` hears it once, just as it hears an explicit
`/weather` write or a GameTest weather parameter write. GameTest saves the batch level's
actual weather type before setup and proposes that type again on teardown. Version 2 of
`WeatherChangeEvent` includes the origin dimension, both requested tick values and both
flags. Each mod gets its own copy of those inputs.

Return `Flow::Cancel` or set `event.cancelled` to veto the whole proposal. The setter returns
`false` on a veto or an invalid handle; `/weather` reports failure on a veto. Other Host writes
from the listener still stand. A nested weather write can proceed without calling listeners
already handling `weather_change` on that thread.

For an immediate change in strength instead, use `LevelRef::set_weather_levels(host, rain, thunder)`
on the addressed resident level, including a parked one. Both interpolants clamp to the 0–1
interval, including infinities; NaN is refused. Strengths belong to each level. Neither strength
changes nor the natural weather cycle fires `weather_change`.

## Which thread runs a hook

Most hooks run on the thread that owns their side: the server thread or the client window's thread.
`thread_local!` is thread-local, not world-local or mod-local. Consult the hook or Part's docs for
exceptions, including these:

- **World generation:** placement and sampling can run concurrently on worker threads; some
  placement also runs on the server thread. These calls get a `WorldgenHost` or no host, not a live
  `Host`. Generation must not depend on which worker reaches your code first.
- **Rendering:** startup can register meshing and tint callbacks that later run on workers without
  a host. Answer from the inputs and immutable registration data, not mutable render state.
- **Chunk-disk callbacks:** `chunk_data_load` and `chunk_data_save` run on the owning server
  thread. `host.job()` borrows the captured `ChunkDiskContext`, and the restricted host edits only
  its job-local [attachment store](../how-to/world/chunk-data.md#saving-chunk_data_save), never the live chunk.
- **Loading and chunk I/O:** data callbacks may run on a loader thread. See
  [Load phases and access](load-phases.md).

For ordinary shared state, use synchronized mod fields or publish immutable snapshots. A lock
makes access safe; it does not make an order-dependent algorithm deterministic.

## Level hosts

`Host` exposes services and handles. A handle identifies a live object; using it still requires the
right host and can fail if its target has gone away. A `ServerHost` can make authoritative writes;
a `ClientHost` reads the client's own state and provides client services. Match `HostSide` when a
hook can run on either side. A host can exist even when its world does not.

A restricted level hook lends only the access its signature needs. `LevelHost` is a read view of the
owning level. `LevelRandomHost` adds a loan of the live random stream; `LevelTicksHost` provides
scheduling, and `LevelWriteHost` permits local writes and effects. `HitHost` lends hit-scoped
operations, not block writes; at `living_incoming_damage`, `living_damage_pre` and
`living_damage_post` it also lends read-only `LivingView`s of the victim and the causing entity
(`host.victim()`, `host.causing()`), whose attachments it reads but never writes. The causing
entity is the one the hit's source names, where the game has it at hand: a melee attacker, Thorns'
wearer, a projectile's shooter or owner, an explosion's igniter, a stinging pufferfish, or a
parked guest read from its hub record (read-only, no random). A victim that caused its own hit is
read through `victim()`; `causing()` answers `None` then. Random draws
from a live stream are not undone by an abort; see [Randomness](randomness.md#borrow-the-live-level-stream).

Hooks that fire inside a tick's own health or movement update take a loop context instead of a host,
and lend the same read-only `LivingView` of the entity they are about as a parameter: `living_heal`
its healed entity (`healed`), `living_jump_power`, `living_speed` and `living_flying_speed` the
moving entity (`living`). The view reads the entity's live state at the call, so an effect changed
earlier in the same tick is what the listener sees, on the server and in the client's prediction.
With no host there, it reads no attachments.

Reads such as collision, light, signals and block entities use the owning side's available data.
They do not load chunks. Light or a vanilla air result is not proof of residency; check `is_loaded`
when you need a resident position. Treat `None` as unavailable where the reference says a view
cannot answer. A client's placement or shape query must not treat unavailable server-only data
as a server answer. Read-only slot reads (`LevelHost::with_container_stack`, which lends an
`ItemStackView`) don't open pending loot; an engine comparator read through a `LevelRandomHost` can. For concrete examples see
[Camera and collision reads](../how-to/client/camera-and-collision-reads.md),
[Block-state facts](../how-to/world/block-state-facts.md) and [Capabilities](capabilities.md).

Use a [transaction](transactions.md) for related journaled writes, passing the host and context
from the same closure. To save from a hook, request a save at the next settled tick boundary
rather than saving synchronously while a subject can be borrowed. Use the supplied draw context
for drawing: it is scoped to the render call, not a way to reach the live renderer. See
[Model drawing](../how-to/client/model-drawing.md).

## Reading resident collections

A `visit_*` read lends resident entries to a callback instead of building a list.
Return `ControlFlow::Continue(())` to keep reading or `ControlFlow::Break(())` to stop.
A completed visit may have had no entries; a typed refusal means the read could not
run at all. Copy a borrowed name or row field only if it needs to outlive the callback.
You can make other shared reads inside the visitor. To write through the same host,
retain only the handles or values you need, then write after the visit returns.
Single-value reads such as `level.block_property(host, pos, key)` lend the result
directly: the property value borrows the host, not the lookup key.

## After a hook that can change the world

A listener can remove, move or change objects before the engine applies an action. For decisions
that must still hold, the engine re-resolves the target by identity and checks the facts it used; if
those facts changed, it abandons the decision rather than recomputing it. The hook's own documentation
names the facts it checks. Do not retain a list position or assume a handle remains live across a call
that can dispatch another hook.
