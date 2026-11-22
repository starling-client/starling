# Sunrise documentation

To play, use [the launcher](https://github.com/sunrise-org/sunrise-launcher/blob/main/README.md); the project site is [playsunrise.net](https://playsunrise.net). These pages are for developers. What is being done now and next, the work board and how to help: [the hub](https://playsunrise.net/hub) ([ROADMAP.md](../ROADMAP.md), [roadmap/README.md](../roadmap/README.md) for keeping it current).

## Build and run

- [AGENTS.md](../AGENTS.md) and [Architecture](architecture.md): where each part of the code lives, how `Game` is organised, which harness checks which area.
- [How Sunrise is built](how-sunrise-is-built.md): who writes Sunrise, how changes are checked, and what is safe to run.
- [Release flow](release-flow.md): pull request checks, merging and the automatic Nightly releases that reach players.
- [Operations](operations.md): running Sunrise by hand when no agent runs and the maintainer's machines are off: merging, releases, offline runners, the mods registry, Kira, the official server, security.
- [Build from source](native-client.md#run-from-a-checkout): your own copy of the game, resource preparation, `cargo run`.
- [Build times](build-times.md): `tools/build_times.py` measures cold and incremental builds per crate and the critical path, writes a JSON report and a table, and compares two reports; the first numbers, Windows and Linux.
- [Native client](native-client.md): options, controls, the dedicated server, `sunrise-ctl` scripting, packaging, architecture.
- [Controllers](controllers.md): playing with PlayStation, Xbox and Nintendo Switch Pro controllers, Steam Deck built-in controls, analog movement, and the input layer.
- [Host a server](hosting.md): install `sunrise-server`, prepare the game files, first run, `server.toml`, the commands, the web panel, backups, services and Docker; what works today and what doesn't yet.
- [A server's levels](server-levels.md): how the dedicated server and single player's server thread tick, save and travel between dimensions.
- [Nix and NixOS](nix.md): the flake's packages: server, engine, prepare, client and launcher, and the `services.sunrise-server` NixOS module.
- [Hosting a server: design](server.md): `sunrise-server`, the design and the plan: install, `server.toml`, the CLI, the web panel, backups, services, Docker, and the bot benchmark against vanilla.
- [The next server: design](server-next.md): Sunrise's dedicated server against vanilla and Pumpkin (measured), the targets, parallel ticking that keeps vanilla's order, the network front, the Sunrise fast path, the plugin API and the plan.
- [Survival multiplayer](smp.md): the official server's `smp` native mod (economy, market, trades, games of chance, spawn, homes, teleport requests, warps, land claims) and the server hook points it needs; not part of Sunrise's own server.
- [For server owners](owners.md): every `server.properties` key next to its `server.toml` field.
- [Agent control](agent-control.md): what AI agents can drive in the launcher and the game, and the design of the control socket, `sunrise-ctl` and MCP.
- [sunrise-ctl commands](sunrise-ctl-commands.md): every command `sunrise-ctl` takes, its CLI arguments, and the corresponding MCP tools for driving the game.
- [Graphics](graphics.md): the graphics settings screen (pages, search, descriptions), the graphics benchmark and what it measured.
- [Memory](memory.md): what holds the game's memory (`sunrise-ctl memory`, the `alloc-census` heap census and its measured cost, resident memory in benchmark reports, heap profilers on probes).
- [HDR and render passes](hdr-passes.md): what a pass on the world's picture (upscaling, anti-aliasing, frame generation, post effects, captures) keeps so HDR works, and how to check it.
- [Realms availability](realms-availability.md): vanilla's check that this client may use Realms, the five results, how a screen reads it (one `Pending`, kept), its caching, and how to test a consumer.
- [Server against Pumpkin](performance/server-vs-pumpkin.md): Sunrise's server and Pumpkin (Rust) under the same bots at 0, 20, 60 and 100 players: tick time, CPU, memory, chunks, join; where Sunrise loses, and how to run it again.
- [Benchmark](benchmark.md): Sunrise against vanilla 26.3 and vanilla with performance mods, on the same machine, world, path and settings: what is measured, how it is kept fair, how to reproduce it.
- [Extras](extras.md): optional conveniences (fullbright, dynamic lights, inventory controls, auto-reconnect), all off by default.
- [Forking](forking.md): this fork's permanent extras exclusions, independent build features and always-available mod management.
- [PvP & HUD](pvp.md): optional client settings for PvP players (custom crosshair, block outline, hit color, HUD widgets), fair-play boundaries, and why each is off by default.
- [Narrator](narrator.md): vanilla's text-to-speech (Ctrl+B): the voices per platform, how screens record what they narrate, `sunrise-ctl narration`, and where it still differs from vanilla.
- [Shader packs](shaders.md): Sunrise's own WGSL shader packs, how to make and check one (`sunrise-shader`).
- [The modding book](modding/src/introduction.md): content mods (data) and native mods (Rust), how to make, install and publish them. [Native mods](modding/src/how-to/native-mods.md): how the launcher builds them. [Hook counterparts](https://sunrise-org.github.io/sunrise/reference/hook-notes.html#counterparts): the NeoForge and Fabric catalog conventions.
- [Upgrading a 26.2 data pack](datapack-upgrade.md): `sunrise-port-pack upgrade-datapack` rewrites a 26.2 worldgen pack in 26.3's formats, and how the rewrite is checked against vanilla.
- [Modpacks](modpacks.md): porting whole packs (ATM10 and smaller): import, inventory, carry-over, an agent per mod with a harness against the Java original, coverage, and how "10x" is measured.
- [Launcher](https://github.com/sunrise-org/sunrise-launcher/blob/main/README.md): installs, updates and starts builds; its CLI and MCP server; releasing it.
- [Versioning](versioning.md): how releases are numbered from 0.2 (`v0.2.x`, `launcher@0.2.x`), and how installs from before 0.2 get there.
- [Resource packs](resource-packs.md): everything a 26.3 resource pack can change, whether Sunrise honours it, and the proof.
- [Sunrise sends nothing](telemetry.md): no telemetry; crash, benchmark and bug reports stay local files.
- [Benchmark report format](benchmark-report.md): the versioned JSON every benchmark writes (game, launcher, `tools/perf_bench.py`) and the local log.
- [Hot-path performance trends](hot-path-trends.md): the main-only job that counts instructions and allocations of the tick, commands, chunk saving and mod dispatch on fixed workloads, and where its trend page is.
- [Security](security.md): how Sunrise protects code, CI and automatic releases against untrusted input while work is performed by AI agents.
- [Known issues](known-issues.md): player-facing issues listed in each game release and removed as pull requests fix them.
- [Sunrise APIs](api/README.md): every surface other programs build on (mods, shaders, agent control, servers, the launcher, data formats), each with its guide, generated reference, stability and changelog, and the [stability and versioning policy](api/stability.md).
- Menus: every screen is vanilla's (title, world list, create world, multiplayer, options, pause), except Sunrise's graphics settings, drawn in the design system's Blocks palette. Sunrise's own settings hang off vanilla's Options the way mods add theirs, each in one place: **Video Settings...** opens Sunrise's graphics settings screen (every video option, the graphics extras and the shader packs), **Controls → Controller Settings...** the controller settings screen in the same style, **Mods...** and **Sunrise Settings...** take the last two slots of the Options grid (Sunrise Settings opens Extras, PvP & HUD, Replay Studio and AI Agent, each a vanilla options list). The title screen's and pause menu's Friends buttons open Join a Friend and Play with Friends ([play-with-friends.md](play-with-friends.md)).

## Launcher and platforms

- [The launcher's mod browser](mod-browser.md): searching Modrinth and the Sunrise registry for mods, modpacks, resource packs and shader packs, compatibility checks before install, and checksum verification.
- [Launcher commands](https://github.com/sunrise-org/sunrise-launcher/blob/main/docs/launcher-commands.md): every command of `sunrise-launcher`, CLI arguments, and the tools `sunrise-launcher mcp` offers for driving launcher actions.
- [Install a local unsigned game build](developer-install.md): package a checkout as a developer base.
- [Launcher resource preparation](launcher-preparation.md): compiling Java exporter classes into release bundles, verifying local Minecraft assets against `research-lock.json`, and multi-lane export workflows.
- [Microsoft account sign-in](microsoft-account.md): signing players in with their Microsoft account in the launcher, joining online-mode servers, the owner gate, and the OAuth device-code flow.
- [Windows allocators](windows-allocators.md): mimalloc defaults, the optional `windows-segment-heap` Cargo feature for Win32 heap APIs, and how to validate the executable manifest.
- [Windows ARM64](windows-arm64.md): building and running native Windows ARM64 executables for `aarch64-pc-windows-msvc`, cross-compilation, and release packaging.

## Parity

- [Rust rewrite status](rust-parity.md): where the state of the port lives (harnesses, port ledger, known issues) and the rules every port keeps.
- [Parity workflow](parity-workflow.md): `tools/parity.py`, the gate, other machines, merging.
- [Paper event parity](paper-parity.md): local trace comparisons, scenarios and tolerances, and informational nightly/manual CI.
- [CI runners](ci-runners.md): which jobs run on GitHub's runners and which on ours (the desktop, playsunrise, the Mac mini), the `SELF_HOSTED_CI` switch, and where a fork's pull request may run.
- Area notes: [block behaviours and ticking](parity/blocks.md), [section meshing](parity/meshing.md), [redstone machines tick by tick](parity/redstone-diff.md), [the same seed in vanilla and Sunrise](parity/world-compare.md).
- [Port ledger](port-ledger.md): `tools/port_ledger.py`, every vanilla method next to the Rust functions that port it, both ways, with the gaps where a port isn't wired like vanilla.
- [Version ports](version-ports.md): how a new Minecraft release is detected, diffed against the pinned one and ported, and what stays manual.
- [Reading vanilla](parity-workflow.md#reading-vanilla): the local decompiled sources and bytecode queries the harnesses build on.

- [Translations](translations.md): language coverage and human review of machine-written strings.

## Design notes

- [Design system](design-system.md): shared tokens and interface rules for the launcher, website and Sunrise-only game overlays.
- [HUD kit](ui-kit.md): the pieces Sunrise's own HUD overlays share: `Room`, which keeps controller notices below the boss bars, and `Plate`, the box behind their text.
- [Linear light for HDR and shader packs](design/hdr-linear-light.md): the proposed shader API 0.2 colour contract and frame format for HDR output.
- [The shader API 0.2 frame](design/shader-api-frame.md): proposed: where a 0.2 pack's stages, post chain and tone curve run, who owns anti-aliasing and coverage, and room for clouds, mods, LabPBR and ray tracing.
- [Live mod reloading](design/mod-hotload.md): loading, enabling and updating mods from inside the game without a restart.
- [Mod registry v2](design/registry-v2.md): project kinds, metadata, installation locations and the registry rollout.
- [Scoped whole-stack rollback](design/stack-snapshot.md): transaction-scoped, same-place savepoints for internals mods; the real transaction lifetime must be confirmed before implementation, and provider-owned places are initially refused.
- [Modularizing gfx.rs](design/gfx-modularization.md): the move-only extractions out of the renderer's 13,837-line module, what each phase took, and the blocks still in `gfx.rs`.

## Architecture: world generation

- [World generation](world-generation.md): the pipeline from noise to features. Each stage's vanilla comparison is its harness (`tools/test_<name>_parity.py`); `python3 tools/port_ledger.py where Class.method` finds the Rust port of a vanilla method.
- [World height](world-height.md): each level's shape from its dimension type, data packs' taller levels.
- [Pregeneration](pregeneration.md): `/pregen`, generating, lighting and saving an area ahead of the players.

## Architecture: world data and simulation

- [World data](world-data.md): chunk storage, palettes, region files.

## Java comparison profiling

- [JFR analysis for Java comparisons](profiling.md).
