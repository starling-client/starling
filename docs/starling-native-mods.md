# Native mods on Starling

Horizon's native mods ([native-mods.md](native-mods.md)) are horizon-mixins bundles: Rust that hooks the game's own
functions and is compiled into the game. On a computer the launcher downloads Rust and builds the game with them. A
phone cannot build the game (Rust and the NDK are gigabytes, and the build takes hours on a phone), so Starling's build
service does it and the phone swaps in the result:

1. **The mod.** The player opens an `.hnmod` with Starling (`import.rs`), which puts it in `horizon-native-mods/`.
2. **Compile.** The top of the Mods page says which game runs (the APK's own, or a build with which mods, compiled
   when), what the next start runs, and whether the native mods turned on are the ones in it; when they are not, it has
   Compile and Use GitHub (slower) (`game/native_builds.rs`). After a warning, the game sends the folder's mods that
   are on, without the assets and data only the game reads, to the service, and the player can fish or mine while it
   waits (`game/setup/fishing.rs`, `game/build_mining.rs`). Turning every native mod off makes the next start play the
   APK's own game; turning on the set of a build still on the phone chooses it again.
3. **The download.** The game's library comes back, is checked against its SHA-256, and is chosen for the next start
   (`files/native/current`). A dropped download goes on from where it stopped.
4. **The swap.** At the next start `StarlingActivity` loads the chosen library in place of the APK's own
   (`NativeMods.java`), so Java's native methods bind to it. NativeActivity still loads the APK's library: its entry
   point is `starling_on_create` (the manifest's `android.app.func_name`, `android_lib.rs`), which finds the chosen
   build already loaded (`STARLING_GAME_LIBRARY`) and starts the game there. Activity and game share one library.

**Safety nets.** A build is only loaded by the install of the APK it was made under: beside its library, `apk` holds
the APK's `lastUpdateTime` then (`NativeMods.stamp`, which `StarlingActivity` also hands the game as `STARLING_APK`).
Every test build of a version has the same versionName and versionCode, so any new APK, even of the same version,
leaves the build out of date: the Mods page says so in red, and Compile makes it again. A build that
closes twice before its first frame is set aside: the next start plays the APK's own game and the card says why. "Play
without native mods" stops using one. Nothing is installed: the APK stays the release.

## The build (native_build.sh)

1. horizon-mixins at Horizon's launcher's pinned commit (`MIXINS_REV`), built once into the cache.
2. The source of the commit (git archive, without links), and copies of the bundles, their pins set to that commit.
3. **One identity for the game module.** While horizon-mixins plans, horizon-android's library is an empty file (it
   refuses a module destination reached under two names, `E_TARGET_AMBIGUOUS`), and `lib.rs` loses the conditional
   attribute that keeps it from looking at the modules under that file. The bundles' targets stay Horizon's
   (`horizon_client::game::Game<'_>::use_item`), so Horizon's own native mods apply unchanged.
4. **The source as horizon-mixins reads it** (`horizon-materialize`, in horizon-mod-api-gen): the game's module lists,
   which its build script generates (`include!` of `OUT_DIR`), are written out as plain `#[path] mod` lines, as
   Horizon's launcher does, and the hook catalog Horizon declares in Rust becomes the `native-hooks.json` named hooks
   (`hook = "frame"`) are resolved through. The build leaves that file out: Horizon's generator refuses it.
5. **Mods made for older hooks** (`native_mod_adapt.py`). Horizon split its native hooks by area
   (`game/native_hooks/`), renamed many and dropped the `Native` prefix of their types; the registry is full of mods
   made for the old ones, which Starling 0.8.0 had. Both kinds apply side by side:
   - a target that moved is found by its type and name in the module tree (or a free function by its name) and the
     mod is pointed there; a `[[module]]` goes to the file its module is in now; `include!("shared.rs")` files are
     written into the file that includes them (horizon-mixins does not look into an `include!`);
   - a named hook the new catalog lacks comes from Starling 0.8.0's (`native-hooks-0.8.json`), pointed at where its
     function is now;
   - a binding to `batch` becomes `draw`, Horizon's name in its drawing hooks, and one to parameters a hook no longer
     has goes to the old hook Starling keeps as `<name>_old`;
   - the old hooks (`native_frame`, `native_hud`, `native_key`, `native_wheel`, `native_tick`, `native_commands`,
     `native_mods`, `native_settings`, `native_destroy_block`, ...) are kept in `game/starling/native_compat.rs` and
     called where their successors are, with the old helpers (`native_on`, `native_setting`, `native_playing`, ...) on
     the new ones; an old mod's buttons show as settings on Horizon's Options > Mods page;
   - the client's old `mod_api` module, with the old `Native*` types (most are the new ones under their old names), is
     `horizon_client::starling_mod_api`, which a mod's `horizon_client::mod_api` and `crate::mod_api` are pointed to.
6. `horizon-mixins apply` into a stage, then into a build folder kept between builds: files whose bytes changed get
   the time of now, the others keep theirs, so Cargo recompiles only what the mods touched.
7. `build_apk.py --profile fast`, as a release: the stripped library, an APK unless `NATIVE_LIBRARY_ONLY` is set, and
   `build.json`.

## Permissions

A native mod is written for Horizon on a computer, where nothing asks before a program opens the microphone. On Android
the app holds the permission and asks the player, and the mod stays as its maker wrote it: `native_build.sh` reads each
mod's source with `native_mod_permissions.py` and writes what it needs in `build.json` (the mod's `permissions`, and
all of them at the top). Today that is `microphone`: capturing sound through cpal (`build_input_stream`,
`default_input_device`...), AAudio, Oboe, OpenSL ES or Android's `AudioRecord`. Once a build with such a mod shows its
first frame, Starling says which mod wants what, then hands it to Android's own question (StarlingActivity
`askModPermissions`); Not now is remembered for that build. The manifest declares `RECORD_AUDIO` for this, and nothing
asks for it without such a mod.

## The service

`packaging/android/mod_build_server.py` builds one job at a time and keeps the last results: the same mods for the same
version get the first library at once. `versions.json` beside it says which commit each version of the game is.

A mod's code runs while it compiles (build scripts, macros), so `mod_build_sandbox.sh` runs each build with no network,
none of the machine's files or keys, and no way to gain privileges, over a clean build cache: what a build writes stays
in an overlay thrown away after it, so a mod can't reach a later player's build.

`mod_build_gateway.py`, behind HTTPS on a small machine that stays on, is what the game talks to. The machine that builds
sleeps until a request wakes it (about a minute) and goes back to sleep once it has been idle for 2 minutes. Finished
libraries stay on the small machine, so asking again wakes nothing. It limits how many builds an address, and everyone
together (`BUILDS_PER_DAY`, 80 unless set), can ask for in a day, and says which limit it was. Mods that are already built
pass at any limit, a build that failed gives its place back, and failures have a day limit of their own, counted by job.
Mods that fail for their own fault (a refused injection, a compile error with its code) aren't built again: asking again
gets the same error at once and counts nothing.

Each new release needs `versions.json` updated and the clean cache rebuilt on its commit, or its first builds are slow.

## On GitHub's machines

**Use GitHub (slower)** on the Compile card asks the gateway for GitHub's runners instead of the build machine
(`via=github`). The player needs no GitHub account, and the APK holds no secret.

1. The gateway keeps the request's mods under a random name and runs the `build.yml` workflow of the public
   `starling-client/mod-builds` repository with that name, using a token that may only run that repository's workflows
   (systemd's `LoadCredential`, `github-token`). Without the token this door is shut.
2. The runner restores the clean cache `base.yml` made for that version, fetches the mods, and runs
   `mod_build_once.py`: the same checks and the same sandbox as the service, so a mod's code has no network and can't see
   the runner's files. The job's token has no permissions.
3. The result, `status.json` and the library, is the run's artifact. The gateway checks it against its SHA-256 and serves
   it as any other, so the same mods asked either way build once.

The run's log is public, and so are its name and the artifact while it lasts (a day). The Compile card says so.

It costs nothing on a public repository, but a repository that keeps runners busy all day may be asked to stop, so
this has limits too (`GITHUB_PER_ADDRESS`, `GITHUB_PER_DAY`), and at most 8 runs go at once, the rest waiting in line on
the gateway. It lives in a repository of its own so that trouble there doesn't reach this one.

Each new release also needs its commit in mod-builds' `versions.json` and a run of `base.yml`.

## Where the service is

`packaging/android/build-server.txt`, read from the organisation's protected `starling` branch before each compile, says
the service's address, so it can move without an update. A game that can't read it uses the last address it read, then
the one built in. Change that file only once the new address answers.

## Another build server

The Native mods card's **Build server...** button lets a player type another server's address. A mod maker or a group can
run their own: it takes `mod_build_server.py` and `mod_build_sandbox.sh` on a Linux machine, the Android build tools, a
clean build cache and `versions.json` for each Starling version it serves, behind HTTPS. Most players can't do that, so
this is not how Starling's own limits go away.

- **Only typed by hand**, in the activity's dialog (`StarlingActivity.askBuildServer`): no link, file or setting from
  outside can choose it. HTTPS to a host and nothing else, so an address can't hide where it goes.
- **What it gets:** the same request as Starling's service, the mods' manifests and sources, and nothing else on the
  phone.
- **What it can do:** the library it sends back runs as the game, with its worlds and its signed-in account. The red
  warning is the only protection, and it is a warning: nothing can check another server's build. It shows in the dialog,
  and with the server's name on the Compile card, on the Native mods card and when such a build is ready.

## Security

- **No secret in the APK.** It only talks to the gateway, over HTTPS, and needs no account.
- **A native mod has the game's full access on the phone.** The warning says so; only the player can decide to trust it.
- **The library is checked** against the SHA-256 the service gives for it: that catches a damaged download, and proves
  nothing about the server.
