/// The release assets of each system Sunrise ships for: (OS, architecture, the game, the launcher).
/// game-build.yml builds the game's, nightly-build.yml signs and publishes them, and launcher.yml
/// builds the launcher's (`platform_assets_are_published`).
pub const PLATFORMS: [(&str, &str, &str, &str); 5] = [
    (
        "linux",
        "x86_64",
        "sunrise-linux-x86_64.tar.gz",
        if cfg!(sunrise_fork_launcher) {
            "sunrise-fork-launcher-linux-x86_64.tar.gz"
        } else {
            "sunrise-launcher-linux-x86_64.tar.gz"
        },
    ),
    (
        "macos",
        "aarch64",
        "sunrise-macos-arm64.zip",
        if cfg!(sunrise_fork_launcher) {
            "sunrise-fork-launcher-macos-arm64.zip"
        } else {
            "sunrise-launcher-macos-arm64.zip"
        },
    ),
    // Intel Macs (#1161).
    (
        "macos",
        "x86_64",
        "sunrise-macos-x64.zip",
        if cfg!(sunrise_fork_launcher) {
            "sunrise-fork-launcher-macos-x64.zip"
        } else {
            "sunrise-launcher-macos-x64.zip"
        },
    ),
    (
        "windows",
        "x86_64",
        "sunrise-windows-x64.zip",
        if cfg!(sunrise_fork_launcher) {
            "sunrise-fork-launcher-windows-x64.zip"
        } else {
            "sunrise-launcher-windows-x64.zip"
        },
    ),
    (
        "windows",
        "aarch64",
        "sunrise-windows-arm64.zip",
        if cfg!(sunrise_fork_launcher) {
            "sunrise-fork-launcher-windows-arm64.zip"
        } else {
            "sunrise-launcher-windows-arm64.zip"
        },
    ),
];
