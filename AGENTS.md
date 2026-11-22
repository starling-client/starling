# Sunrise: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature: Sunrise behaves and looks bit-exactly like it on the same machine, while its code is free to differ
([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)). Read [CONTRIBUTING.md](CONTRIBUTING.md) once;
this page tells you where things are.
Feature → files → harness for the most-changed areas: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Never commit Mojang's files**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Keep your runs away from real player data.** Any game or launcher you start gets fresh `mktemp -d`
  folders in `SUNRISE_HOME` (logs, crash reports, telemetry consent, recordings; not saves), `SUNRISE_LAUNCHER_HOME`, `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, plus `SUNRISE_LAUNCHER_NO_SELF_UPDATE=1` and `SUNRISE_NO_AUDIO=1`. Saves, `options.txt` and
  caches go in the game folder, by default the working directory: start the game from a scratch folder with
  `--game-dir` pointing at it, or they land in the repo root, hidden by `.gitignore`. For the
  launcher, `SUNRISE_LAUNCHER_TRASH`, `SUNRISE_LAUNCHER_CLIPBOARD` and `SUNRISE_LAUNCHER_DESKTOP` pointing into the scratch home. Never write to
  `~/.local/share/{applications,sunrise,sunrise-launcher}` or `~/.local/bin`.
- **Mark what an agent posts.** A PR body starts with `> 🤖 Opened by <login>'s AI agent` and a blank line, where
  `<login>` is the GitHub account the PR is opened from (`gh api user --jq .login`). Issues say "Filed by", reviews
  "Review by". Every comment starts with `🤖 Agent:`. That line is the only mark: commits carry no
  `Co-Authored-By:` and PR bodies no "Generated with" line.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Sunrise-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- One concern per PR, at whatever size it needs; no line limit. Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges reach players through the
  nightly, published by the daily run when every leg is green ([docs/release-flow.md](docs/release-flow.md#daily-run)).
- **CI tiers** ([CONTRIBUTING.md](CONTRIBUTING.md#ci-tiers)): every PR push runs `format` and `verify-static`; rollups,
  syncs and main pushes add `lint`, `verify` (unit tests only) and `launcher`; the ignored-test shards,
  `fn-replace-check`, the SDK build and its linkage probe, the machine-sample build and the modding book's
  quickstart run once a day (`daily.yml`). A new test that needs prepared resources, boots the game, starts a
  child process, uses the network or takes 2 s or more is `#[ignore]`d with a row in `tools/ci/ignored-tests/`;
  say in the PR when you could not run it. A daily red becomes an issue listing the suspect PRs, fixed by an
  ordinary PR. `tools/ci/move_to_daily.py` moves a `verify` test that needs resources, a child process or the
  network. Builds: Dev (a commit of main, gate green; published as
  `dev-<date>-<short>`, which no launcher offers), nightly (published by a fully green daily run), release (cut from a nightly).
- **No hardcoded user-facing strings.** Keep UI translation-ready from the start. In the launcher ([sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher)), wrap visible text with `tr("...")` or `trf("... {arg} ...", &[("arg", &val)])` from `crate::i18n` instead of raw string literals or `format!`. In the game, use translation keys (`Text::tr`, `lang::get`), never hardcoded English strings.
- **Claim before you start**: comment `/claim` on the issue (no issue yet: open one, then claim it). An assigned
  issue is taken, and so is one labelled `in progress` (claimed on Discord): pick another. `/unclaim` when you stop;
  a claim with no open PR linking it is released after 5 days. The issues free to claim, without reading threads:
  `gh issue list --search 'is:open no:assignee -label:"in progress" -label:tracker -label:proposal -label:"not claimable" -linked:pr'`
- **Machine translations stay unreviewed.** `tools/lang_review/` records only text a person who speaks the language
  wrote or checked; never `lang_coverage.py --mark-reviewed` machine text ([docs/translations.md](docs/translations.md)).
- **Controllers reach what registers.** Draw controls with a registering helper (`ui::button`, `settings_shell::foot_button`); `graphics_style::button` and `screen_draw::button` only paint, and registering one part of a screen ends its crosshair, so register all of it.
- **Render passes keep HDR's light.** A pass that reads, filters or redraws the world's picture (upscaling,
  anti-aliasing, sharpening, frame generation, post effects, captures) follows [docs/hdr-passes.md](docs/hdr-passes.md).
- **Docs say where and how, not how many.** Point to where a thing lives and the command that lists or measures it.
  Don't write counts, growing lists, timings or current status into a doc: they go stale, and the next agent
  spends a turn fixing them.

## Mod API: the clean break

Sunrise (sunrise-org/sunrise) is where the native mod API is built: hooks, the `Host`, handles, Parts,
attachments and mod loading. The mod API is taking **one clean
break**, and every change to it follows these rules. Reviews block a PR that breaks any of them.

- **No compatibility paths.** No aliases, shims, deprecated names, migration or fallback code, version
  gates, or "kept for old mods" branches. Old mods rebuild; the compiler says what moved. Removing an old
  path updates every caller, the catalog, docs, tests and sunrise-machine-sample in the same change.
- **One hook per moment.** A hook with a NeoForge 26.3 counterpart takes its name, side and full contract.
  Two hooks at the same vanilla call site (including `_v1`/`_v2` pairs) merge into that one, and the old
  one is deleted, not kept beside it. A veto before an action and a notice after it are separate moments
  when vanilla runs code between them. A hook whose vanilla call site is gone is removed from the catalog.
- **Every hook gets a `Host`.** Hooks that fire inside client-lib split into decide, fire, apply: client-lib
  decides, the game fires the hook with a `Host`, then applies. The listener's writes and the apply run in
  one transaction, and the apply re-checks the decision's preconditions and refuses (never recomputes) if
  they changed.
- **`Host` shape.** A small shared `Host` (handle constructors, registries, settings, `transaction`), with
  `ServerHost: Host` and `ClientHost: Host` generated from each catalog entry's `side`, so a wrong-side
  call doesn't compile; a hook that fires on both sides gets a typed `HostSide`. Per-object methods live on
  handles (reads take `&dyn Host`, writes `&mut dyn ServerHost`), and the ID-based helper a handle method
  covers is removed when it lands. Drawing goes through a `DrawContext` given only to render and screen
  hooks. Items, fluids and energy go through Parts under transactions. Commands, players, messages and
  scheduling are small service traits reached from `Host`; attachments live on handles. Don't add a
  method to `Host` that has a natural receiver.
- **One dependency model.** Mods declare required and optional dependencies in `[dependencies]` in
  `sunrise-mod.toml`, using semver constraints on manifest versions. The game dispatches in dependency
  order. Native mods link through Cargo; a library mod can publish functions and statics to its
  dependents, which share one copy of the library.
- **Native binaries target the exact game build.** The loader checks the build ID a native mod was built
  against. `.smod` is the one package format for both native and content-only mods.
- **DRY.** One implementation per behaviour: reuse the existing helper, write path or generator rule, and
  let the catalog and generator own repetition instead of hand-written match arms.
- **SOLID.** Each module, trait and handle has one job; traits stay small and split by side and concern;
  new behaviour comes in through the catalog, handles and parts; mods depend on `mod_api`, never on game
  internals, and lower crates never reach up into `Game`.
- **Neat.** Names, layout and docs match the code around them. No dead code, leftover TODO paths, or
  anything a later PR must tidy up.
- **Fast.** No allocation, string building or scan over every mod on hot paths (tick, per-entity and
  per-block hooks). Dispatch only to the mods and parts that subscribe, resolve keys once at load, and a
  PR that touches a hot path states its tick-time effect.

## Don't lose time

Where agent time goes: reading code, compiling `sunrise-client`, waiting in loops, and work someone else had
already done.

- **Check nobody else is on it.** Before you write code, look at the issue's linked pull requests and run
  `gh pr list --state open --search "<issue number, function or file name>"`; then `/claim`.
- **Find code with the map, not by reading.** Start from the tables below and
  `python3 tools/port_ledger.py where Class.method`. Read the function you change and its callers, not whole
  files.
- **Compile cheaply while you work.** `cargo check -p <crate>` after an edit. Clippy, fmt and tests run once, at
  the end, through `pr_ready.py`. `cargo clippy -p sunrise-client` and `cargo build -p sunrise-client` are the
  slowest commands here: never after every edit, and build the game only when you need to run it.
- **Don't wait in loops.** Start a long command in the background and carry on, or wait for it once. No
  `sleep` or `until grep` polling of builds, harnesses or CI.
- **One test per changed behaviour.** None for a refactor or a file move, none that reads source text, and no
  harness where vanilla has nothing to compare (Sunrise's own features). Performance work is proven by the area's
  bit-exact harnesses (add one first if none covers the code) and a before/after benchmark.
- **A red check isn't always yours.** When `verify` fails in a test your change can't reach, say so in the pull
  request and move on; don't hunt it.
- **Unsure about a rule? Finish the work.** Take the reading that lets you finish, and say in the pull request
  which one you took. Don't stop, and don't change direction halfway.

## Crates

Game workspace (root `Cargo.toml`); `crates/sunrise-telemetry-server` is a separate workspace, and the launcher is its own repository, [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher).

| Crate | Owns |
|---|---|
| `sunrise-client` | Gameplay library (`src/lib.rs`: `game.rs` + `game/`, `entry.rs`) and thin executable (`src/main.rs`). Also the dedicated server (`--dedicated`). Re-exports `sunrise-client-lib` for existing mod paths. |
| `sunrise-client-lib` | Reusable client systems, `sunrise-ctl`, probes and their tests. Its manifest points to the existing sources under `crates/sunrise-client/`; the library root is `src/client_lib.rs`. It never depends on gameplay. |
| `sunrise-gameplay` | Vanilla's logical side, what the server jar holds (common code included), moved out of `sunrise-client` module by module ([#2210](https://github.com/sunrise-org/sunrise/issues/2210)). Holds the `MobLevel` service trait and the mob families on it (`allays.rs`, `foxes.rs`, `mob_looting.rs`), the cat spawner's clock and picks (`cat_spawner.rs`) and the wandering trader spawner's state (`wandering_trader.rs`), `mob_index.rs`, `trident.rs`, `stats.rs`, the block-entity ticker helpers (`block_entity_ticks.rs`), `item_areas.rs`, `interaction_shapes.rs`, `tick_profile.rs`, and the server-side halves of `lan/` and `dedicated/`. `LanHost`, `World`, `ChunkMap`, `Mob`, `ItemEntity`, the level-event dispatchers, the custom spawners' dispatch (`G/mobs/custom_spawners.rs`), the village siege (`G/mobs/raids/mod.rs`) and the block-entity tick loop (`G/blocks/block_entity_ticks.rs`) have not moved. Engine layer, not yet portable: `portable.pending` lets it take `sunrise-client-lib` until the types it needs move down. Never depends on `sunrise-client-gameplay` |
| `sunrise-client-gameplay` | Client-only gameplay moved out of `sunrise-client`: occlusion and entity culling (`occlusion_culling/`, `entity_occlusion.rs`, `particle_culling.rs`), `section_scene.rs` and `transient_blocks.rs`. Depends on `sunrise-client-lib` and portable crates, never on `sunrise-gameplay`. Engine layer, desktop |
| `sunrise-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `sunrise_client::items`... |
| `sunrise-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `sunrise_client::command` |
| `sunrise-client-core` | The client lib's bottom layer: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`sunrise_client::text`...) |
| `sunrise-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `sunrise-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings): its modules (`ls crates/sunrise-client-render/src`), which sunrise-client re-exports under the same paths. Nothing in it reaches gameplay |
| `sunrise-stage` | Prebuilt game staging, SDK surface export and comparison, and standalone mod source scans (`surface`, `surface-diff`, `uses`, `fits`) |
| `sunrise-mod-runtime` | Engine-independent mod attachment storage, query index and generic registry slot tables; depends only on `sunrise-core` |
| `sunrise-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage |
| `sunrise-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `sunrise-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `sunrise-assets` | Resource decoding: block-state and item models, baking, atlases |
| `sunrise-prepare` | Builds `.local/sunrise-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `sunrise-server` | `sunrise-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `sunrise-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `sunrise-java-properties` | `java.util.Properties` load/store and the JDK date text, standard library only; used by `sunrise-server-files` |
| `sunrise-friends`, `sunrise-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `sunrise-mod-package`, `cargo-sunrise-mod` | Content mods (`.smod`: manifest, content fence, no code), the registry index and `.spack`, and `cargo sunrise-mod` (`new`, `build`, `run`, `check`, `publish`, `registry`, `native-build`) ([docs/modding/](docs/modding/src/introduction.md)). Code mods are native, built by the launcher ([docs/modding/src/how-to/native-mods.md](docs/modding/src/how-to/native-mods.md)) |
| `sunrise-native-build` | Native mods compiled against a signed prebuilt SDK: the launcher's build, and `cargo sunrise-mod native-build`'s JSON contract that every `sdk-<build id>` ships ([docs/modding/src/reference/sdk/08-native-build.md](docs/modding/src/reference/sdk/08-native-build.md)) |
| `sunrise-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `sunrise-proxy` | Shared proxy selection and connection pools for CLI downloads |
| `sunrise-connected` | Connected textures whatever file declares them: the neighbour geometry, the format-neutral `Rule` (OptiFine's and mods' both fill it) and its `table` method. Read by `sunrise-client-render`'s `connected.rs` and the mesher, and by `sunrise-optifine`; a mod's files are `assets/<mod>/sunrise/connected/` ([docs/modding/src/how-to/client/connected-textures.md](docs/modding/src/how-to/client/connected-textures.md)) |
| `sunrise-optifine` | OptiFine-format pack files below every feature that reads them: `.properties`, the name/range/block matchers, paths and `optifine/` folders, and custom entity models (`cem/`: `.jem`/`.jpm` parsing and baking) |
| `sunrise-polytone` | Polytone-format pack files (`assets/*/polytone/`), read clean-room from the public wiki: targets, biome and dimension modifiers and their environment attribute layers |
| `sunrise-shader-api` | Sunrise's own WGSL shader packs ([docs/api/shaders.md](docs/api/shaders.md)): `sunrise-shaders.toml`, the `.sshader` package; shared by the game, the launcher and the registry |
| `sunrise-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `sunrise-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `sunrise-perf-format` | The `.hzperf` capture contract ([docs/perf-platform.md](docs/perf-platform.md) §3): `proto/sunrise/perf/v1/capture.proto`, its generated types, the gzip writer and its thread. No Sunrise dependencies; nothing in the game reads a capture |
| `sunrise-telemetry`, `sunrise-telemetry-server`, `sunrise-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `sunrise-addresses` | Every built-in address of an outside service vanilla doesn't contact (Sunrise layer; [docs/layers.md](docs/layers.md#built-in-addresses)) |
| `sunrise-system` | The machine the game runs on (OS, CPU, memory, drive, power, GPU maker and memory), for the crash report, the renderer's GPU info and telemetry |
| `sunrise-platform` | The platform's public API: what the Sunrise layer may use of the engine ([docs/layers.md](docs/layers.md)); re-exports `sunrise-system` today |
| `sunrise-hooks`, `sunrise-game`, `sunrise-official`, `sunrise-heartbeat` | The layer seam: the engine's hooks for crates above it, the game an executable starts, the official executable (shipped as `sunrise-client`) that registers Sunrise's in-game parts, the first of them ([docs/layers.md](docs/layers.md#how-a-sunrise-crate-plugs-in)) |
| `sunrise-reconnect` | Auto-reconnect: its switch, its policy and backoff, on the engine's connection hooks; registered by `sunrise-official` ([docs/extras.md](docs/extras.md)) |
| `sunrise-dualsense`, `sunrise-threads`, `sunrise-release-sign`, `sunrise-bots`, `xtask` | DualSense output, thread priorities, signed release checksums, load-test bots, repo tasks |
| [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) | GPUI launcher, in its own repository (moved there with its history, 2026-11-18): make launcher changes there. `launcher-bridge.yml` signs that repository's releases with this repository's release key and publishes them there and here as `launcher@X.Y.Z`, for launchers installed from here |

Other top-level folders: `tools/` (harnesses and dev scripts), `tests/` (Python unit tests: `python3 -m
unittest discover -s tests`), `mods/sunrise/src/test/java/dev/sunriselab/parity/` (Java oracles),
`docs/`, `.github/workflows/`. The website (playsunrise.net) is `frontend/site` in the private sunrise-org/sunny: its build
reads `README.md`, `docs/api/`, `roadmap/roadmap.json` and `crates/sunrise-theme/tokens.css` from main.

## Write tests with signal

- Test observable behavior, not words in docs, comments or source. Keep checks for machine-consumed contracts.
- Use independent expected values or a vanilla oracle; do not derive the answer from the code under test.
- Test outcomes, not only calls to stubs. Use mocks to isolate behavior, not as the behavior itself.
- Skip tests of trivial constants, derives and getters, or behavior already covered by another test.
- One test per changed behaviour is enough. A refactor or a move that changes no behaviour gets no new test.
- Avoid pinning implementation details; a harmless refactor should not break a behavior test.
- Make slow tests faster without removing cases or weakening their coverage.
- Never delete parity harnesses or tests that pin vanilla behaviour: they are the spec.
- No workarounds: when a harness differs, fix the cause until it matches bit for bit; never add a tolerance, rounding,
  a special case or a compensating patch to pass it. Removing one, from the code or a harness, is welcome while the
  harnesses stay bit-exact: smaller code is a goal.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # the self-check: fmt, clippy -D warnings, touched crates' tests, harnesses; paste its block in the PR
python3 tools/pr_ready.py --quick          # same without harnesses
cargo test -p sunrise-client -p sunrise-game -p sunrise-client-lib --lib --bins --tests   # skips the examples (pr_ready does this); `--doc` separately
cargo test -p sunrise-client-lib --lib mod_content::tests:: -- --ignored   # the slow content-root tests, #[ignore]d; pr_ready runs them when mod content, assets, blocks or items change
cargo build --profile fast -p sunrise-game        # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # sunrise-ctl / MCP end to end
python3 tools/lang_coverage.py --summary   # one line per language: how many keys it carries and what it misses
python3 tools/lang_coverage.py --summary --show-keys   # the same, then the keys behind the counts
```

- GPU tests share `gfx::test_gpu::lock()`: bind the guard before creating GPU objects so it outlives
  every resource, queue, device and instance, including GPU-owning workers (join them before unlocking).
  Helpers returning GPU objects leave the guard with their caller; helpers returning only CPU data may
  own it themselves, without another outer guard. CPU-only tests need no guard. Run renderer coverage
  with `cargo test -p sunrise-client-render --lib -- gfx::`; no global serial-test flag.

- For a player-facing change, drive the game once with `sunrise-ctl` (launch, the action, a `state` or `screenshot`)
  and report the result in the PR's "Tried in game" line; never claim it when it was not run.
- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `sunrise-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI.
- A harness = `tools/test_<name>_parity.py` (`python3 tools/parity.py --all --list` names them) + a Java oracle (`mods/sunrise/src/test/java/dev/sunriselab/parity/Vanilla*Oracle.java`)
  + a Rust probe: a module `crates/<crate>/examples/probe/<name>.rs` of that crate's probe binary (`ls crates/*/examples/probe`),
  or the game binary with a fixture env var (`game/interface/dispense_fixture.rs`, `game/blocks/sculk_fixture.rs`,
  `game/administration/command_harness.rs`, `game/network/net_harness.rs`). Copy a nearby harness for a new one.
- Native mods compile against a pinned prebuilt SDK; `[[native.mixin]]` rows compile into a function-replacement patch library.
  Use `native-build --trust-native-code`; the launcher never rebuilds the game for a mod.
- Probes build into `.local/parity-target/`; harnesses find binaries via `lab.cargo_target()`, never `target/`.
- Harness selection uses Cargo dep-info and reachable-code narrowing (`tools/parity_reach.py`).
  `sunrise-client-lib` builds the control tool and client probes without gameplay; their dep-info
  excludes `game.rs` and `game/`, so gameplay edits do not rebuild or select those probes.
  Source paths remain in `crates/sunrise-client/`. Measure actual harness selection on refreshed
  records with `tools/parity.py --changed-files paths.txt --list`; historical counts are not a guarantee.
- Rust integration tests: `crates/sunrise-client/tests/` (`ls crates/sunrise-client/tests`).
- Linux needs `libasound2-dev libudev-dev`; `--no-default-features` drops audio, controllers, `discord`, `friends` and `heartbeat` (the server build).
- Run and drive the game (starts the `sunrise-client` beside `sunrise-ctl`; other arguments go to the game):
  `target/fast/sunrise-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/sunrise-client/26.3`,
  then `sunrise-ctl --socket … state|keys|turn|command|screenshot|quit`
  ([docs/sunrise-ctl-commands.md](docs/sunrise-ctl-commands.md)).

## Vanilla reference

- Start parity work from the port ledger, not grep: `python3 tools/port_ledger.py issue N` (or `where
  Class.method`, `what file:line`), and `check --base origin/main` before the PR. How:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md), [docs/port-ledger.md](docs/port-ledger.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode queries (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
- A harness runs the oracle against the real 26.3 jars and the Rust probe on the same inputs, then compares
  their JSON. Oracle runs are cached by `tools/parity_shim.py`; `--fresh` reruns them.
- Match vanilla's `Math` results with `sunrise_core::jmath`, and vanilla's RNG with `sunrise_core::random`:
  random draws must happen in vanilla's order, in performance work too ([CONTRIBUTING.md](CONTRIBUTING.md#vanilla-first)).

## sunrise-client: where things live

`L:` = lib (`crates/sunrise-client/src/`, or `crates/sunrise-client-render/src/` for the modules that crate holds, under
the same `sunrise_client::<module>` paths), `G:` = gameplay (`crates/sunrise-client/src/game/`), also in the lib in normal builds.
Gameplay keeps its `sunrise_client::<module>` imports through a self-crate alias.
Gameplay code that needs no `Game` lives in the side crate of its side: `crates/sunrise-gameplay/src/` (SG) for the
server's logic, `crates/sunrise-client-gameplay/src/` (SCG) for client-only code. `G:` keeps the `impl Game` glue
under the old names, and the two side crates never depend on each other (the `sides` in `tools/layers.json`). The executable
contains only the allocator, mod API marker and a call to `entry::main`. For unit tests, the lib
excludes `entry` and `game`, and the executable test target owns `game` so its process-wide test
state stays separate from the library tests. This compiles gameplay twice for executable tests.

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | `game.rs`: `fn run`, `impl ApplicationHandler for Game` (`window_event`, `about_to_wait`), `fn tick`, `fn render`; G: `world/level_tick.rs` (`fn tick_world`, `fn tick_mobs`); L: `clock.rs`, `spans.rs`; `sunrise-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::new`, `Renderer::render`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `render_tick.rs`, `item_draw.rs`, `screen_backgrounds.rs` (panorama, menu blur, border wall) |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs` (binds sunrise-world's generator), `lod.rs` + `lod/`, `server_terrain.rs` (a server's kept chunks), `bluemap.rs` (far terrain from its web map), `ticket_levels.rs`; G: `world_loading.rs`, `world_entry.rs`, `postprocess.rs`; game.rs `fn update_world` |
| World generation | `crates/sunrise-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/sunrise-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); game.rs `fn tick_blocks`, `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `creatures.rs` (ticking, spawning, mob events), one file per mob family (`dragon.rs`, `wither.rs`, `villagers.rs`, `piglins.rs`, `golems.rs`...), `natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `arrows.rs`; G: `boats.rs`, `minecarts.rs`, `mounts.rs`, `armor_stands.rs`, `paintings.rs`, `frames.rs`, `fireworks.rs`, `tnt.rs`, `explosion.rs` |
| Player actions | game.rs (`fn use_hand`, `fn use_item`, `fn attack`, `fn tick_mining`, `fn break_block`); G: `hand.rs`, `combat.rs`, `consume.rs`, `using_item.rs` (an item in use, tick by tick), `potions.rs`, `shield.rs`, `crossbow.rs`, `trident.rs`, `fishing.rs`; L: `motion.rs` + `motion/` (movement), `collisions.rs`; crate `sunrise-items`: `items.rs`, `item_data.rs`, `enchant.rs`, `blocks.rs` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; game.rs `fn menu_ops`, `fn click_inventory`; G: `stations.rs`, `trading.rs`, `recipe_book.rs`, `machines.rs` |
| Commands | parse: crate `sunrise-command` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); run: G: `commands.rs` (dispatch) + `commands/*.rs` (`admin`, `execute`, `items`, `players`, `world_cmds`...); L: `scoreboard_cmd.rs`, `data_cmd.rs`; `sunrise-client-core`: `nbt_path.rs`, `scoreboard.rs`; G: `functions.rs`, `command_blocks.rs` |
| UI screens | `Screen` enum in game.rs; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `settings_shell.rs` (Sunrise's settings screens on it: `extras_ui.rs`, `graphics_ui.rs`, `controller_ui.rs`), `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `hud_kit/` (Sunrise overlays' `Room` and `Plate`, the collapsed page strip `tabs` that Shaders' screens use, [docs/ui-kit.md](docs/ui-kit.md)), `pad_glyph.rs` (controller glyphs, by what the button does), `chat.rs`, `tab_list.rs`; G: `startup_menu.rs`, `loading_screen.rs` (the loading screen, the pre-game `App`), `screens.rs` (pause, death), `options.rs`, `lan_screen.rs`, `chat.rs`, `sign_edit.rs`, `books.rs`, `creative_screen.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `network/remote/mod.rs` (`Game::apply_packet`), `network/remote/packets_*.rs` (packet handlers), `network/remote/tick.rs` (`Game::tick_remote`), `remote_*.rs` (mob data, objects, projectiles, vehicles, stations) |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `lan.rs` (`LanHost`), `network/lan_actions/` (`turns.rs`: `fn as_guest`, `tick.rs`: `fn tick_guests`), `lan_*.rs` per feature, `lan_entities.rs` (tracked entities) |
| Integrated server | G: `integrated.rs` (the server thread), `levels.rs` (one level per dimension, `fn tick_server`), `server_travel.rs` |
| Dedicated server | G: `dedicated.rs`, `dedicated/status.rs`; crates `sunrise-server-files`, `sunrise-server` |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `sunrise-client-core/src/block_entity_nbt.rs`; G: `saving.rs`, `persist.rs` (entities), `world_session.rs` (opening and closing a world); `crates/sunrise-world/src/{region.rs,datafix/}`, `crates/sunrise-core/src/nbt.rs` |
| Input | game.rs `fn window_event`; G: `play_keys.rs` (`fn key_pressed`), `play_input.rs` (`fn play_mouse`, toggled keys, the hotbar wheel); crate `sunrise-client-input` (`keybinds.rs`, `keymapping.rs`, `key_names.rs`, `mouse.rs`, `input.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`); L: `osk.rs`; G: `pad.rs`, `pad_menu.rs`, `pad_craft.rs`, `pointer_capture.rs` (mouse look, pointer grab) |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `music.rs`, `subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`, `mobs/cem.rs` + `mobs/cem_anim.rs` (OptiFine/EMF entity models and their animations); `sunrise-client-core/src/lang.rs`; crates `sunrise-assets`, `sunrise-optifine`; G: `packs.rs` |
| Agent control | L: `control.rs` (socket), `bin/sunrise-ctl/`; G: `control.rs`, `agent.rs`, `events.rs`, `ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crate `sunrise-mod-package`; G: `modding/mods/mod.rs` (content and mod load), `modding/mods/standalone.rs` (standalone model registration); L: `mod_content.rs`, `mod_list.rs` |
| Sunrise extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`; G: `interface/extras.rs` + `interface/extras/` (each extra's glue: `mouse_tweaks.rs`, `sorting.rs`, `pvp.rs`), `options.rs` |

## How `Game` is organised

- A module that needs no `Game` moves to `sunrise-gameplay` or `sunrise-client-gameplay` (#2210), and its `game/` file
  keeps the `impl Game` glue under the old name. A service trait (`MobLevel`) carries a family's
  level calls, statically dispatched because it runs per mob per tick. `tools/layers.json` forbids any dependency
  between the two side crates.
- `struct Game` (game.rs) keeps the core loop and fields. Each `game/<area>/<feature>.rs` (or
  `<feature>/mod.rs`) adds an `impl Game<'_>` block and state types. Logical modules remain direct
  children of `game`: `game.rs` declares each with a `#[path]`, so an area folder is not a module.
- Find a feature's code: `grep -rn "fn tick_<thing>\|fn <verb>_<thing>" crates/sunrise-client/src/game*`;
  the file is under `game/<area>/`. Find its callers in game.rs's `fn tick` and `world/level_tick.rs`'s `fn tick_world`.
- Fields that say which mode runs: `remote` (this window is a client of a server: multiplayer, LAN guest, or
  single player on its server thread), `lan` (`LanHost`: this world is open to the network), `integrated`
  (the window's own server thread), `serving` (this `Game` *is* that server thread), `options.dedicated`.
  `fn tick` returns `tick_remote()` when `remote` is set; a server ticks its levels (`fn tick_world`).
- A server's players run through the shared gameplay code: `as_guest` swaps the guest's
  state (position, inventory, mode, mining, item use) into `Game` for one call (`game/network/lan_actions/turns.rs`), and
  `levels.rs` swaps whole dimensions in the same way. Changes reach clients through the block log, level
  events and the entity tracker (`lan/tracking.rs`, `game/network/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in game.rs, their methods in `game/resource_tables.rs`.

## Single player runs on a server thread

`game/world/integrated.rs` runs a second `Game` (`serving`) on a "Server thread" with one level per dimension;
the window joins it as a client through memory (`remote`), exactly as a LAN guest or a dedicated server's
player does. A window's `Game` never ticks a world itself: the old in-frame path is gone
(`--in-frame` and `--no-server-thread` only print a notice; `SUNRISE_SERVER_THREAD` is ignored).
Open `[server-thread]` issues list the gaps left.

Single player, LAN guests and dedicated servers all run the server code, so a fix there serves all three.
When you change how the server and its clients talk (packets, tracking, guests, the server thread itself), check
it there. Other gameplay fixes are covered by their own harness and CI's parity job:

```sh
cargo run --profile fast -p sunrise-game -- --resources .local/sunrise-client/26.3 --world T               # server thread
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest is a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, sunrise-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/rust-parity.md](docs/rust-parity.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
