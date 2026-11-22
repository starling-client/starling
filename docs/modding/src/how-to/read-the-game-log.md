# Find your mod's diagnostics

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

On builds affected by [#3677](https://github.com/sunrise-org/sunrise/issues/3677),
`log` macros in native mods built against the prebuilt SDK do not reach the game logger.

For a loading problem players should see, call
`r.warn("mymod.warn.optional_integration", &["other_mod"])` on the
`Registrar` passed to `Hooks::register`. Put the key in your mod's language
files, with a `%s` for the argument. Sunrise logs the [loading warning](../concepts/registration-and-diagnostics.md#loading-warnings)
under your mod's ID, and the Mods screen can show it without refusing the mod.
