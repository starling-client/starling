# Commands

The `cargo sunrise-mod` commands for mod projects. One tool serves every build: it reads each build's data from its
SDK, fetched as [In a mod project](03-in-a-project.md) says, and refuses a schema it doesn't know. Run them in a mod's
folder.

| Command | What it does |
|---|---|
| `new <path> --template native [--sunrise <build>]` | Makes a code mod pinned to the current Nightly's build, or the one named, and fetches its SDK. Without `--template` it makes a content mod, which has no SDK. |
| `build` | Fetches the pinned SDK when needed, then packs the `.smod` with its [`uses.json`](04-fit.md#usesjson). For a native mod it then compiles the crate against the build's prebuilt game, as [`native-build`](08-native-build.md) does, with no launcher: see [`build` for a native mod](#build-for-a-native-mod). |
| `build --binaries <dir> --sunrise-release <tag> --releases <file> [--server]` | Builds and packs a standalone API native mod pinned to that stable SDK. Writes the unsigned archive for this computer's SDK host; `--server` selects the dedicated-server host. |
| `build --describe-binaries <dir> --sunrise-release <tag> --releases <file> --sdk-data <dir>` | Reads the SDK host list, checks each signed archive's structure and identity, and writes the unsigned descriptor. Does not compile or sign. |
| [`release plan`](#release-plan) | Writes a native release or rebuild plan and fetches the SDK metadata it needs, without building or publishing. |
| [`release scripts`](#release-scripts) | Writes the workflow scripts embedded in this tool. |
| [`release prepare-rebuild`](#release-prepare-rebuild) | Checks the exact reviewed package and prepares a disposable project or manifest. |
| `keygen [--out <dir>] [--no-password]` | Makes a Minisign pair in the [mod’s per-user key folder](../../how-to/publish.md#signing) or the named folder. Prints the public key; the private key is passphrase-encrypted by default and always on Windows. Refuses to overwrite either file. |
| `sign <file> [--key <path>] [--no-password]` or `sign <file> --key-env` | Writes the detached `<file>.minisig`, without overwriting an existing signature. Uses the current mod’s per-user signing key by default. For CI, `--key-env` reads `SUNRISE_MOD_SIGNING_KEY` and optional `SUNRISE_MOD_SIGNING_PASSPHRASE` with no prompt; see [Signing](../../how-to/publish.md#signing). |
| `run [--resources <folder>] [--jars <dir>] [-- <game args>]` | For a native mod, builds, stages the game with the mod in `target/sunrise-mod/run/` and starts it. |
| `check [--uses]` | The package rules, and every render pipeline shader (`assets/*/shaders/pipeline/**/*.wgsl`) composed and validated by the game's own code, failing with `<file>: <reason>` for each refusal ([render pipelines](../../how-to/client/render-pipelines.md)); then, for a code mod, fetches the pinned SDK when needed and runs `cargo check` for this computer, then what the mod uses of its pinned build: `Greeter uses 14 items of the API (6 hooks, 5 methods, 3 types). Made for nightly-2026-11-13 (afe471c).` `--uses` lists each item with the line that brought it in, and how the mod builds each struct it builds with a literal. |
| `hooks <text> [--all]` | Searches the pinned build's hooks offline, by id, doc and their NeoForge, Fabric and Paper names; every word must match. `--all` searches every record. Matches in the id come first, then in a counterpart, then in the doc. |
| `sdk diff <build> <build>` | What changed in the surface from the first build to the second, [classified](02-surface.md#what-a-change-is), with the second build's change notes (why, and what a mod does now) under the changes they name: the notes of the changes since the SDK published before it. It reads only the builds' small files, so it works for builds whose compiled files are pruned. |
| `fits [<file.smod>] [--build <build>]…` | Whether the mod, or a package, fits the current Nightly or each build named: one line per build, `made for`, `fits`, or `does not fit` with the changed items. |
| `update [--sunrise <build>]` | Moves the pin to the current Nightly (or the build named). Reports API uses that would break before moving the pin. |

## `build` for a native mod

After packing, `build` fetches the build's `runtime`, `api` and `engine` slices for this computer once, checks the
signed sums and each slice as the SDK fetch does, and unpacks them into the tool's cache, shared by every project:
`$XDG_CACHE_HOME/sunrise-mod` (else `~/.cache/sunrise-mod`) on Linux, `~/Library/Caches/sunrise-mod` on macOS,
`%LOCALAPPDATA%\sunrise-mod` on Windows; `SUNRISE_MOD_CACHE` names another folder. Deleting the folder clears it.
The Rust is the one `sdk.json` names: `rustup`'s, installed with `--profile minimal` when missing, or the `rustc` on
`PATH` when it is exactly that version; otherwise `build` stops and says the version and both ways to get it.
A mod with crates.io dependencies and no `Cargo.lock` gets one; `build --update-lock` resolves again, and is refused for
a content mod. The compile
is [`native-build`'s](08-native-build.md), in process, with registry crates approved (the mod's own `Cargo.toml`),
for the `client` side. It prints `Built <library>` and the build log's path, and exits non-zero with the error on failure.

In `--binaries` mode, `build` uses the same compiler but fetches only the verified `runtime` and `api` slices
for the selected host. It refuses mod dependencies and routes other than the standalone API route. It creates a
`Cargo.lock` if absent, or resolves it again only with `--update-lock`, then includes it in the `.smod`. See
[Publishing native binaries](../../how-to/publish.md#publishing-native-binaries) for signing and combining the outputs.

`run` additionally needs `--resources <folder>`, prepared resources made from your own Minecraft (`sunrise-prepare
--prepare --output <folder>`). It copies the built game program, its libraries, the `native-mods.json` record naming
the mod's library and the mod's package into `target/sunrise-mod/run/app/`, keeps the game folder and `SUNRISE_HOME`
in `run/game/` and `run/home/`, and starts the program with `--game-dir`, `--resources` and the arguments after `--`.
When the pinned game supports `SUNRISE_LOG_STDERR`, `run` shows the native mod's own log records (INFO and above)
live in the terminal. The full client log is normally in `target/sunrise-mod/run/home/logs/latest.log`.
For a mod with `[[jar]]` entries, `run` verifies and stages the originals before building. See [Java jars as external
assets](../../how-to/content-mods.md#java-jars-as-external-assets) for the default store, `--jars <dir>` and the development home requirements.

## `release plan`

For a tagged release, run from the checked-out mod project. The tag must be `v` followed by the manifest's
full version, and its native pin must match exactly one stable release for its game:

```sh
cargo sunrise-mod registry releases --out releases.json
cargo sunrise-mod release plan --tag v0.1.0 --releases releases.json --sdk-data sdk-data --out plan.json
```

For a rebuild, supply a checkout of the reviewed registry and its published `index.json`:

```sh
cargo sunrise-mod release plan --registry sunrise-mods --index index.json --id my_mod --repo you/my_mod \
  --releases releases.json --sdk-data sdk-data --out plan.json
```

Without a requested pair, the newest eligible reviewed mod version wins, and only the newest stable release of its
source line is considered for each version. An older target is not backfilled automatically. Pass `--version <version>`
and `--target <stable-tag>` together for another pair; a requested pair that is unavailable, already covered, on a
different line or otherwise ineligible is an error. Versions without published binaries and taken-down source are not
rebuild candidates. Planning does not prove that the source fits the target: verify the reviewed package's bytes and
signature, then use `publish` to check fit before submitting a build.

The JSON output has `schema: 1`, `tool_commit` (the planning tool's commit) and `plan`. A scheduled rebuild with no
candidate has `plan: null` and writes no SDK data. Otherwise `plan` has the mod `id`, `version`, `author_tag`, stable
`source` and `target` records (`tag`, `commit`, `line`), the target SDK's `rust` release and `mixins` revision, and `hosts`.
Each host records its `name`, Rust `triple`, Actions `runner`, `side`, optional `archive`, and whether it is the
`source_producer`. The plan names the `source_asset` and, when making binaries, the `descriptor`. `mode` is `initial`
for a release with binaries, `source-only` for a release with just one client job and a `reason`, or `rebuild` for a
binary append with `reviewed_source` (author asset URL, size, SHA-256, key, signature and owners).

`--sdk-data` must be a new directory under an existing parent. `--out` must also have an existing parent, outside
that SDK-data directory. Release discovery supplies tags and commits; it does not authenticate an SDK. The command checks the signature for each `sdk-<commit>` tag, its SDK index, and the assets
it fetches. It writes `sdk.json` and `surface.json` under `sdk-data/<commit>/` and extracts `build-data/` there for each
SDK needed by a binary plan. It does not fetch compiled SDK slices. Failed verification leaves any existing plan
file unchanged.

## `release scripts`

`cargo sunrise-mod release scripts --out <new-dir>` extracts the native release workflow scripts from this tool
into a new directory under an existing parent. It never overwrites an existing directory. The generated workflows
first authenticate the tool, then use the scripts from that same tool commit rather than checking copies into the
mod repository.

## `release prepare-rebuild`

`cargo sunrise-mod release prepare-rebuild --plan <plan.json> --package <original.smod> --out <new-dir> [--build]`
checks that a signed native package is the exact reviewed source named by the rebuild plan. It reads the same
standalone eligibility and lock rules as the registry. With `--build` it unpacks into a new directory and repins
that directory's manifest to the plan's target SDK. Without `--build` it writes only the original manifest so
signing and publication can read metadata without unpacking source. Neither mode runs Cargo or modifies the input.

## `fits`

`fits` exits 0 when every build fits, 1 when one does not, and 2 when it cannot tell: an input it cannot read or
fetch, or a question that doesn't apply (a content mod, a mod made for exactly one build, a package made by an older
tool). A mod's own scheduled CI can run it to learn of a break before players do:

```text
Greeter:
nightly-2026-11-13 (afe471c)  made for
nightly-2026-11-14 (0c1d2e3)  fits
nightly-2026-11-20 (9a8b7c6)  does not fit: 2 items changed
    the hook `player_logged_in` (changed)
    the method `ServerHost::give_item` (removed)
```

## `update`


For a native mod, before it repins, `update` lists every item the mod uses that changed between the two builds, the source line that
uses it, for a removed item the closest one in the new build (one with the same counterpart name first, then the
nearest id and doc text), and the change note's `migrate` line when the change has one. It then lists the game
code the mod declares (mixin targets, loops, internals) that changed. A mod made for exactly one
build is only named; its declared internals are checked as the pin moves.

The same checks without the tool, from a build's `surface.json` and a mod folder, are `sunrise-stage uses` and
`sunrise-stage fits` ([Checking a mod against a build](../../how-to/native-mods/14-checking-a-mod-against-a-build.md)).
