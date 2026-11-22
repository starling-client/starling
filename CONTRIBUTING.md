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

<<<<<<< sync-sunrise-1
The APK carries no game files. On its first start the app signs in, downloads them from Mojang for an account that owns the game and prepares them on the phone; to skip that while testing, prepare the resources from your own copy of Minecraft 26.3 as described in [Build from source](README.md#build-from-source), zip them as a `.starling` file and open it with the app. The setup's hand-drawn skin comes from `python packaging/lowbudget/make.py` (Pillow): run it after changing `hand.py`, `make.py` or `strings.json`, and commit `pack/` with them.
=======
- **Launcher:** its own repository, [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) (`cargo run` there; on NixOS, `nix-shell` first).
>>>>>>> sunrise/main

## Test on a phone

<<<<<<< sync-sunrise-1
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
=======
Wherever vanilla has a feature, vanilla is the specification, down to the order of random calls. **Parity** means
Sunrise behaves and looks bit-exactly like Minecraft: Java Edition 26.3 on the same machine: the same inputs give the
same outputs, every float, random draw and pixel. A **harness** checks it: a `tools/test_*_parity.py` script that
runs a small Java oracle against the real game and a Rust probe against Sunrise, and compares their outputs exactly.
A harness compares the values that decide behaviour and the picture: floats, random draws, entity and block state,
model and mesh data, colours, draw lists, HUD draw calls and sound events, and the images the CPU builds (atlases,
mipmaps). The screen picture is compared between builds, pixel for pixel (`tools/parity.py frame_identity`), so a
change can prove it leaves it alone. Comparing it with vanilla's needs both games offscreen on the same graphics API
and driver; never a tolerance on a screenshot.
Out of scope: what no run repeats, such as timing, thread scheduling, frame pacing and the mixed audio (the sound
events are in scope).
A measured gap that isn't fixed yet, such as a `Math` result next to a rounding tie where HotSpot's own routine
misrounds, is listed in #2295 until it is. So are the harnesses that still compare with a tolerance: they lose it area
by area. A pull request never adds one, and isn't held back by one it didn't add.

- **Read the vanilla code for what it does, not how.** `tools/rea_vanilla.py` queries the pinned game bytecode:
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
  Java is the reference for behaviour, never for structure or speed: code that looks like Java proves nothing, a
  bit-exact harness does. Say in your pull request which vanilla classes and methods you read.
- **One version: 26.3.** Parity work targets 26.3 until its parity is good, redstone first. Don't open pull requests
  that move Sunrise to a newer Minecraft version for now; to join servers on a newer version, put a Via proxy
  (ViaVersion, ViaProxy) in front of them.
- **Gameplay changes come with a harness**, new or extended, so the next change can't silently break them.
  [The parity workflow](docs/parity-workflow.md) shows how harnesses run; copy a nearby one.
- **Performance work stays bit-exact.** A faster path gives the same results, random draws and pixels as the code it
  replaces. The area's harnesses prove it; if none covers the code, add one first. Add a before/after benchmark.
- **No workarounds.** When a harness differs, fix the cause until it matches bit for bit. Never add a tolerance,
  rounding, a special case or a compensating patch to make it pass. Removing a workaround, or a tolerance from a
  harness, is welcome when the harnesses stay bit-exact: smaller code is a goal.
- **Sunrise's own features** (the launcher, the new menus) follow [the design system](docs/design-system.md) and are
  off by default wherever they would change the vanilla look.
- **Three layers: engine, platform, Sunrise.** Vanilla code never depends on the platform (mods, shader packs,
  Sunrise-only settings) or on Sunrise (Discord, Play with Friends, telemetry), and the platform never
  depends on Sunrise. [docs/layers.md](docs/layers.md) says where a new feature goes; CI checks the dependencies.
- **Everything players run is Rust.** Python and Java are fine for dev tools and harness drivers in `tools/`, never
  for player-facing code.
- **No game files in the repo.** Never commit game files, decompiled source or prepared resources (`.local/`):
  `tools/check_public_tree.py` rejects them (`git config core.hooksPath .githooks` runs it before every commit).
  The pre-push hook checks the full history reachable from the branches and tags being pushed. Run
  `python3 tools/check_public_tree.py --history` to audit the history of all local refs.

## Only original code, or code whose license allows it

Sunrise's code is under the PolyForm Shield licence ([LICENSING.md](LICENSING.md)).
Its code is written from Minecraft 26.3 itself and from public documentation. Since 30 October 2026
another project's source may also be read and used, **only when that project's license allows it**:

- **Allowed:** MIT, MIT-0, BSD-2-Clause, BSD-3-Clause, 0BSD, ISC, zlib, Apache-2.0, Unlicense, and the CC0-1.0
  public-domain dedication. That code may be read, copied or adapted.
- **Off limits, even to read:** GPL, LGPL (Iris), AGPL, MPL, EPL, Polyform (Sodium), the Creative Commons licenses
  (CC BY, CC BY-SA, CC BY-NC and the others; CC0 is a public-domain dedication, not one of them), custom or
  "source available" licenses, all rights reserved (OptiFine), and any project with no license or an unclear one.
  Don't read, copy, translate or paraphrase their source, and don't send code an AI tool wrote from it. Being able
  to download a mod or a pack does not give you the right to read its files.
- **Java's own library is GPL too.** OpenJDK's source (`java.lang.Math`, `StrictMath`, `FdLibm`, `HashMap`, the
  sorts, `Double.toString`, HotSpot's intrinsics...) is GPL-2.0, so it is off limits like the rest. Reproduce Java's
  behaviour from its API documentation (the Javadoc) and from what a JVM returns, or from a permissive original such
  as Sun's fdlibm C sources (see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
- **Check the license yourself.** Read the LICENSE file at the commit you use and the header of each file you take:
  some repositories mix licenses, and a file under another license is off limits. A license that changed: the one at
  your commit counts. In doubt, don't: ask in #contributors first.
- **Say it in the pull request:** the project, a link to the commit, its license, and what you read or copied. A
  pull request that used another project's source without saying so is closed.
- **Copied or adapted code** keeps its copyright line and license notice at the top of the file, and gets an entry
  in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Apache-2.0 code also keeps its NOTICE text and says what
  was changed.

User documentation and file formats are fine to read, and packs and mods are fine to run and observe, whatever their
license. Shader packs are a clean-room rewrite of the pack format, produced from a written description of it:
the [specification and its provenance log](docs/cleanroom/README.md) are public. They carry no foreign
source, and the rules above still apply to anything you read while working on them. A shader-pack pull request
names its sources: the sections of that specification, the documentation pages and the packs it ran.

## Vendor libraries and Java mods

- **Upscalers and latency tools.** AMD FSR (MIT) may go in the repository, like any code under an allowed license.
  NVIDIA's DLSS, Reflex and any other library under an NVIDIA license may not: they ship only as an optional native mod
  that downloads NVIDIA's files itself, on the player's machine. No NVIDIA binaries, headers or SDK files in the
  repository.
- **The JVM bridge** (running Java mods) is only ever a separate, optional download, never installed or on by
  default. The game may offer to turn it on when it finds a Java mod; the player decides. Promotion out of beta requires MSPT and RAM benchmarks against Java with the same mods.

## Before asking for review

Run the self-check and paste the block it prints into your pull request:
>>>>>>> sunrise/main

```sh
git config core.hooksPath .githooks
python3 tools/pr_ready.py
```

<<<<<<< sync-sunrise-1
It runs rustfmt, clippy with warnings denied, the tests of the crates you touched, the Python tests and the public-tree check. Keep pull requests small, and write commit messages in English.
=======
It looks at what your branch changes and runs what matters: rustfmt, clippy with warnings denied, the tests of the
crates you touched (and the launcher's), generated files, `Cargo.lock` and the Python tests. It runs the
vanilla harnesses locally only when your branch changes a harness, an oracle, a probe or the runner; otherwise it
lists the ones your change can affect. The daily checks the automatic harnesses after the merge. `--harnesses` runs them on your
machine anyway, and a change to mod content, assets, blocks or items also runs the `#[ignore]`d content-root tests;
both need the prepared vanilla data and are reported as not run without it. `--quick` never runs the harnesses.
It exits non-zero when something fails.
>>>>>>> sunrise/main

For the game workspace, Clippy checks all targets of the touched crates and their transitive workspace dependents, including examples. The dependency scope includes optional, platform, build and development dependencies. Tests run library, binary, integration and doc tests without building unrelated example executables. Shared Cargo configuration and toolchain changes select the whole workspace; CI also keeps its workspace-wide checks.

<<<<<<< sync-sunrise-1
Never commit game files, decompiled source or prepared resources (`.local/`): `tools/check_public_tree.py` rejects them (`git config core.hooksPath .githooks` runs it before every commit). The pre-push hook checks the full history reachable from the branches and tags being pushed; `python3 tools/check_public_tree.py --history` audits the history of all local refs.
=======
CI is split by how long a test takes and what it needs. Main and rollups run only quick checks and unit tests;
every test that boots the game runs once a day. `tools/ci/move_to_daily.py` moves an existing `verify` test that
needs prepared resources, a child process or the network.

| Tier | Runs on | What |
| --- | --- | --- |
| Push | every pull request push | `format`, `verify-static` |
| Gate | rollup and sync pull requests, pushes to main | `lint`, `verify` (unit tests only), `verify-static` |
| Daily | once a day on main's head | every test that boots the game or needs prepared resources, `fn-replace-check`, the SDK build and its linkage probe, the machine sample built against the mod API, the modding book's quickstart run with the mod tool alone, and vanilla parity on the main head captured at dispatch |

**A unit test** uses no prepared resources, boots no game, starts no child process, uses no network, and finishes in under 2 seconds. Any other test is a daily test: it carries `#[ignore]` and a row in
`tools/ci/ignored-tests/` (the format is in that folder's README). `verify` fails a unit test that takes over 5
seconds and `verify-static` one that boots the game.
In Linux `verify`, test binaries run normally under an inet guard: opening, binding or connecting an IPv4/IPv6
socket or calling libc name-resolution APIs (`getaddrinfo`, `gethostbyname`/`gethostbyname2`/`gethostbyaddr`, their reentrant
variants, or `getnameinfo`) fails the test by its full name, even if it handles the refusal. Unix sockets remain
allowed. Only binaries recording violations are rerun per test to name the culprits; an unreproducible violation
still fails by binary name. Move a network test with `tools/ci/move_to_daily.py`, using reason `network`; daily runs without the guard.

- A pull request is reviewed and merged on the push and gate tiers. Nothing else is asked of you.
- Write a quick test where a quick test can catch the mistake. A test that needs the game or prepared resources is
  welcome: mark it `#[ignore]` with its policy row, and say in the pull request that it has not run if you could not
  run it.
- If a daily run fails on a test your pull request added or touched, an issue is filed that names the test, where it
  panicked and the pull requests merged since the last green daily run. The fix is an ordinary pull request.

Three kinds of build: a **Dev build**, from a commit on main on request, needs only the gate tier and no in-game tests
(it is published as `dev-<date>-<short>`, installed only by explicit tag, to test packages and platforms after a
merge); the **nightly** is published by the daily run when every part of it is green; a **release** is cut from a
nightly.

Reviews here read the diff, the design and CI; they do not run your change. So for anything a player or a
mod author can see, please start the game once and try it (single player is enough:
`cargo build --profile fast -p sunrise-game`, or `sunrise-ctl` for agents) and say in one line what you did, under
"Tried in game" in the template. It is not a gate: "no, could not run it here" is an acceptable answer, and it tells
the reviewer where to look harder.

For the game workspace, Clippy checks all targets of the touched crates and their transitive workspace dependents,
including examples. The dependency scope includes optional, platform, build and development dependencies. Tests run
library, binary, integration and doc tests without building unrelated example executables. Shared Cargo
configuration and toolchain changes select the whole workspace; CI also keeps its workspace-wide checks.

For a quick check while you work, `python3 tools/check_changed.py` runs rustfmt, clippy and tests for only the crates your branch changed and the crates that depend on them (`--dry-run` lists the commands).

### Translations

Machine-written translations remain unreviewed until a speaker checks them. See [docs/translations.md](docs/translations.md). Never mark machine-written text as reviewed.

## Claim before you start

Work is tracked in GitHub issues, and an issue that is assigned is taken (so is one with the `in progress` label:
claimed on Discord with Kira's `/claim`): pick another, or ask its holder in the thread. Before you start, comment
**`/claim`** on the issue; you are assigned at once, no role needed. No issue for your work yet? Open one (a line is
enough), then claim it. `/unclaim` gives it back. Issues labelled `tracker`, `proposal` or `not claimable` aren't
claimed whole: claim one of the issues a tracker lists, or open one for the part you take.

A claim with no open pull request linking the issue (`Fixes #123` in the description, or a draft) is released after
5 days, with a comment: open a draft early. Agents claim the same way.

You can hold at most 5 claimed issues that no open pull request of yours links. A sixth `/claim` is refused with a
comment that gives the count; opening a pull request for one of them, or `/unclaim` on one, frees a slot. The
repository's collaborators, and the trusted contributors listed in the repository variable `CLAIM_LIMIT_EXEMPT`, are
not limited.

When your pull request changes the same functions as another open one, or closes the same issue, a bot comment names
it (edited on every push, never repeated): talk to its author in the thread before you both finish the same work.

## Pull requests

- **One change.** One concern per pull request (a hook, a handle, a feature or a fix), at whatever size it
  needs. There's no line limit; split by concern, never by size, and don't bundle unrelated changes.
- **Fill in the template**: what and why, the vanilla reference, the pr_ready block, screenshots for anything you
  can see.
- **We never push to your branch.** When something small is left (a lint, a rebase, a nit) we carry
  it in our own commits on top of yours in a rollup, and your pull request shows as merged with your commits
  intact; only design questions come back to you. You don't need to tick "Allow edits by maintainers".
- **Keep your branch yours.** You can push to it as you like. If you want to bring it up to date, `git merge
  origin/main` is the safe way; a rebase or force-push is fine too, since nothing of ours lives on your branch.
- **Title: `kind(area): what changes`**, so the queue reads at a glance: `parity(mobs): a ghast's fireball uses its
  ExplosionPower`, `perf(launcher): download the game's files in parallel`, `bug(saves): keep /attribute modifiers
  through a save`. The kind is one of the words below; the area is an area label, several with commas
  (`perf(chunks,rendering): ...`), or leave it out (`docs: ...`). Release notes drop the prefix.
- **Tick its kind** in the template: `parity` (Sunrise now does what vanilla does), `bug` (a crash or breakage),
  `perf`, `feature` (new for players), `extra` (Sunrise-only, off by default), `tooling` or `docs`. A workflow sets
  that label from the box, else from the title, and area labels (`launcher`, `mobs`, `worldgen`, `rendering`...)
  from the title and the paths you change.
  `label:parity` is the list of real progress toward vanilla.
- **Release labels** (we set them if you can't): `feature` also puts it under "New" in the release notes,
  `skip-release` when players see no change (tooling, docs, tests), nothing for a fix. `release-now` is for crash
  fixes that shouldn't wait. pr_ready suggests one.
- **`breaking`** when players or server owners must do something after they update: a save or settings format that
  changes, a removed feature, a changed command or `server.toml` key. Say what they must do under a
  `## Breaking change` heading in the description, for example "Set your render distance again in Options." Sunrise's
  Discord feed posts that text as a ⚠️ breaking-change notice, so write it for players. Mod API breaks don't need the
  label: the `migrate` line of their change note in `crates/sunrise-mod-vocab/changes/` says what mod makers do.

Who reviews and merges: the Sunrise Team, on CI for ordinary pull requests; risky ones (saves, network, world
generation, the launcher's updates) wait for a local proof first ([merging](docs/release-flow.md#merging)).

Merges reach players through the nightly, which the daily run publishes only when every in-game test passed
([release flow](docs/release-flow.md)).
How agents, checks, releases and signed updates keep this safe: [security](docs/security.md).

### Large pull requests

A change too big to review as one concern starts from a spec:

1. **Open an issue with the spec** before the code: what changes, the files it touches, the behaviour players and
   other code will see, and the tests or harnesses that prove it.
2. **A maintainer approves the spec** in the issue. Code written before that may have to change or wait.
3. **The pull request links the approved issue** (`Spec: #123` in the description) and stays within it; a change of
   plan goes back to the issue first.
4. **An agent check compares the code with the spec**, run by the maintainers' agents: anything the spec doesn't
   cover, or that it promises and the code doesn't do, comes back to you.
5. **Then the normal review**, as for any pull request.

## Parity checks

A pull request from a fork gets the push tier ([CI tiers](#ci-tiers)); the gate runs when a rollup carries it. Vanilla
parity never runs on a pull request or rollup. Run a relevant harness locally if you have prepared vanilla data;
the [daily](docs/release-flow.md#parity-on-main) checks the automatic harnesses after merge.

## Talk to us

[Discord](https://discord.gg/playsunrise) for questions and ideas, [issues](https://github.com/sunrise-org/sunrise/issues)
for bugs. To take something on, `/claim` its issue ([above](#claim-before-you-start)).

How Sunrise is run, and how people join the Team or lead a group: [GOVERNANCE.md](GOVERNANCE.md).

## Licence and DCO

Sunrise's code is under the licence in [LICENSE](LICENSE): PolyForm Shield 1.0.0 with the Sunrise Additional Terms,
version 1.0 or later. Your contribution comes in under that same licence, and you keep your copyright. There is no CLA. Instead, sign off every commit (`git commit -s`)
to certify the [Developer Certificate of Origin](https://developercertificate.org/): that you wrote it, or have the
right to submit it under the project's licence. The DCO check fails a pull request with a commit that isn't signed off;
`git rebase --signoff origin/main` fixes a branch.
>>>>>>> sunrise/main
