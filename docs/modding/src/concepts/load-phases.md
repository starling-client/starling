# Load phases and access

A hook's signature determines what it can access. Registration callbacks receive declarations and startup
facts, not a `Host`. An ordinary game hook receives its side's host, whether or not a world is open.

## Callback order

![Client startup order: discover packages, collect registrations, apply initial assets, run client setup,
load configs, then show the title.](../images/load-phases.svg)

Registration, config loading and reload-listener registration all use the installed dependency order.
The resolver keeps bundle-order ties, checks present optional dependencies and refuses cycles. Content-only
packages stay after code in the loaded-mod inventory; listener-specific edges use `ReloadOrder`.
Data or asset reloads run their own callbacks on first load or when resources change. They do not switch the
game into a different access mode.

| Callback | What it lends |
| --- | --- |
| Registration (`register`, `blocks`, `items`, and others) | `&RegistrationContext` for the mod list, built-in registry and captured settings. Write declarations through the hook's registrar or event. |
| `client_setup` | `&mut dyn ClientSetupContext` for client setup operations and published assets and data. No world handles or transactions. |
| Config registration and initial loading | `&dyn ConfigContext` for settings reads and mod-scoped `ConfigFiles`. |
| `config_reloading`, `config_unloading` | `HostSide<'_>` for the polling game and `&mut dyn ServerHost` for the stopping server, respectively. |
| Data and asset reload callbacks | The callback's `ReloadContext`, `ClientReloadListenerContext`, `ClientReloadHost` or `DataLoadContext`, as described below. |
| Ordinary game callbacks | `ServerHost` or `ClientHost` (or a `HostSide` selecting between them). World and session queries return `None` when no corresponding world or session is open. |

`RegistrationContext::instance_dir()` borrows the instance directory captured before
registration begins, as `Option<&InstanceDirectory>`: `Unix(Vec<u8>)` preserves raw Unix
path bytes, and `Windows(Vec<u16>)` preserves Windows UTF-16 code units, including
unpaired surrogates. Never convert Unix bytes with a lossy UTF-8 conversion when
constructing a filesystem path. A context with no engine snapshot returns `None`.

A mod that does slow work while the game starts shows it on the startup loading screen
(NeoForge's `StartupNotificationManager` and `ProgressMeter`). `RegistrationContext::progress(label,
steps)` opens a bar under the game's stage bar and returns `Result<StartupProgress, StartupProgressError>`.
It returns `TooManyBars` when that listener's mod already has 64 open bars. Opening bars from a scoped
worker with the same context counts toward the same quota. The returned bar is owned and `Send + Sync`:
call `step()`, `set(n)` and `label(text)` to update it, and drop it to remove it. A mod can keep the bar
after registration and update it or call `bar.message(text)` from a worker thread or from `client_setup`.

`RegistrationContext::message(text)` shows a line for a few seconds without opening a bar. Both
message methods and `bar.label(text)` borrow `text`; the caller need not keep it alive after the call.
The newest four bars and the last three lines are drawn; message lines are cut to one line
while bar labels are kept verbatim. A dedicated server
logs each label, line and progress in 10% increments instead.

This is part of the startup progress feature, not live rendering of synchronous setup work.
Bars can be opened through `RegistrationContext`; `client_setup` has no creation context and
must retain a handle from registration. Updates become visible only while the loading screen
is actually drawing. A synchronous `client_setup` listener runs on the render thread, so its
updates cannot draw live and may appear only after it returns, if another loading frame is drawn.
Slow work can run in a mod-owned worker and step a retained bar, but neither that worker nor
the bar keeps the loading screen open. Live display of synchronous setup work and a creation
route from `client_setup` remain outside this change.

## Registration and setup

Startup hooks such as `register`, `component_types`, `blocks`, `items` and `standalone_models`
receive no `Host` and read startup facts through `RegistrationContext`.
`register` receives `&mut dyn Registrar` for writing declarations. In `component_types`, call
`event.register("mymod:charge".into())` with each owned, namespaced ID. The game checks IDs and
duplicates before freezing the component registry.

Scoped registration events are borrowed only for the listener call. Pending entries are accepted
in dispatch order when the callback completes. A callback panic discards its pending entries,
disables that callback for the session and reports the mod in the log and error toast. Other
callbacks still run. Ordinary registration errors keep their own contract and never disable the
callback. A [HUD layer registrar](../how-to/client/hud-drawing.md#replacing-or-wrapping-a-layer)
can use pending anchors. It drops a listener's superseded callbacks before publication and retires
bodies from earlier listeners after publication; a retirement panic does not undo a replacement.

A callback registered on the event cannot borrow the event or the context. Capture owned inputs
for its `Send + Sync + 'static` closure. A model layer supplier runs once at the startup freeze;
a particle group factory runs once per particle engine, not on sprite reload. A standalone model
baker can run on a resource worker during each load, and borrows its bake context only for that
invocation. See [model drawing](../how-to/client/model-drawing.md) and
[particle render groups](../how-to/client/particles-and-movement-queries.md#particle-render-groups).

Not every registration-context hook runs only at startup. `register_fluid_models` and
`register_texture_atlases` collect declarations again on client resource reload before baking or
stitching, respectively. Atlas admission, duplicate handling and dispatch timing are unchanged.

The context's `mods()`, `registry(name)` and `setting(mod_id, key)` borrow
one view captured before the first of them runs, so every registration step sees the same values
whatever the dispatch order: the enabled, admitted packages (`&LoadedMods`), the built-in registries as
`registries.json` lists them (`Option<&[String]>`, without what mods declare during registration),
and the settings on disk when the snapshot is captured (`Option<&str>`). An unknown registry,
an unset key, or a context with no engine snapshot returns `None`.

A native mod switched off at startup still registers, but its package is absent from the snapshot.
`package_files(id)` lists and reads the admitted `assets/` and `data/` files for the packages in that view
(see [Data files](../how-to/native-mods/03-data-files.md#another-packages-files)).

`client_setup` runs once after initial assets are applied, with `&mut dyn ClientSetupContext` and
`&mut ClientSetupEvent`. Afterward the window loads its Client and Common configs, before title,
connection, input, render or ordinary tick callbacks. If pack selection fails during initialization,
the recovery options screen opens only after config loading. A title-screen listener may receive a
`ClientHost`; world-handle queries then return `None` because no world is open. A dedicated server
loads Common configs once before its first world. Server configs load before `server_about_to_start`
and unload after `server_stopping` for each world. A second world does not repeat startup registration
or client setup.

Attribute bounds are registration data too. `Registrar::widen_attribute_bounds` asks, from a both-side package's
`register`, join the attribute catalog once when it grows: after the attributes mods declare with
`declare_attribute`, and before entity attribute rows resolve and anything reads a definition. The bounds become
the union of every mod's asks, so the result does not depend on the dispatch order, and the default never moves
([content mods](../how-to/content-mods.md) has the call, its refusals and what saves keep).

During `config_reloading` and `config_unloading`, a callback panic disables only that callback for
that mod and reports its name. Subsequent listeners and server teardown still run. A live setting
write that already took effect is not part of a registration batch and is not undone. In startup
registration, the registrar checkpoint or declaration batch discards the panicking listener’s
pending entries; later mods can still register.

## Pack repository discovery

`add_pack_finders` takes `&mut dyn PackDiscoveryContext` and `&mut AddPackFindersEvent` at startup
and on rediscovery. Register through `event.packs`; use `event.packs.pack_type()` to distinguish
client resource and server data repositories. `context.text_string(&text)` resolves local text,
using fallbacks before assets are loaded. `context.client_text()` lends font measurements only
when the client is ready. `context.live_host()` returns `None` before the game exists, and lends
a `HostSide<'_>` during live rediscovery. For example, match `Some(HostSide::Client(host))` to read
a registered client setting via `host.setting(mod_id, key)` while rediscovering resource packs.
If a callback panics, none of its offered packs are kept, and other finders still run.

## Reloading

Data and asset reload hooks lend their own contexts instead of a full `Host`:
`add_server_reload_listeners`, `tags_updated`, `data_maps_updated` and `default_components_bound`
receive `&dyn ReloadContext`, `add_client_reload_listeners` receives
`&dyn ClientReloadListenerContext` (whose `published()` method returns `None` on initial startup
before any resources are published and `Some(&dyn ClientReloadHost)` on later reloads), and
`client_resources_reloaded` receives `&dyn ClientReloadHost` (which extends `ReloadContext` with
published client render assets, sprites and instance groups). Published asset/data queries read
the published generation, not pending prepare output: completion callbacks can refresh sprite and
recipe caches after apply, including on first world open, without gaining world handles or
transactions. Callbacks that take `DataLoadContext` use its restricted reads. Neither permits
opening a world handle or transaction. In setup and reload, `context.block_states()` and
`context.fluid_states()` still read registry tables, and an `ItemResource` can query
`damageable(context)`, `foil(context)`, or `repairable(context, &material)`. A position-based
lookup such as `lit_block_name(pos)` requires a live game host. The first client registration and prepare run on the loader
thread; later registration runs on the render thread, prepare on its worker, and apply on the
owning game thread. A panic during reload listener registration discards only that callback’s
pending listeners, dependencies and retained entries. Registration errors are returned and
reported without failing the whole reload; a cycle in the completed graph still fails it.

## Load contexts

`DataLoadContext`, `LoopContext` and `WorldgenHost` lend frozen metadata for their resource
generation. `registry(name)` returns `Option<&[String]>` in registry ID order,
`setting(mod_id, key)` returns `Option<&str>`, and `mods()` returns `&LoadedMods` in dispatch order:
the packages enabled and admitted at startup. Reloading resources or switching code live does not
change this snapshot.
The getters do not read files. `None` means the registry or setting is unavailable, not that it is
present and empty. Use the references within the callback; take an owned value only if it must
outlive the context borrow. Use `DataLoadOrigin` to identify the load origin and `LoopSide` to identify
the owning side.

`DataLoadContext::fluid_tags()` borrows the recipe pass's fluid-tag table. It returns `None` when
there is no recipe overlay, `Some(&[])` when that overlay has no fluid tags, and otherwise a slice
of `(tag ID, members)` pairs with each tag's members in source order.

During registration, `RegistrationContext` lends the same startup inventory (`&LoadedMods`),
built-in registry names (`Option<&[String]>`), captured settings (`Option<&str>`), and instance
directory (`Option<&InstanceDirectory>`) from the startup snapshot.
[Chunk disk callbacks](../how-to/chunk-data-load.md) also borrow registry names, but from the
resource generation retained by their worker.

### Ordinary Host reads

Both client and server hosts expose `registry_entry(registry, id)`,
`registry_entries(registry)` and `registry_entry_in_tag(&entry, tag)`. These read the
current side's live registry and merged tags, not pending reload data. Unknown or
unavailable registries and entries return `None`; an absent tag or a tag that does
not contain the entry returns `Some(false)`. Enumeration yields each identity once.

`ConfigHost::setting` still returns `Option<String>` on an ordinary `Host`; it is not the
captured `Option<&str>` read on a load, loop or worldgen context.

Server world-generation reads use the published decoded owners, including the
live feature data in worlds without a terrain locator. Invalid source files do
not establish entries. Prepared world-preset files have no published complete
registry owner, so `minecraft:worldgen/world_preset` is unavailable; selecting a
preset, sharing generator resources or reopening a saved stem does not change that.

`RegistryEntry` is a plain Wire value containing `registry: Identifier` and
`id: Identifier`. Its names may be retained across
reloads; constructing the value does not establish that the entry exists.
`as_block_key(host)`, `as_item_key(host)` and the corresponding narrowing methods
for block entity types, boolean game rules, dimensions, effects, enchantments,
entity types, fluids, menus and particles resolve a real host-issued typed key.
A different registry or unknown entry returns `None`. Typed keys remain
process-local; narrowing never treats a protocol numeric ID as a key slot.

Mod-declared datapack registries resolve tags against the accepted candidate
before publication; tags with missing required members are omitted. Synchronized
snapshots carry those exact tag names and members (including empty tags);
server-only and unnegotiated registries send neither entries nor tags. Clients
validate membership before activating a snapshot.
On clients, generic reads never fall back to prepared server-only datapack files:
built-ins and the connection's currently synchronized registries are the entire view.

There is no separate common-setup, load-complete or intermod event. Registration and the first server
reload cover common setup, `client_setup` covers client setup, and `server_about_to_start` covers
dedicated server setup. Dependencies express intermod ordering.

`DataLoadOrigin` and `LoopSide` describe the local load or loop, not data to encode. Read them
through `DataLoadContext::origin()` and `LoopContext::side()` as before; if a payload needs such
a fact, map it into a field of your own [Wire record](wire-types.md).

[FML loader APIs and their Sunrise routes](../reference/fml-loader-apis.md) compares the two loaders'
file and path APIs, including the game directory.

## Settings snapshots

Settings snapshots accept regular UTF-8 files only. File and package-directory symlinks,
including internal links, are not followed. Non-UTF-8 filenames, invalid UTF-8 contents,
special files and unreadable entries are skipped individually and omitted from the snapshot
rather than discarding the mod's remaining settings or failing load.

Capture proceeds in sorted package-ID and key-name order. Missing settings directories and
empty settings produce no warning. A capture with skipped entries emits one warning naming
each omitted key, its mod and its reason; an inaccessible or incompletely enumerated
directory is identified by scope instead of inventing a key.
Initial registration, item-data loading, table preload and loop publication share one
captured settings snapshot rather than rescanning it on each startup thread. The game,
audio and world-generation readers retain that generation's immutable snapshot without
recapturing or repeating diagnostics. A new capture reports new omissions,
even if an earlier generation already warned.
