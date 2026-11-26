# Standalone server export

Build a dedicated-server runtime with your native mods:

```sh
sunrise-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code
```

`--trust-native-code` saves your consent to run unsandboxed native code. You can omit it when
consent is saved. Optional flags include `--profile PROFILE` and `--allow-jar-downloads`.

The command exports a standalone dedicated-server engine into `DIR`, which must not exist. `FOLDER` (a mod folder or a
folder of them) is required. The export does not select, edit or launch an instance.
It uses the pinned prebuilt dedicated-server SDK and builds only the mods and native
function-replacement libraries, never the game. `--version` pins the Sunrise release/Nightly
for reproducibility. The export holds the engine executable, prepared resources,
`sunrise-native-mods/<id>/` projects and provenance `native-mods.json`, and prints the exported engine executable's path. Use that value for `engine.path`.
It runs on the same OS/architecture as the host and requires an installed launcher base.
It is copied outside managed cache folders. On Windows use `sunrise-engine.exe`.

Install and enable content `.smod` packages in the server game folder's `sunrise-mods/`.

A mod's crate depends on `sunrise-mod-api` and never on the game: a `sunrise-client` dependency,
under any name or target, is refused naming the mod and the key, before toolchain work. The mod API
has no sound or controller features, so server and client builds stage the same crate.

Declared jars use the same exact-hash preparation as ordinary launches. A server export refuses
missing jars by default and never asks interactively; `--allow-jar-downloads` permits the shared
preparer to download the exact files into the local cache. Cached jar bytes are **never exported**.
When moving the runtime to another machine or game home, prepare its jar cache separately; errors
name the required files and cache paths. No Java code from a jar runs.

## Run the server

Point a server at the exported engine:

    sunrise-server --dir SERVER config set engine.path=/absolute/export/sunrise-engine
    sunrise-server --dir SERVER restart

Updates do not rebuild a pinned `engine.path`: build a new export folder, set `engine.path` to it, restart, and keep
the old folder for manual rollback. The supervisor does not auto-update or rebuild a pinned engine path.

To inspect the running server, its authenticated
[`GET /api/v1/mods`](https://github.com/sunrise-org/sunrise/blob/main/docs/api/server.md) queries the engine's canonical `LoadedMods` inventory:
`mods` lists loaded native and content package ids and versions in inventory order.
`provenance` separately reports the configured program's build record, which may differ and is never
used as an inventory fallback. A stopped or unreachable engine returns HTTP 409, not an empty list.
The configuration handshake checks both-side mod IDs and versions (see [Multiplayer](#multiplayer) below).
Native mods run unsandboxed: only build code you have read and trust.

## Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** client-side hooks run locally. The configuration handshake requires matching both-side mod IDs
  and versions, and, for a mod whose package lists `[runtime]` files, a digest of their names and hashes; a refusal names
  the missing, extra or different-version mods and the ones whose runtime files differ.
- **Dedicated server:** native mods run in a standalone engine export built with `--server-output`, not in an
  instance build (`--server-output` conflicts with `--instance` and needs an explicit mod `FOLDER`).

## Runtime packages and identity

Move the executable, prepared resources and `sunrise-native-mods/` package directory
together when you distribute a native build. Keep each copied project's flat manifest,
crate, assets and data.

The launcher generates the game's embedded native inventory from your selected
`sunrise-mod.toml` manifests: mod id, version and side. Cargo package metadata and
hook factories do not supply identity.

| Rule | Behaviour |
| --- | --- |
| Package location | The game reads each compiled mod's package from `<executable directory>/sunrise-native-mods/<id>/`, not the working directory or a build record. |
| Identity | The package's id, version and side must match the embedded inventory. |
| Startup validation | Before installing hooks or starting a world, the game refuses missing, unreadable, content-only or identity-mismatched packages. The error begins `Compiled native mods cannot be loaded:` and lists the affected mods and paths. Restore the packages exported with that executable or rebuild it. |
| Content-only mods | These use the working directory, player toggles and explicit `--dev-mod` paths. A content package cannot shadow a compiled mod's id. |
| Display metadata | The mod list and crash details include code-only mods from the embedded inventory. Display names and descriptions come from validated manifests. |

`native-mods.json` is provenance and rebuild status, not game identity, authentication, announcements
or startup input. The launcher retains it for build/status displays. The separate `sunrise-server`
supervisor may report the adjacent record as a provenance snapshot, not a query of the running engine.
Its SHA-256 values and revision describe the built binary's inputs, not the running mod set.
The launcher prepares jar data from canonical manifests as non-authoritative candidates; the engine
validates identity against its embedded inventory. Original-jar pack order and
whole-package refusal rules are in [Original Java jar layers](../../reference/manifest.md#original-java-jar-layers).
