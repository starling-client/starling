> [!IMPORTANT]
> **Discord:** join Sunrise: [https://discord.gg/eswQcHfRKC](https://discord.gg/eswQcHfRKC).

<p align="center">
  <img src="https://img.shields.io/badge/Minecraft-Java%2026.3-5D9C3F" alt="Minecraft Java 26.3">
  <img src="https://img.shields.io/badge/Rust-stable-B7410E?logo=rust&logoColor=white" alt="Rust">
  <a href="https://github.com/ludovic111/sunrise"><img src="https://img.shields.io/badge/fork%20of-Sunrise-F0795A" alt="Fork of Sunrise"></a>
  <img src="https://img.shields.io/badge/platform-Android-2A2470" alt="Android">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3B2F7E" alt="MIT license"></a>
  <a href="https://discord.gg/7xcp8Y5nkv"><img src="https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
</p>

<p align="center">
  <a href="#download"><b>Download</b></a> ·
  <a href="https://discord.gg/7xcp8Y5nkv"><b>Discord</b></a> ·
  <a href="#android-status"><b>Android status</b></a> ·
  <a href="roadmap/starling.md"><b>Roadmap</b></a> ·
  <a href="#mods"><b>Mods</b></a> ·
  <a href="#build-from-source"><b>Build from source</b></a> ·
  <a href="CONTRIBUTING.md"><b>Contributing</b></a> ·
  <a href="https://github.com/ludovic111/sunrise"><b>Sunrise</b></a>
</p>

# Starling

**Starling, formerly Sunrise Android, is a lightweight port of Minecraft: Java Edition 26.3 to Android.** Co-founded by [JuanLuci](https://github.com/JuanLuci), founder of Starling, and [Vasysik](https://github.com/Vasysik), founder of Sunrise Android. It is a fork of [Sunrise](https://github.com/ludovic111/sunrise), the Java game rewritten in Rust: it reads the same worlds, follows the same rules and generates the same terrain as Java Edition, and is checked against it by differential harnesses. You play it on the files from your own copy of Minecraft; it is free, open source and unofficial. And it runs natively, with no Java virtual machine and no OpenGL translation in between, so it starts fast and plays smoothly on a phone.

<<<<<<< sync-sunrise-1
> Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft. Starling contains no Minecraft code, assets or data: you need your own legally obtained copy of Minecraft: Java Edition 26.3.

## Download

**Join us on [Discord](https://discord.gg/7xcp8Y5nkv)** for help, bug reports and news.
=======
**Minecraft: Java Edition 26.3, rewritten in Rust.** Your worlds, the same rules and the same terrain as the Java game, checked against it by hundreds of differential tests, in one native program with no Java. Free, public source and unofficial: you play it with your own copy of Java Edition.

<p align="center">
  <img src="docs/media/readme/valley.webp" alt="A river valley with a plains village, savanna hills and a cherry grove, seen from above with a long view distance" width="100%">
</p>

## What works today

Everything below is in the current release.

- **The game.** Survival, creative, hardcore and spectator; vanilla world generation in all three dimensions, mobs, villagers, raids, redstone, every command, the HUD and the recipe book. Vanilla saves open as they are, and worlds from 1.8 to 26.2 are upgraded.
- **Multiplayer.** Join vanilla and Sunrise servers (online mode included), open a world to LAN, or invite friends with a code from the launcher's Home page.
- **Graphics.** Distant terrain up to 1,024 chunks, graphics presets, HDR and controllers, all off by default where they would change the vanilla look ([settings](https://playsunrise.net/docs/settings)).
- **Packs.** Resource packs, data packs and shader packs ([below](#mods-and-shader-packs)).
- **Mods.** Content mods and native Rust mods, installed from the launcher ([below](#mods-and-shader-packs)).
- **Servers.** Host one from the launcher, or run `sunrise-server` on a VPS ([below](#host-a-server)).
- **The launcher.** It installs and updates the game, prepares its files from your copy of Minecraft, and updates itself.

<table>
  <tr>
    <td width="50%"><img src="docs/media/readme/sunset.webp" alt="Sunset behind a cherry grove on a hill, with a river below"></td>
    <td width="50%"><img src="docs/media/readme/nether.webp" alt="A crimson forest in the Nether, with a piglin among the fungi"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/media/readme/survival.webp" alt="Survival in a village square: two villagers, flowers, the hotbar, health and hunger"></td>
    <td width="50%"><img src="docs/media/readme/multiplayer.webp" alt="Two players in a LAN world: the other player's name tag and the player list"></td>
  </tr>
</table>

<sub>Nightly 2026-11-05 on a Mac mini (Apple M4 Pro, Metal): vanilla's textures, no mods or shader packs, render distance 32 with distant terrain on.</sub>

## Performance

**Sunrise draws more frames per second than Java Edition, and loads faster, on the same machine, world and path.** `tools/perf_bench.py` runs each game in turn, from a fresh world (seed 1234), waiting for the terrain and then flying it over a village, a forest, an ocean and a mountain. Measured on 2026-10-29/30 on an idle Windows 11 machine (Ryzen 7 9800X3D, RTX 5080), Sunrise v0.2.97 as the launcher installs it, 1280x720 in a window, VSync off, three games per figure. Frames per second, mean of the four scenes, with the slowest 1% of frames in brackets:

| Render distance | Sunrise v0.2.97 | Vanilla 26.3 | Vanilla + mods |
| --- | --- | --- | --- |
| 8 chunks | 3,289 (1,327) | 2,437 (640) | 2,375 (709) |
| 16 chunks | 2,074 (598) | 1,661 (375) | 1,586 (432) |
| 32 chunks | 1,207 (333) | 1,031 (176) | 879 (264) |

"Vanilla + mods" is Sodium, Lithium, FerriteCore and ImmediatelyFast. Sunrise reaches the title screen in 2.9–3.2 s against vanilla's 7.5–7.7 s (the modded client's 10.8–11.1 s), loads a scene's chunks 2.9–3.8 times faster, and playing holds a steady working set of 1.1–1.7 GB against 1.9–2.9 GB. Every scene with its confidence interval and the run's load log: [the 2026-10-29 reports](benchmarks/perf/windows-9800x3d-5080-2026-10-29.md). Method, and how to measure your own machine: [docs/benchmark.md](docs/benchmark.md). [playsunrise.net/benchmarks](https://playsunrise.net/benchmarks) puts these runs beside a 2016 laptop, a Mac mini and a gaming desktop.

## Download

Get **the Sunrise launcher** from [playsunrise.net/download](https://playsunrise.net/download), or from GitHub: it has its own repository, [sunrise-org/sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher), whose releases are also published here as `launcher@` releases ([1.0.0](https://github.com/sunrise-org/sunrise/releases?q=launcher&expanded=true) came from here):
>>>>>>> sunrise/main

**[Starling v0.8.0](https://github.com/starling-client/starling/releases/latest)**. Already have Starling? Tap **Update** inside the game. New here? Download `starling-0.8.0-arm64-v8a.apk` from [the releases page](https://github.com/starling-client/starling/releases) and open it on the phone (Android asks you to allow installing apps from your browser or file manager).

**You need** Android 8 or newer, a compatible graphics driver, and about 2 GB free. See [Android architectures](docs/android-armv7.md) for ABI, GPU and preparation limits; 32-bit x86 imports prepared resources. Sign in with a Microsoft account that owns Minecraft: Java Edition, or play offline (most servers need the sign-in). No computer: on the first start Starling downloads the game's files from Mojang's servers and prepares them on the phone, once (79 seconds on a POCO F6 Pro, longer on slower phones). Go fishing or change the settings while it works.

Development happens in [starling-client/starling](https://github.com/starling-client/starling), on the `starling` branch, where Starling and Sunrise Android are now one app. Builds of every push there are **Starling Preview**, in [GitHub Actions](https://github.com/starling-client/starling/actions/workflows/android.yml) (the `starling-apk-<abi>` artifact of a run; GitHub asks you to sign in to download it): an app of its own, installed beside the release, with its own `StarlingPreview` folder and sign-in. You can also [build it from source](#build-from-source).

## What works today

- **Setting up on the phone:** sign in with your Microsoft account, and the game's files come from Mojang and are prepared on the phone, by a Java runtime fetched just for that and removed after, with every language the game has. While it works, fish in a little pond, or change the settings in a hand-drawn skin made for this screen (the game's own look is part of what is being prepared). Resources prepared on a computer still open too.
- **The game:** singleplayer with vanilla's world generation, mobs, villagers, raids, redstone, every command, the HUD and the recipe book, on saves in vanilla's format, saved as soon as the app leaves the screen. Worlds from 1.8 to 26.2 are upgraded as vanilla does. Starling follows Sunrise closely: v0.2.0 has its main of 2026-11-02.
- **Touch controls:** every action on screen, from the movement stick to the hotbar, chat, commands and F3, with a layout you can move, resize and fade in the game, before a world too, and Sneak as Hold or Toggle. Bedrock's control modes: a stick (floating or fixed) or a D-pad, and tap to interact (a tap hits the mob or uses the block under the finger, a press breaks the block, interacts with the mob or uses the item in the air) or aim crosshair. F3 held offers its combinations (chunk borders, hitboxes, tooltips...); Pick Block and the player list are buttons to turn on; the phone buzzes when a block breaks or a stack splits, and it can aim by turning (gyroscope, off by default). They wear Bedrock's own buttons, fetched on the phone from Mojang's samples repository after the preparing (Starling ships none of them, only their names and hashes), else Starling's own.
- **Custom controls:** open **H > Controls > Customize Controls**, or **Options > Controls > Customize Layout**. The Sunrise Android editor supports MCPE and GoyControl, separate presets, undo/redo, button dimensions, colors, visibility, and up to three input bindings. **Look while interacting** includes attack and use/place gestures; **360-degree movement** is off by default. See [Android controls](docs/android-controls.md).
- **Bedrock Inventory:** the creative inventory shows the items and your inventory side by side, with six rows of items and a 2x2 crafting grid, and a tap takes a full stack. It is on by default; turn it off in **Options > Controls** for the Java layout.
- **Your name and your skin:** in Options > Skin Customization, the name your worlds use and a skin from your files (or `Starling/skins`), classic or slim; a skin PNG opened with Starling is put on at once.
- **The phone's own keyboard:** a tap on a text field brings it up and a tap beside it puts it away, on every screen. Its suggestions, autocorrect, swipe typing and clipboard in the chat, signs and books (commands and searches go without autocorrect), a Paste button on the chat bar, and Enter finishing a command when only one completion fits (`/gamemode c` runs `/gamemode creative`).
- **Coming from Sunrise Android:** Starling offers to bring its worlds and resource packs over, in the setup or over the title screen, and copies them: Sunrise Android keeps its own.
- **Your files within reach:** worlds, resource packs, shader packs, mods and screenshots live in a `Starling` folder on the phone's storage, which any file manager opens (turned down on the first start, it is offered again by the Open Pack Folder buttons). Open a `.starling`, `.smod` or `.hnmod` file with Starling to add a pack, a world or a mod.
- **Made for phones:** settings picked for your device on the first start, and a stats overlay for FPS, memory and temperature.
- **Sunrise's own extras:** new menus (vanilla's are one setting away), graphics presets and more, all off by default where they would change the vanilla look. See [Settings](https://playsunrise.net/docs/settings).
- **Resource packs and data packs:** resource packs work, and data packs load functions, predicates and item modifiers.
- **Servers:** offline-mode servers, behind a proxy such as Velocity too, with their resource packs, coloured chat, boss bars, signs, workstation menus (enchanting, anvil, smithing...), flight and the attributes they set. Plugin servers' dialogs, chat buttons, custom armour (ItemsAdder, Nexo), models riding their mobs (ModelEngine, BetterModel) and clickable furniture work, and the touch stick moves in a way each server's anticheat (GrimAC and the like) accepts: Free Direction, D-pad or Analog, remembered per server.
- **Shader packs:** Iris and OptiFine packs such as Complementary, MakeUp, BSL, Photon and Bliss run on the phone, translated when they load: put them in `Starling/shaderpacks` and pick one in **Video Settings > Shader Packs**, where the **Auto** profile fits it to the phone, beside **Shader Options**, **Resolution** and **FPS Limit**. On Auto on a Snapdragon 8 Gen 2: MakeUp 87 fps, Complementary Reimagined 59, BSL 54, Complementary Unbound 39, Photon 27 to 39. See the [roadmap](roadmap/starling.md).
- **Starling Visuals:** Starling's own shader pack, made for phones, which comes with the app: pick it in **Video Settings > Shader Packs** ([packaging/shaderpacks/starling-visuals](packaging/shaderpacks/starling-visuals), also `StarlingVisuals.zip` on the releases page). Sunlight and shadows, water with waves and depth, haze thicker down low and at dawn, sun rays through the trees and clouds, bloom, and light from a torch in your hand.
- **Mods:** Sunrise's native mods (`.hnmod`), compiled from the phone itself, and content mods (`.smod`). See [Mods](#mods).

## Android status

What the Android port covers so far; what is being done now and next is in the [roadmap](roadmap/starling.md). The game itself is Sunrise's: its parity audit, measured on the desktop, is in [Sunrise's project status](https://github.com/ludovic111/sunrise#project-status) ([as of Starling's base](docs/sunrise-parity.md)).

| Part | Progress | Notes |
| --- | --- | --- |
<<<<<<< sync-sunrise-1
| The game, singleplayer | <img src="docs/media/progress/100.svg" alt="100%"> | Sunrise's game, as on the desktop |
| Touch controls | <img src="docs/media/progress/100.svg" alt="100%"> | Every action, with an editable layout and buttons of your own; a stick, a D-pad or Free Direction on servers; Bedrock's buttons, fetched from Mojang |
| Starling folder, `.starling`, `.smod` and `.hnmod` files | <img src="docs/media/progress/100.svg" alt="100%"> | Packs, worlds, mods and game files, opened with Starling |
| Resource packs | <img src="docs/media/progress/100.svg" alt="100%"> | Server packs too, loaded without freezing the game |
| Performance tuning | <img src="docs/media/progress/95.svg" alt="95%"> | Settings picked per phone, resolution down to 25% for weaker ones; mid-range phones not measured yet |
| Settings | <img src="docs/media/progress/100.svg" alt="100%"> | Options that do nothing are greyed out, those a phone can't use are hidden |
| Mods | <img src="docs/media/progress/95.svg" alt="95%"> | Native mods compiled from the phone, tried end to end with test mods; content mods; a run over the registry's mods still to do |
| Servers | <img src="docs/media/progress/100.svg" alt="100%"> | Offline-mode servers and proxies, dialogs, chat buttons and links, secure chat with a signed-in account |
| Server plugins | <img src="docs/media/progress/100.svg" alt="100%"> | Custom armour from any pack, animated models riding mobs, block and item displays, clickable furniture, item cooldowns, HUD text without shadows |
| Movement on anticheat servers | <img src="docs/media/progress/95.svg" alt="95%"> | Free Direction, D-pad or Analog per server, no pull-backs on GrimAC or Pika in tests; breaking blocks as vanilla predicts them (a dig before the move into its space); some old-version servers behind ViaVersion still pull back |
| Shader packs | <img src="docs/media/progress/95.svg" alt="95%"> | Complementary, MakeUp, BSL, Photon and Bliss run on a phone, with an Auto profile and Shader Options; some effects still differ |
| APK builds and updates | <img src="docs/media/progress/100.svg" alt="100%"> | Signed releases on the releases page, offered and installed by the game itself from v0.1.0.1 (the first offered: v0.1.1); every push also builds an APK in GitHub Actions |
| Open to LAN and invites | <img src="docs/media/progress/95.svg" alt="95%"> | Sunrise's, in the game; little tried on a phone so far |
| Physical controllers | <img src="docs/media/progress/100.svg" alt="100%"> | Android's gamepads, tried on a phone |
| Setting up on the phone | <img src="docs/media/progress/100.svg" alt="100%"> | Prepared on a phone in 79 s from the official 26.3 files, with Java fetched for it; fishing and hand-drawn settings while it works |
| Microsoft account sign-in | <img src="docs/media/progress/100.svg" alt="100%"> | On the first start and in the game (the title screen's corner); the code and the ownership check work on a phone |
=======
| Windows x64 | `sunrise-launcher-setup.exe` | Windows 10 or 11, a GPU with Vulkan or DirectX 12 |
| macOS, Apple Silicon | `sunrise-launcher-macos-arm64.dmg` | macOS 12 or later; signed ad hoc, not notarized |
| Linux x86-64 | `sunrise-launcher-linux-x86_64.AppImage` or `.tar.gz` | a glibc distribution, X11 or Wayland, a GPU with Vulkan or OpenGL 3.3 |
>>>>>>> sunrise/main

The title screen’s account button also manages saved Microsoft, Ely.by and offline
profiles. See [Android accounts](docs/android-accounts.md) for switching, registration
and provider compatibility.

<<<<<<< sync-sunrise-1
ARMv7 build instructions and GPU/memory limitations are documented in
[Android architectures](docs/android-armv7.md). Release APKs are published for
ARM64, ARMv7, x86 and x86_64; install the one matching your device.
=======
<table>
  <tr>
    <td width="50%"><img src="docs/media/readme/launcher-home.webp" alt="The launcher's Home page on the Dawn theme: the Sunrise instance with its icon, Sign in to play, the friend invite field and the instance's tabs"></td>
    <td width="50%"><img src="docs/media/readme/launcher-versions.webp" alt="The launcher's Versions page: the installed Nightly, marked latest and in use, with its notes"></td>
  </tr>
</table>
>>>>>>> sunrise/main

Local crash reports, Android exit history and optional CPU/GPU performance
recordings are available under **Options → Diagnostics**. Export a diagnostic ZIP,
review it, then share it with a bug report. See [Android diagnostics](docs/android-diagnostics.md)
for contents, limits and controls. Before installation or after an early startup
failure, Diagnostics also opens in a separate window; long-press the app icon to
reach it without starting the game.

<<<<<<< sync-sunrise-1
## Mods
=======
- **Content mods** (`.smod`): blocks, items, recipes, loot, models and textures, with no code.
- **Native mods** (also `.smod`, whose manifest has a `[native]` table): Rust that the launcher compiles into the game for you, with nothing to install.
- **Java mods** (Fabric, NeoForge, Forge) don't run: Sunrise has no Java. The launcher carries over the data and assets inside their jars, and imports modpacks (`.mrpack`, CurseForge zips, Prism instances) the same way ([modpacks](https://playsunrise.net/docs/modpacks)).
- **Shader packs**: Iris/OptiFine-format packs (`shaders/` folders) and Sunrise's own WGSL packs, under Video Settings → Shader Packs ([pack format](docs/shaders.md)).
>>>>>>> sunrise/main

Starling runs Sunrise's mods. Open the file with Starling, from your file manager or your downloads, and it shows up in **Options > Mods**.

- **`.hnmod`, native mods (ARM64 only):** tap **Compile**. Starling tells you the risks, builds the game with your mods while you fish or mine, and restarts with them. It takes a few minutes and needs a connection, and after an update you compile again. The build service currently produces ARM64 libraries; other app architectures explain this limitation and do not offer compilation or prebuilt downloads. A native mod runs as part of the game, so install only the ones you trust. See [native mods](docs/native-mods.md) and [how Starling builds them](docs/starling-native-mods.md); [wrench](native-mods/wrench) is an example.
- **`.smod`, content mods:** data-only mods, with nothing to compile. See [mods](docs/mods.md).

The old Starling mods and the WebAssembly mod format of v0.1 are gone.

## Build from source

For developers. Requirements: Rust 1.90 or newer with the `aarch64-linux-android` target, [cargo-ndk](https://github.com/bbqsrc/cargo-ndk), the Android SDK (build-tools 35.0.0, platform 36, NDK r28c), a JDK 25 and Python 3.10 or newer.

```sh
<<<<<<< sync-sunrise-1
git clone -b starling https://github.com/starling-client/starling.git
cd starling
python packaging/android/build_apk.py            # the APK lands in target/android/
=======
git clone https://github.com/sunrise-org/sunrise.git
cd sunrise
cargo run --release -p sunrise-prepare -- --prepare --server-jar ~/Downloads/server.jar   # needs JDK 25
cargo run --release -p sunrise-prepare -- sounds   # optional: sounds from your launcher install
cargo run --release -p sunrise-game
>>>>>>> sunrise/main
```

The build compiles Sunrise's original exporter classes with the host JDK and
packages their checked bundle. It uses `SUNRISE_MINECRAFT_DIR` as a read-only
source when set; otherwise it downloads checksum-pinned compilation inputs into
an ignored build cache. Minecraft jars, libraries and assets are never packaged.

The setup's hand-drawn skin is made by `python packaging/lowbudget/make.py` (with Pillow) into `packaging/lowbudget/pack`, which the APK carries as it is.

The APK contains no game files. On the phone, Starling downloads them from Mojang's servers and prepares them there. To prepare them on a computer instead, it builds its resources from **your own copy** of Minecraft: Java Edition 26.3, verified against the checksums in `research-lock.json`, the same way Sunrise does:

1. **Client:** install Java Edition with the official launcher and play 26.3 once. Sunrise reads `versions/26.3/`, `libraries/` and `assets/` from the launcher's game directory. If yours lives elsewhere, pass `--minecraft-dir` or set `SUNRISE_MINECRAFT_DIR`.
2. **Server jar:** the 26.3 server jar, from the `downloads.server.url` in your launcher's `versions/26.3/26.3.json`. Pass it with `--server-jar` or set `SUNRISE_SERVER_JAR`.

```sh
cargo run --release -p sunrise-prepare -- --prepare --server-jar ~/Downloads/server.jar   # needs JDK 25
```

The prepared files land in the gitignored `.local/sunrise-client/26.3`. Zip that folder, rename the zip to `.starling` and open it with Starling. It is derived from the game: never commit it or share it.

## Contributing

<<<<<<< sync-sunrise-1
Issues and pull requests are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md). Changes to the game itself belong upstream, in [Sunrise](https://github.com/ludovic111/sunrise); Starling is the Android app around it.

## Licence

[MIT](LICENSE), for the project's original work only, as Sunrise's. Dependencies keep their own licences. Minecraft is a trademark of Mojang Synergies AB; Minecraft code and assets belong to Mojang and Microsoft and are covered by the [Minecraft EULA](https://www.minecraft.net/en-us/eula). Prepared resources stay on your devices and are never published.
=======
Issues and pull requests are welcome: read [CONTRIBUTING.md](CONTRIBUTING.md) (vanilla first, small pull requests). Questions: this fork's [issues](https://github.com/sunrise-org/sunrise/issues). Before a pull request, run the self-check and paste its output:

```sh
git config core.hooksPath .githooks
python3 tools/pr_ready.py
```

It runs rustfmt, clippy, the touched crates' tests, the Python tests, the public-tree check and the vanilla harnesses your change affects. Never commit game files, decompiled source or prepared resources.

How Sunrise is run (the council, theme groups and their leads, the Sunrise Team, moderation): [GOVERNANCE.md](GOVERNANCE.md).

## Project status

**Player-visible gaps: 0 open** of the 833 the [port ledger](tools/port_ledger.py) found, reading every vanilla 26.3 method against Sunrise's code: vanilla behaviour a player would notice that Sunrise doesn't have yet. By area: Animals [0](https://github.com/horizon-rust/horizon/issues/3512); Monsters and bosses [0](https://github.com/horizon-rust/horizon/issues/3513); Entities, projectiles, vehicles and decorations [0](https://github.com/horizon-rust/horizon/issues/3514); Items, enchantments and inventories [0](https://github.com/horizon-rust/horizon/issues/3515); Blocks, redstone and fluids [0](https://github.com/horizon-rust/horizon/issues/3516); World, world generation and saves [0](https://github.com/horizon-rust/horizon/issues/3517); Screens, HUD and client packets [0](https://github.com/horizon-rust/horizon/issues/3518); Entity and block rendering, models [0](https://github.com/horizon-rust/horizon/issues/3519); Server and players on the server [0](https://github.com/horizon-rust/horizon/issues/3520); Mob AI, villagers and raids [0](https://github.com/horizon-rust/horizon/issues/3521). This number goes down as gaps close (`tools/parity_gaps.py`); the table below says what the harnesses compare.

The table is the project's own audit of what the harnesses compare, subsystem by subsystem: a score says how much of that subsystem the harnesses cover and pass, not that nothing else differs from vanilla. What players still run into: [known issues](docs/known-issues.md).

### What the harnesses compare

Scores: 0 (absent), 0.25 (started), 0.5 (playable, major features missing), 0.75 (mostly complete) or 1 (its harnesses cover the subsystem and pass). Last audited 2026-10-13: the harnesses pass on Linux and macOS. The [port ledger](tools/port_ledger.py) adds a method-by-method view of what is ported.

| Subsystem | Score | Main gaps |
| --- | ---: | --- |
| Window, input and game loop | 1 | None: game loop, options, camera, auto-jump, toggle sprint/sneak and frame limiter match |
| Assets and resources | 1 | None: item models match over 196,290 stacks; the resource pack screen over 3,000 sessions |
| Network protocol and multiplayer | 1 | None: joins vanilla servers; the dedicated server matches vanilla's files, login, configuration, play and command tree; LAN discovery over 462 cases (offline mode only) |
| Chunk storage and palettes | 1 | None: client and saves share paletted containers, identical to vanilla over 266 scenarios |
| Section meshing | 1 | None: 112 scenes and over a million quads identical to vanilla |
| Rendering | 1 | None: entity models, block entity renderers (407 cases) and animations (296), HUD overlays (2,200 states), the recipe book (600), player list, subtitles and world border match |
| Lighting | 1 | None: saved and relit light matches vanilla, slabs and stairs included |
| World generation | 1 | None: 2,352 whole chunks over 225 structure sites, in all three dimensions, identical to vanilla |
| Region files and saves | 1 | None: saves round-trip with vanilla; worlds from 1.8 to 26.2 upgrade as vanilla's DataFixers do |
| Entities and physics | 1 | None: attributes, spawn rules, natural spawning (15,500 cases), raids and patrols (1,900), `finalizeSpawn` (24,000) and mob AI (92 scenarios) match |
| Interaction and inventory | 1 | None: menu clicks over 8,000 sequences; recipe book placement (6,000), buttons (1,200), saved book (1,500), loom and crafter screens (1,600) |
| Menus, HUD and chat | 1 | None: every command (758 over 39,685 inputs), scoreboards (81,000 operations), the player list (872,000 draw calls), chat and its suggestions (3,000 sessions) match |
| Audio | 1 | None: the sound engine, sound events, music, situational music, ambience and subtitles match vanilla |
| Block behaviours and ticking | 1 | None: block updates, random ticks, redstone, dispensers (6,000 cases), crafters (4,000), container spills (4,000), copper golems and the sculk family (435 scenarios) match |

## Licence

Sunrise's code is under the [PolyForm Shield licence with the Sunrise Additional Terms](LICENSE): use it for anything, servers, hosting and mods included, and fork or port it freely as long as the fork stays free and open ([what that means](LICENSING.md)). Code that came from Horizon is also available under its original [MIT licence](LICENSES/MIT-Horizon.txt); dependencies keep their own licences ([notices](THIRD_PARTY_NOTICES.md)). Sunrise is a fork of Horizon, which is made by [Ludovic Marie](https://lu4ovic.com) ([@ludovic111](https://github.com/ludovic111)) and [its contributors](https://github.com/horizon-rust/horizon/graphs/contributors).
>>>>>>> sunrise/main

Third-party credits and licenses: [Third-party notices](THIRD_PARTY_NOTICES.md).

## Author

Starling, the Android port, by JuanLuci ([@JuanLuci](https://github.com/JuanLuci)). Sunrise made by [Ludovic Marie](https://lu4ovic.com) ([@ludovic111](https://github.com/ludovic111)).

Tested on a POCO F6 Pro (Snapdragon 8 Gen 2, Adreno 740, 12 GB of RAM, Android 16). It has also run on a Tecno Pova 7, on Mali-G57 and Mali-G68 GPUs, on a Redmi Note 8 (Mali-G52) and on many others.
