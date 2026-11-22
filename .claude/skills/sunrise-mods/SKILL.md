---
name: sunrise-mods
description: Make, build, test or debug a Sunrise mod (a content or native mod, the `.smod` package, the mod SDK, `cargo sunrise-mod`, `native-lock`), or change the mod tools in this repository and try the change on a real mod. Use for "make me a mod", "why doesn't my mod build", "does this mod fit the new build", a manifest or lock error, and any work in `crates/cargo-sunrise-mod`, `crates/sunrise-mod-package` or `crates/sunrise-native-build`.
---

# Sunrise mods and the mod tools

The modding book (`docs/modding/src/`, built at https://playsunrise.net/docs/modding) is the reference. This page
says which page answers what and the few things that are not in it. Read the page; do not work from memory of it.

## Writing a mod

Choose the walkthrough under `docs/modding/src/` for the kind of mod you are making:

- **Native:** `for-agents.md` and `start/first-mod.md`. Use `cargo sunrise-mod check` after an edit, `build`
  to compile and pack, `run` to stage and play, and `cargo test` for the Rust logic.
- **Content-only:** `start/first-content-mod.md`. Use `cargo sunrise-mod build` and `check` the resulting
  `.smod`; there is no Cargo test target or SDK to install.

`cargo sunrise-mod new` generates the project and `sunrise-mod.toml`. Edit declared fields such as the name,
description and dependencies as needed (`reference/manifest.md`); do not write the manifest from scratch
or change generated pins by hand. For code mods, `cargo sunrise-mod update` changes the pins.

- `cargo sunrise-mod help all` lists the commands; `reference/sdk/05-commands.md` describes the SDK commands.
- Find a hook with `cargo sunrise-mod hooks <words>`, never by guessing a name.
- The mod tools need no launcher. If a step seems to need one, that is a bug in the tool or the book: report it.
- To try a mod in game, follow `AGENTS.md` for a scratch home and game directory, never an existing player
  install. Use resources prepared from the user's own Minecraft (`start/first-mod.md`, "Prepared resources").
  Ask for the folder. Never download or commit Mojang's files.

## Which page

| Question | Page under `docs/modding/src/` |
|---|---|
| Blocks, items, recipes, loot, worldgen as data files | `how-to/content-mods.md` |
| The crate, the package, data files of a native mod | `how-to/native-mods.md` and its numbered pages |
| A compile fails | `how-to/native-mods/05-build-log.md` |
| Changing game code (`[native.*]` tables, mixins, loops) | `concepts/native-route.md` |
| The SDK: what a release holds, the pin, signatures | `reference/sdk.md`, `reference/sdk/01-release.md`, `reference/sdk/03-in-a-project.md` |
| Does a mod still fit a newer build (`fits`, `update`, `uses.json`) | `reference/sdk/04-fit.md`, `how-to/native-mods/14-checking-a-mod-against-a-build.md` |
| crates.io dependencies and `Cargo.lock` (`build --update-lock`, `native-lock`) | `start/first-mod.md` (step 5), `reference/sdk/08-native-build.md` |
| The JSON contract a build server or launcher drives (`native-build`, `native-check`, `native-lock`) | `reference/sdk/08-native-build.md` |
| Testing a mod's logic | `concepts/testing.md` |
| Publishing and the registry | `how-to/publish.md` |
| Installing a `.smod` by hand | `players/installing-mods.md` |

## Changing the mod tools

- The tool is `crates/cargo-sunrise-mod`; the package rules and manifest are `crates/sunrise-mod-package`; the
  native compile and its JSON commands are `crates/sunrise-native-build`. `AGENTS.md` has the crate table and the
  mod API's rules.
- Try a change on a real mod with a host debug build of the tool. From the checkout, resolve the executable
  before changing directories. Only code templates fetch an SDK: the current Nightly by default, or the
  build named by `new --sunrise <build>`. The example is for a native mod; use the matching walkthrough above
  to smoke-test a content-only change.

  ```sh
  target_dir="$PWD/target/mod-tool-smoke"
  cargo build -p cargo-sunrise-mod --target-dir "$target_dir"
  mod_tool="$target_dir/debug/cargo-sunrise-mod"
  scratch=$(mktemp -d)
  export SUNRISE_MOD_CACHE="$scratch/mod-cache"
  "$mod_tool" new "$scratch/smoke-mod" --template native
  cd "$scratch/smoke-mod"
  "$mod_tool" check
  "$mod_tool" build
  ```

- `python3 tools/book_quickstart.py --tool target/debug/cargo-sunrise-mod` runs the start pages' commands with that tool in a
  fresh home; run it after changing `new`, `check`, `build` or a start page.
- The SDK a project uses is in its `.sunrise/sdk/`; compiled slices and build work are in the per-user cache
  (`SUNRISE_MOD_CACHE`, see `start/first-mod.md`). `SUNRISE_SDK_BASE` (a URL or a folder) replaces the release
  server (`crates/cargo-sunrise-mod/src/sdk_fetch.rs`).
- Classify tests using the [CI tiers](../../../CONTRIBUTING.md#ci-tiers). Mark new daily tests
  `#[ignore = "daily"]` and add sorted rows in a new `tools/ci/ignored-tests/daily-PACKAGE-TOPIC.txt`.
  That directory's `README.md` explains the format; `python3 tools/ci/check_ignored_test_rows.py` checks the rows.
- A change to a command, a refusal or a file format updates the book page that describes it in the same pull
  request. Input from a mod (manifest, package, source) is refused by name, never a panic.
- An error tells the author what they did, why it is refused and what to do next, in their terms: their file,
  their key, the command to run.
