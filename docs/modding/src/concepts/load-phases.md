# Load phases and access

A hook's signature determines what it can access. Registration declares content against a startup
snapshot; reloads prepare and publish resource data; ordinary hooks use the live game. A reload
runs its own callbacks, not a new access mode for the rest of the game.

## Callback order

![Client startup order: discover packages, collect registrations, apply initial assets, run client setup,
load configs, then show the title.](../images/load-phases.svg)

Registration, initial config loading and reload-listener registration use dependency order.
Bundle order breaks ties; present optional dependencies count and cycles are refused. Content-only
packages follow code packages in the loaded-mod inventory. Use `ReloadOrder` for dependencies
between individual reload listeners.

| Callback | What it lends |
| --- | --- |
| Registration (`register`, `blocks`, `items`, and others) | `&RegistrationContext` for captured startup facts; declare changes through the registrar or event. |
| `client_setup` | `&mut dyn ClientSetupContext` for client setup and published assets/data, not world handles or transactions. |
| Registered config's `ConfigEvent::Loading` | Owner-bound `&dyn ConfigContext` for settings reads and the native config directory. |
| Registered config's `ConfigEvent::Reloading`, `ConfigEvent::Unloading` | `HostSide<'_>` for the polling game and `&mut dyn ServerHost` for the stopping server, respectively. |
| Data and asset reload callbacks | A reload or data-load context for the relevant resource generation, not a live `Host`. |
| Ordinary game callbacks | `ServerHost`, `ClientHost` or `HostSide`; world or session queries may still be unavailable. |

## Registration and setup

Declare keys, Parts and providers through the registrar or the event. Registration callbacks have
no live world. Their snapshot contains admitted packages, built-in registry names and captured
settings, not declarations added by earlier callbacks. A native mod switched off at startup still
registers, but its package is absent from the inventory. For package-scoped asset and data reads,
see [Data files](../how-to/native-mods/03-data-files.md#another-packages-files).

**A callback's pending declarations are staged.** They publish only after that callback succeeds.
A panic discards its pending entries or replacements and leaves earlier listeners' work intact.
Other listeners still run. Ordinary registration refusals follow the declaring API's contract;
they do not disable a callback. See [Failures and panics](hooks-and-the-host.md#failures-and-panics).

Factories or callbacks that run later cannot borrow a registration event or context. Capture
owned inputs in the closure. For example, a standalone model baker runs with its own bake context
on a resource load; see [Model drawing](../how-to/client/model-drawing.md).

The client runs `client_setup` once after applying initial assets, then loads Client and Common
configs before title, connection, input, render or tick callbacks. A dedicated server loads
Common configs before its first world. Server configs load before `server_about_to_start` and
unload after `server_stopped` for each world; opening another world does not repeat startup
registration or client setup. Use registration and the first server reload for common setup,
`server_about_to_start` for server setup, and declared dependencies for intermod ordering.
A panic in a live config callback does not undo settings writes that already took effect or
prevent server teardown.

For startup progress, open a bar through `RegistrationContext` and retain the handle if later
work needs to update it. A synchronous setup callback on the render thread cannot draw its own
progress while it runs. A worker or retained bar does not keep the loading screen open.

## Pack repository discovery

`add_pack_finders` lends a discovery context and a registrar for the client resource or server
data repository. It can run before the game exists, so only live rediscovery can lend a live
host. Text can use fallbacks before assets are ready. A panicking finder publishes none of its
pending packs and does not stop the others. See `PackDiscoveryContext` in the
[generated API reference](../reference/api.md) for the supported queries.

## Reloading

Prepare work is not published data. Read completed data and refresh caches after apply, not
from another listener's pending output. Initial client reload-listener registration has no
published client generation to inspect; later reloads can read the previous one. Completion
callbacks can read the newly published recipes, tags or client assets as their signature allows,
but cannot open a world handle or transaction.

The first client listener registration and prepare run on the loader thread. Later registration
runs on the render thread, prepare on a worker, and apply on the owning game thread. Some
asset declarations, such as fluid models and texture atlases, are collected again before their
assets are rebuilt. A failed registration callback publishes none of its pending listeners,
dependencies or retained entries; a cycle in the completed reload graph fails the reload.
See [Hook notes](../reference/hook-notes.md) and [Recipe caching](../how-to/items/reload-stable-recipes.md).

## Load contexts

`RegistrationContext` lends the startup inventory, built-in registry names, settings and
instance directory. Preserve native path bytes or code units when constructing a path. See
[FML loader APIs](../reference/fml-loader-apis.md) for the path and inventory queries.

`DataLoadContext`, `LoopContext` and `WorldgenHost` lend frozen metadata for their generation.
`mods()` gives the admitted packages in dispatch order; `registry(name)` and `setting(mod_id, key)`
borrow captured values without reading files. `None` means unavailable or unset, not a present
empty value. Reloads and live code switching do not change the startup package inventory. Keep
an owned copy only when it must outlive the callback. [Chunk-disk callbacks](../how-to/world/chunk-data.md)
borrow registry names from the generation retained by the worker.

`DataLoadContext::fluid_tags()` borrows the recipe pass's fluid-tag table. `None` means no
recipe overlay; `Some(&[])` means an overlay with no fluid tags. See
[Fluid ingredients](../how-to/items/ingredient-kinds.md#fluid-ingredients) for how to use it.
`DataLoadOrigin` and `LoopSide` describe local execution, not an encoded payload.

### Ordinary Host reads

An ordinary host reads the current side's published registries, merged tags and live settings,
not the captured settings or pending reload data. A `RegistryEntry` carries canonical registry
and entry names; constructing one does not prove the entry exists. Narrow it through the host
when you need a real [typed key](registry-keys.md). On clients, generic registry reads see
built-ins and the connection's synchronized registries, not local copies of server-only data.
A host can serve these reads even when its world queries return `None`. See `KeysHost` in the
[generated API reference](../reference/api.md) for registry availability rules such as
`minecraft:worldgen/world_preset`.

## Settings snapshots

Startup readers share one immutable settings capture. It includes only readable, regular
UTF-8 files with UTF-8 names and follows no file or package-directory symlinks. Unreadable or
invalid entries are omitted individually and reported for that capture, not again on every read.
A captured value is not updated by a live settings write; use the ordinary host for live reads.

## Phase refusals and panics

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
