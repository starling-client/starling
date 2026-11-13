<!-- chapter: Standalone server export -->
<!-- separator: 1 -->

### Standalone server export

`horizon-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code [--profile PROFILE] [--allow-jar-downloads]`

Builds a standalone dedicated-server engine into `DIR`, which must not exist yet. `FOLDER` (a mod folder or a
folder of them) is required; no instance is read, selected, edited or played, and no launcher config is written.
It reuses the pinned toolchain, source and horizon-mixins and compiles `--no-default-features` (no audio or
controllers). `--version` pins the Horizon release/Nightly the
export reuses (explicit for reproducibility). The export holds the built `horizon-engine` executable, prepared resources
the canonical `horizon-native-mods/<id>/` projects and the provenance `native-mods.json`, and prints its absolute path. It runs on the same OS/architecture as the
host only (no cross compilation), needs a prepared installed launcher base release, and survives cache pruning
because it is copied outside the managed folders. On Windows use the printed `horizon-engine.exe` path.

For wasm mods that declare internals, put their `.hmod` packages directly in `FOLDER`, alongside any
native projects. A folder containing only wasm packages with internals also works. The export compiles
their declared internals and raw listener targets into the engine's table, without consulting an
instance or its enabled-mod list. The selected paths are recorded in `native-mods.json` and changing
them selects a new cached build. Packages without wasm internals do not select table entries.
Install and enable the same `.hmod` packages in the server game folder's `horizon-mods/` as usual;
the export carries the compiled table, not those packages or the server's configuration.

A mod's crate depends on `horizon-mod-api` and never on the game: a `horizon-client` dependency,
under any name or target, is refused naming the mod and the key, before toolchain work. The mod API
has no sound or controller features, so server and client builds stage the same crate.

Declared jars use the same exact-hash preparation as ordinary launches. A server export refuses
missing jars by default and never asks interactively; `--allow-jar-downloads` permits the shared
preparer to download the exact files into the local cache. Cached jar bytes are **never exported**.
When moving the runtime to another machine or game home, prepare its jar cache separately; errors
name the required files and cache paths. No Java code from a jar runs.

Point a server at it:

    horizon-server --dir SERVER config set engine.path=/absolute/export/horizon-engine
    horizon-server --dir SERVER restart

Updates do not rebuild a pinned `engine.path`: build a new export folder, set `engine.path` to it, restart, and keep
the old folder for manual rollback. The supervisor does not auto-update or rebuild a pinned engine path
(remaining work in horizon-rust/horizon#6304). To inspect the running server, its authenticated
[`GET /api/v1/mods`](../../../../api/server.md) queries the engine's canonical `LoadedMods` inventory:
`mods` lists loaded native, wasm and content package ids and versions in inventory order.
`provenance` separately reports the configured program's build record, which may differ and is never
used as an inventory fallback. A stopped or unreachable engine returns HTTP 409, not an empty list.
Configuration mod matching is implemented as described above.
Native mods run unsandboxed: only build code you have read and trust.
`--trust-native-code` records the operator's decision; an agent must not supply that consent.
