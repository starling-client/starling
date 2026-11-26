# Registration and diagnostics

Declare keys, Parts, and crash-report callbacks in `Hooks::register`; submit blocks and items
with `event.register(declaration)` in `Hooks::blocks` and `Hooks::items`. Sunrise collects these
declarations once at startup, before resources load. The registrar and events are borrowed for
their respective hooks and cannot be kept to submit a declaration after the hook returns.

If you expose registration to scripts, require the matching registration hook before
queuing a declaration. `mod_api::registration_open()` can reject calls outside the phase,
but it does not tell you which hook is running. A late call (for example, from
`player_logged_in`) must not enter the queue: return `DeclarationError::RegistrationClosed`
and log the submitting mod and requested ID in the binding. The phase query itself does not log.

The submitting package owns its block and item declarations. `BlocksEvent::register` and
`ItemsEvent::register` return a typed `DeclarationError` for invalid IDs or fields. Checks that
need other declarations or loaded resources run after collection: for example, a submitted item
may conflict with another declaration's generated block item. A refusal is fatal at startup,
even if the caller ignores the error. The startup error and log identify the submitting mod,
declaration kind, ID, and reason.

If your `Hooks::register` bootstrap cannot complete, call `r.refuse("reason")` on the
`Registrar` and return. Sunrise records the refusal directly against the mod whose `register`
hook is running. It does not panic or exit the hook for you. Because `refuse` and `warn` live
on `Registrar` (via `DiagnosticsRegistrar`), they are available only in `Hooks::register`.
These reports are separate from the native block and item declaration errors described above.

## Registration refusal

- `refuse` applies only to the calling mod; it takes no mod ID.
- All declarations from that registration are discarded, including declarations made before
  or after the call. Later declarations do not add secondary validation errors.
- Other mods' declarations are unchanged.
- Every refusal reason recorded during the callback is preserved verbatim and reported,
  including an empty reason. If the callback later panics, Sunrise logs those reasons and
  any already-recorded registration errors with the mod's ID before discarding its pending batch.

## Loading warnings

For log locations and filtering, see [Finding your mod's logs](#finding-your-mods-logs).

A problem that should not stop your mod, such as a missing optional integration or a setting it
corrected, is a warning: `r.warn("mymod.warn.no_jei", &["jei"])` on the `Registrar` inside
`Hooks::register`. Like NeoForge's `ModLoader.addLoadingIssue` with
`ModLoadingIssue.withAffectedMod`, the registration records the warning against the mod that
reported it.

- The key is a translation key in your mod's own `lang/` files; the arguments fill its `%s` in
  order.
- Like `refuse`, it takes no mod ID: the warning belongs to the calling mod. The mod keeps
  loading and its declarations are kept.
- The log names the mod, the key and the arguments at warn level. The Mods screen shows the mod's
  first warning under its row in the player's language, or the key when no language has it; the
  row is not marked failed.
- Warnings are kept in call order with no per-mod limit, and each key and argument is stored
  verbatim.

## Native startup and hook panics

The launcher supplies ordered mod IDs and hook factories to the installed game's
[native entry template](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-client/src/native_main.rs.template).
The template constructs `ModEntry::new("example", example::hooks())` entries and passes them to
`sunrise_mod_host::main_with`. `Mods` is `Arc<[ModEntry]>`: each entry binds its ID to its hooks.
Before installation, `mod_api::mods()` returns an empty list.

Game source without the required template is refused before mod staging with:
“This game source has no native inventory entry template. Update Sunrise and rebuild the mods
against this game.”

A panic in `Hooks::register` disables that mod's registration callback for the session. The
[shared panic report](hooks-and-the-host.md#failures-and-panics) names the mod and hook in the
log and client error toast. Sunrise discards the callback's pending registrations, including
biome modifiers, datapack registries, and attribute widenings, without undoing live writes outside
a transaction. A panic while dropping a rejected Part uses the same report and does not stop
the next mod from registering.

## Crash-report callbacks

Register callbacks inside `Hooks::register` (the counterpart to NeoForge's `CrashReportCallables`):

| Registration | Report output |
| --- | --- |
| `r.crash_section("caches", Arc::new(Caches))` | `<mod id>: caches: <value>` after Sunrise's system details |
| `r.crash_header(Arc::new(Banner))` | `<mod id>: <value>` under the report title |

Both callbacks implement `CrashSection`. The report calls `value()` once when it is written,
not during registration. Return `None` to omit an entry. The report includes the full label and
value, including line breaks, without clipping or flattening them, and marks every continuation
line with the registering mod's ID (`\t<mod id> | ` for headers, `\t\t<mod id> | ` for system
details, including error text). That includes entries from the mod named in the crash itself.

Callbacks run and appear in registration order on clients and dedicated servers. A report is
built after the panicking thread has unwound, as vanilla builds it outside the throw, and the
callbacks run then, on the thread that builds it: the locks the panicking code held are released.
Read your own diagnostic state, and keep a callback cheap.

A failed callback's entry begins with `~~ERROR~~`. A panic logs the mod's ID, queues an error
toast, and disables only that callback for the session. Other failures are reported on each
occurrence and do not disable the callback.

## Converted-package attribution

If your native runtime hosts other packages, declare their IDs with
`mod_api::content::crash::ConvertedPackages::new(runtime_id, ids)`. Obtain a handle with
`packages.package(id)`, then call its `enter(export, site)` method around the converted package’s call.
To also register recipe types, serializers, menu screens and other
`owns_namespace`-checked declarations for generated content packages, opt the
runtime in with `[native] serves_content = true` and declare
`served_by = "<runtime_id>"` in each served content package's manifest
([Manifest reference](../reference/manifest.md#sunrise-modtoml)). Because
`serves_content = true` accepts any installed content package naming the runtime,
verify that a package in `context.mods()` belongs to your runtime before
registering declarations for its namespaces. All namespace-owned registrations
delegate uniformly through `served_by`; only filesystem paths keyed strictly by
package ID (such as `config/<mod_id>.toml`) stay scoped to the host's own ID.

The scope adds `in converted package <id>, export <export>, call site <site>` below the runtime's
`in native mod` line. Nested scopes restore the previous attribution on return and panic unwind.

- `package(id)` returns `None` for an undeclared ID.
- `enter` returns `None` unless the thread is executing a native call owned by that runtime.
- The guard is neither `Send` nor `Sync`. Attribution ends when the native frame that entered
  it exits, even if you retain the converted-package guard.
- A nested native frame hides the outer frame's package until it returns.
- Worker threads need their own native call and package scope.
- Entering a package clones its stored ID on each call; borrowed static IDs do not allocate.

## Finding your mod's logs

Start with `logs/latest.log` in the game's **home**, not the world or mod project folder.
For a default `cargo sunrise-mod run`, that's
`target/sunrise-mod/run/home/logs/latest.log` inside your project. Otherwise check the
home set by `--home` or `SUNRISE_HOME`; without either, the client uses the `sunrise`
directory in the platform's data folder. A dedicated server logs under its game directory.
On the client, `--player-dir` overrides the log location.

The default filter writes `INFO`, `WARN` and `ERROR` records, except that
`calloop` and its submodules log only `ERROR`. Repeated `WARN` and `ERROR` records
are rate-limited by source location: after 100 in a minute, further records
from that location are counted instead of written. For targets that don't start
with `sunrise`, the logger prefixes the message with the record's target in
brackets: the Rust module path by default, not the mod ID from the manifest.
For example:

```text
[14:03:12] [Server thread/INFO]: [mymod::machine] Loaded machine recipes
```

Look in the file rather than the terminal: the game logger echoes only targets that
start with `sunrise` to stderr, without the target prefix. It drops `DEBUG` and
`TRACE`; no setting or environment variable (including `RUST_LOG`) enables them.
Native `log` records reach the game logger only when the pinned SDK declares native
log forwarding (`native_log`).

## Measuring a mod's calls

The server's `/modprofile` command measures how long native mods spend in their hooks and Parts. It needs
permission level 3 and is off until you start it.

1. `/modprofile start` starts a measurement. Starting twice reports an error.
2. Play through what you want to measure.
3. `/modprofile stop` ends it and saves `profiles/modprofile-<timestamp>-<counter>.json` in the game folder.
   Stopping with no measurement running reports an error.
4. `/modprofile report` prints the last finished summary, without saving a file or stopping a running measurement.

Replies go only to the player who ran the command. The same controls work through
`sunrise-ctl command modprofile start` (and `stop` or `report`).

### The profile file

Open the file in [Firefox Profiler](https://profiler.firefox.com) and look at its Marker Table. Sunrise never uploads
it.

- The file is written atomically and is at most 8 MiB. If saving fails, the command says so and the summary stays
  available to `report`.
- Each marker names a mod, a hook or Part and holds call counts, exact timings in nanoseconds and log2 histograms. It
  never holds arguments, player data or world data.
- Timings are inclusive (an outer call includes the calls inside it) and add up over the whole process: they are
  totals, not a trace of each call or a sampling profile.
- Only native mods' calls are measured.
