## Nightly from arismoko/horizon

This is the modding fork of Horizon at [arismoko/horizon](https://github.com/arismoko/horizon), not an official upstream Horizon release. **These signed Nightlies serve developers and server hosts; they are not a ready-to-play client installer. The official Horizon launcher does not install them, and no prebuilt fork launcher is published.**

The archives provide the dedicated server, an exact source pin for native mods, and builds exercised by CI. A launcher built from this fork's source can install the signed game today. With Rust and the platform's native build prerequisites installed:

```sh
git clone --depth 1 https://github.com/arismoko/horizon.git
cd horizon
HORIZON_LAUNCHER_HOME="$PWD/.local/fork-launcher-install" cargo run --locked --release --manifest-path launcher/Cargo.toml --bin horizon-launcher -- install nightly
```

Installation is not authenticated Play. An unconfigured source launcher cannot obtain a new Microsoft sign-in. Microsoft sign-in is part of the configured fork launcher work, using upstream's application with permission; that launcher is not published yet. A separate launcher home avoids mixing installation data, but does not isolate the current source launcher's OS keychain. Do not copy another launcher's credentials.

For launcher-managed client development without sign-in, [build and install an unsigned developer base from source](https://github.com/arismoko/horizon/blob/main/docs/modding/src/how-to/native-mods/042-local-developer-builds.md), prepare it from your own Minecraft: Java Edition installation, and opt into single-player-only developer Play with `HORIZON_LAUNCHER_DEVELOPER_OFFLINE=1`. That option does not enable signed Nightly Play or account features. Signed releases retain their ownership check; existing compatible cached sessions and standalone game entry are separate code paths, not a new sign-in service.

The game archives below are not launcher installers. A Nightly can include selected pull requests before they merge; report bugs to this fork with the exact Nightly date and commit.
