# Contributing to Starling

Thanks for helping. Starling is the Android port of [Sunrise](https://github.com/ludovic111/sunrise), Minecraft: Java Edition 26.3 rewritten in Rust. This page is the short version of how we work.

## Where a change goes

- **The game itself** (gameplay, world generation, rendering shared with the desktop, menus) belongs upstream, in Sunrise, where it is checked against vanilla. Follow [Sunrise's contributing guide](https://github.com/ludovic111/sunrise/blob/main/CONTRIBUTING.md): vanilla is the specification, and gameplay changes come with a parity harness.
- **The Android app** belongs here: the packaging (`packaging/android/`), the app's entry (`crates/sunrise-client/src/android_lib.rs`), touch controls (`touch.rs`, `game/touch.rs`), the Starling folder and imports (`import.rs`, `game/imports.rs`, `game/files_offer.rs`) and anything that only matters on a phone.

**Mod-related pull requests come here too.** Sunrise is where the mod API is built: the mod API, native hooks and
events for mods, native mods, content mods (`.smod`), mixins, `cargo sunrise-mod`, `sunrise-mods`, mod
builds and the registry, and their docs ([mods](docs/mods.md), [native mods](docs/native-mods.md)). A change to the mod
API follows its clean-break rules ([AGENTS.md](AGENTS.md#mod-api-the-clean-break)).

When adding a native hook, keep its declaration under `crates/sunrise-mod-vocab/src/hooks/` and wire the engine
function named by its target. Vocabulary generation does not read engine bodies, so it cannot tell whether
those targets fire the hooks. `verify-static` runs `bash tools/ci/check_hook_wiring.sh` separately, without
changing any sources.
The check follows the declared Rust module to the target body, including conditional files and inline modules.
It checks non-test `cfg` branches using supported build targets' Rust cfgs, including `cfg!` conditions, and
does not count calls in closures that are never invoked. When a hook uses a client-library or world bridge,
the bridge must invoke the slot and every reachable installer call must bind a callback that reaches the
dispatcher. For slots upstream of a catalog target, the gameplay entry point must also call the bridge.
A native render replacement slot is intentionally empty, so the check starts at the installed renderer callback.
`cargo run -p xtask -- mod-api` fills a new game hook's placeholder during authoring; a placeholder left in the
checkout fails the wiring check.

## Build it

You need Rust (stable) with the `aarch64-linux-android` target, [cargo-ndk](https://github.com/bbqsrc/cargo-ndk), the Android SDK (build-tools 35.0.0, platform 36, NDK r28c), a JDK 25 and Python 3.10+.

```sh
python packaging/android/build_apk.py            # build the library, then the APK
python packaging/android/build_apk.py --install  # and install it over adb
```

The APK carries no game files. On its first start the app signs in, downloads them from Mojang for an account that owns the game and prepares them on the phone; to skip that while testing, prepare the resources from your own copy of Minecraft 26.3 as described in [Build from source](README.md#build-from-source), zip them as a `.starling` file and open it with the app. The setup's hand-drawn skin comes from `python packaging/lowbudget/make.py` (Pillow): run it after changing `hand.py`, `make.py` or `strings.json`, and commit `pack/` with them.

## Test on a phone

Say what you checked on a real device, and on which one. A `sunrise-args.txt` in the app's own files adds command-line arguments and environment variables at launch (one per line, `NAME=value` for the environment); with `--control-socket` and `SUNRISE_CONTROL_PORT`, `sunrise-ctl` drives the game over `adb forward`. Empty the file afterwards so the app starts normally.

## Only original code, or code whose license allows it

Starling is MIT, as Sunrise is. Its code is written from Minecraft 26.3 itself and from public documentation. Another project's source may also be read and used, **only when that project's license allows it**:

- **Allowed:** MIT, MIT-0, BSD-2-Clause, BSD-3-Clause, 0BSD, ISC, zlib, Apache-2.0, Unlicense, and the CC0-1.0 public-domain dedication. That code may be read, copied or adapted.
- **Java's own library is GPL too.** OpenJDK's source (`java.lang.Math`, `StrictMath`, `FdLibm`, `HashMap`, the sorts, `Double.toString`, HotSpot's intrinsics...) is GPL-2.0, so it is off limits like the rest. Reproduce Java's behaviour from its API documentation (the Javadoc) and from what a JVM returns, or from a permissive original such as Sun's fdlibm C sources (see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)); see #6741.
- **Everything else is off limits, even to read:** GPL, LGPL (Iris), AGPL, MPL, EPL, Polyform (Sodium), the Creative Commons licenses (CC BY, CC BY-SA, CC BY-NC and the others; CC0 is a public-domain dedication, not one of them), custom or "source available" licenses, all rights reserved (OptiFine), and any project with no license or an unclear one. Don't read, copy, translate or paraphrase their source, and don't send code an AI tool wrote from it. Being able to download a mod or a pack does not give you the right to read its files.
- **Check the license yourself.** Read the LICENSE file at the commit you use and the header of each file you take: some repositories mix licenses, and a file under another license is off limits. A license that changed: the one at your commit counts. In doubt, don't: ask in an issue first.
- **Say it in the pull request:** the project, a link to the commit, its license, and what you read or copied.
- **Copied or adapted code** keeps its copyright line and license notice at the top of the file, and gets an entry in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Apache-2.0 code also keeps its NOTICE text and says what was changed.

User documentation and file formats are fine to read, and packs and mods are fine to run and observe, whatever their license.

## Before a pull request

```sh
git config core.hooksPath .githooks
python3 tools/pr_ready.py
```

It runs rustfmt, clippy with warnings denied, the tests of the crates you touched, the Python tests and the public-tree check. Keep pull requests small, and write commit messages in English.

For the game workspace, Clippy checks all targets of the touched crates and their transitive workspace dependents, including examples. The dependency scope includes optional, platform, build and development dependencies. Tests run library, binary, integration and doc tests without building unrelated example executables. Shared Cargo configuration and toolchain changes select the whole workspace; CI also keeps its workspace-wide checks.

Never commit game files, decompiled source or prepared resources (`.local/`): `tools/check_public_tree.py` rejects them (`git config core.hooksPath .githooks` runs it before every commit). The pre-push hook checks the full history reachable from the branches and tags being pushed; `python3 tools/check_public_tree.py --history` audits the history of all local refs.
