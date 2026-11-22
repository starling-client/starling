## Nightly from sunrise-org/sunrise

Install these signed game builds with the [Sunrise launcher for Linux x86_64, Windows x64 or macOS arm64 (Apple Silicon only)](https://github.com/sunrise-org/sunrise-launcher/releases/latest). Signed Nightly Play requires an account that owns Minecraft. Windows launcher packages are not Authenticode-signed: expect SmartScreen or unknown-publisher warnings. macOS launcher packages are ad hoc signed, not notarized: on macOS 14 and earlier, right-click the app and choose Open, then confirm Open; on macOS 15 and later, open it once, then use System Settings › Privacy & Security › Open Anyway and confirm.

The game archives below are not launcher installers. They include CI-tested game builds, a dedicated Linux server and the exact source pin for native mods. This Nightly's SDK is the separate `sdk-<full source SHA>` release for the same commit; native mods must match the installed game's source revision. The source archive is not a compiled SDK.

For client development without sign-in, [build an unsigned developer base from source](https://github.com/sunrise-org/sunrise/blob/main/docs/developer-install.md) and opt into single-player-only Play with `SUNRISE_LAUNCHER_DEVELOPER_OFFLINE=1`. This does not enable signed Nightly Play or account features.

Report bugs with the exact Nightly date and commit. You need your own copy of Minecraft: Java Edition.
