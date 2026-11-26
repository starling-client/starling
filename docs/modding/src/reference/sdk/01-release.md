# The release

Every SDK is the release `sdk-<build id>` of this repository, at
`https://github.com/sunrise-org/sunrise/releases/download/sdk-<build id>/<file>`. Its files:

{{#sdk-assets}}

The compiled slices exist once per [host](#hosts); a file named `…-server.tar.xz` is the dedicated server's.
Everything else exists once per build. `.tar.xz` files are tar archives compressed with xz; `surface.json` is the
file itself.

A Nightly carries a copy of its build's `sdk.json` in its own signed sums. The launcher learns which build a game is
from the copy in that game's release, and keeps the answer, so it is known offline afterwards.

## Which slices a build needs

The launcher fetches only the compiled slices the instance's mods need, decided once for the whole set of enabled
mods. The routes and their slices come from the same input list the launcher and the packager read:

{{#sdk-routes}}

Each slice unpacks into its own folder. Using a kept slice again checks only its files' names and sizes against a
stamp written after its verified unpack; it does not hash them again.

## `sdk.json`

The index. Readers ignore fields they don't know and refuse a `schema` they don't know: adding a field keeps the
number, removing or redefining one changes it.

| Field | What it is |
|---|---|
| `schema` | The index's schema number (1). |
| `build.commit` | The build id: the game's full commit sha. |
| `build.game` | The vanilla version the build implements (`26.3`). |
| `build.date` | The commit's date, `YYYY-MM-DD`, UTC. |
| `build.nightly` | The Nightly name that ships this build, when one does. |
| `package_format` | The prebuilt package's layout number. Readers of compiled packages require a match. The [offline registry fit check](../registry-index.md#fit-on-stable-and-nightly) does not open compiled packages and does not need a match. |
| `toolchain.rust` | The Rust release everything in the SDK was compiled with. |
| `content_api` | The content manifest's version: the `api` key of `sunrise-mod.toml`. |
| `surface.hash` | `surface.json`'s own hash: equal hashes, equal surface. |
| `surface.previous` | The build id of the SDK published before this one, which it is compared with. Absent on the first. |
| `surface.source` | That comparison in one word, its worst change: `identical`, `additive`, `conditional` or `breaking` ([classes](02-surface.md#what-a-change-is)). |
| `surface.changes` | The ids of the records that changed, breaking ones first. |
| `surface.notes` | The change notes attached to them, each `{ ids, why, migrate }` ([notes](02-surface.md#change-notes)). |
| `mixins.rev`, `mixins.patch` | The pinned horizon-mixins tool's revision and the SHA-256 of the patch carried on it. |
| `assets` | Every file of the release but `sdk.json` and the sums, by name: `sha256`, `size`, `kind`, `codec` (`none` or `xz`), and for a compiled slice its `target` and `side` (`client` or `server`). |
| `inputs` | The files each route needs, named before any download, build or packaging starts: its SDK slices, the programs beside the game and the checkout files, how to make each, and what each slice must hold. |

## How a download is checked

The mod tool and the launcher check an SDK the same way, whatever the [base](03-in-a-project.md#mirrors-and-offline-use)
it came from, and refuse it by name at the first step that fails:

1. `SHA256SUMS` carries Sunrise's signature for the tag `sdk-<build id>`. That signature never passes for a game
   release, and a game release's never passes for an SDK.
2. `sdk.json` is the file those sums list, names this build, and has a schema and package format the reader knows.
   Every asset names its kind once. For an asset of a known kind, the reader checks that its file name says what
   the entry claims and that it knows its codec. An asset of a kind the reader doesn't know (a newer SDK's) is
   left out unread, and a route that needs one is refused by name only when it is used.
3. Every other file has the size and SHA-256 `sdk.json` gives, checked before it is decoded. An archive unpacks into
   a temporary folder, renamed into place only when every entry unpacked, and is refused past an unpacked size
   limit or for any entry that is not a plain file or folder inside it.

An SDK newer than the launcher reads (its schema, package format or Rust) asks for a launcher update; one made with
an older Rust than the launcher compiles with asks for a newer game build.

## Hosts

The compiled slices are built for these computers, the game's and, where listed, the dedicated server's:

{{#sdk-hosts}}

Building a native mod requires a matching prebuilt game slice. Sunrise does not publish game or SDK builds for
Intel Macs, Windows on ARM or musl Linux. Dedicated server slices are available only for Linux x86-64.

Authoring a mod can use the API source in `.sunrise/sdk/` on any host where Cargo runs. Running native
`cargo sunrise-mod check` or `cargo sunrise-mod build` requires matching compiled SDK slices for the target host.

## Retention

An SDK release is never deleted whole. Pruning deletes only the assets whose `kind` in the release's signed
`sdk.json` is `runtime`, `api` or `engine`. A pruned build keeps `sdk.json`, `surface.json`, `build-data.tar.xz`,
`api-source.tar.xz`, `SHA256SUMS` and `SHA256SUMS.sig`, so for it:

- `cargo sunrise-mod sdk diff` and `check --build`, and the launcher's fit test, keep working;
- a mod project pinned to it still fetches its `.sunrise/sdk/`;
- building a mod against it on a player's computer needs its compiled slices, which are gone.

The set kept whole is every build a current or dated Nightly ships, and the ten newest other SDKs. Automatic pruning
is not switched on yet: until it is, every published SDK stays whole. The policy, with how the kept set is computed,
is in the release flow's
[retention design](https://github.com/sunrise-org/sunrise/blob/main/docs/release-flow.md#prebuilt-sdk-retention-design-3011).
Releases of the older `prebuilt-<sha>` form are outside it and removed by hand.

The launcher keeps its own copies under its own rules ([Cache and disk](../../how-to/native-mods/04-what-the-launcher-does.md#cache-and-disk)):
a build's small files are kept, so a mod's fit is told offline, and its compiled slices are evicted with the build.
