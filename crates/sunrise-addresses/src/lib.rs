//! Every built-in address of an outside service vanilla doesn't contact, in one place
//! (docs/layers.md#built-in-addresses). `tools/check_addresses.py` keeps new ones from going
//! anywhere else.
//!
//! This is a Sunrise-layer crate: the engine and the platform never depend on it. Platform code
//! that talks to a service gets the address passed in from a Sunrise crate.

/// Sunrise's website, where the launcher is downloaded: the game started without its prepared
/// files sends the player there.
pub const WEBSITE: &str = "https://playsunrise.net";

/// The repository's licence file, linked from the Mods screen's Sunrise row.
pub const LICENSE: &str = "https://github.com/sunrise-org/sunrise/blob/main/LICENSE";

/// The repository's issue tracker, linked from the Mods screen's Sunrise row.
pub const ISSUES: &str = "https://github.com/sunrise-org/sunrise/issues";

/// The telemetry ingest server (`crates/sunrise-telemetry-server`): opt-in telemetry, the
/// "I'm playing" ping, benchmark uploads (`SUNRISE_TELEMETRY_URL` overrides it).
pub const TELEMETRY: &str = "https://telemetry.playsunrise.net";

/// The performance viewer page a saved capture opens in (`SUNRISE_PERF_VIEWER` overrides it).
pub const PERF_VIEWER: &str = "https://playsunrise.net/perf";

/// The capture upload API behind Share (`SUNRISE_PERF_CAPTURES` overrides it).
pub const PERF_CAPTURES: &str = "https://playsunrise.net/api/perf/captures";

/// Public STUN servers for Play with Friends' NAT discovery: two independent operators, so one
/// outage doesn't break discovery (`SUNRISE_STUN_SERVERS` overrides them).
pub const STUN_SERVERS: &[&str] = &["stun.cloudflare.com:3478", "stun.l.google.com:19302"];

/// Play with Friends' troubleshooting page, shown to hosts nobody can reach.
pub const FRIENDS_HELP: &str =
    "https://github.com/sunrise-org/sunrise/blob/main/docs/play-with-friends.md#troubleshooting";

/// Cargo's canonical source identity for the crates.io registry.
pub const CARGO_CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The pinned horizon-mixins archive the launcher downloads for `[[native.mixin]]` builds.
pub const HORIZON_MIXINS_SOURCE: &str =
    "https://codeload.github.com/horizon-rust/horizon-mixins/tar.gz";

/// Where the fork releases the prebuilt horizon-mixins tool; a release's tag and file follow.
pub const MIXINS_TOOL_RELEASES: &str = "https://github.com/sunrise-org/sunrise/releases/download";

/// Where the fork publishes each game build's mod SDK (`sdk-<build id>`) and the game releases
/// whose `sdk.json` names their build; a release's tag and file follow. `SUNRISE_SDK_BASE`
/// overrides it.
pub const SDK_RELEASES: &str = "https://github.com/sunrise-org/sunrise/releases/download";

/// A file of the game's source at one commit; the commit and the file's path follow. `native-lock`
/// reads the pinned build's `Cargo.lock` from it.
pub const GAME_SOURCE_FILES: &str = "https://raw.githubusercontent.com/sunrise-org/sunrise";

/// The fork's newest releases, which `cargo sunrise-mod` reads to find the one SDK a short sha names.
pub const SDK_RELEASE_LIST: &str =
    "https://api.github.com/repos/sunrise-org/sunrise/releases?per_page=100";

/// The game repository's Git object API, used to resolve stable release tags to their commits.
pub const GAME_GIT_API: &str = "https://api.github.com/repos/sunrise-org/sunrise/git";

/// The official Horizon Minecraft server's host, as a player types it to join (its SRV record
/// leads to port 24454). Reached by name, never by IP address, which can shift. The Sunrise layer
/// presents the client identity the server requires (`sunrise-official`).
pub const HORIZON_SERVER: &str = "play.playhorizon.dev";
