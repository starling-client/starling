//! Every built-in address of an outside service vanilla doesn't contact, in one place
//! (docs/layers.md#built-in-addresses). `tools/check_addresses.py` keeps new ones from going
//! anywhere else.
//!
//! This is a Sunrise-layer crate: the engine and the platform never depend on it. Platform code
//! that talks to a service gets the address passed in from a Sunrise crate.

/// Sunrise's official survival server (SMP), offered once at the top of the server list by the
/// game and the launcher (`launcher/links.json` may name another). Its SRV record leads to the
/// port.
pub const OFFICIAL_SERVER: &str = "play.playsunrise.net";

/// The official server's earlier addresses: the game and the launcher move a server list's
/// entries for them to [`OFFICIAL_SERVER`], once. A move appends the old address here (and to
/// `official_server_previous` in `launcher/links.json`).
pub const OFFICIAL_SERVER_PREVIOUS: &[&str] = &["sunrise.bunchy7s.dev"];

/// Afri-Network, a partner server the game added once below the official server (#9299) until
/// the partnership ended: kept so the game can take back out the entry it added.
pub const WITHDRAWN_PARTNER_SERVER: &str = "afri-network.co.za";

/// [`OFFICIAL_SERVER`] with the port its SRV record gives, for askers that don't look SRV up
/// (the telemetry server's status checks).
pub const OFFICIAL_SERVER_STATUS: &str = "play.playsunrise.net:24454";

/// Sunrise's website, where the launcher is downloaded: the game started without its prepared
/// files sends the player there.
pub const WEBSITE: &str = "https://playsunrise.net";

/// The cosmetics service: cosmetics, emotes, "Show me as a Sunrise player"
/// (`SUNRISE_COSMETICS_API` overrides it).
pub const COSMETICS_API: &str = "https://cosmetics.playsunrise.net";

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
    "https://github.com/horizon-rust/horizon/blob/main/docs/play-with-friends.md#troubleshooting";

/// Sunrise community invite used by launcher defaults.
pub const COMMUNITY_DISCORD: &str = "https://discord.gg/playsunrise";

/// The sponsoring hosting provider’s referral page.
pub const HOSTING_PARTNER: &str = "https://my.sculkhosting.com/ref/sunrise";

/// The official cosmetics shop.
pub const SHOP: &str = "https://playsunrise.net/shop";

/// The official launcher installer's product home.
pub const LAUNCHER_ABOUT: &str = "https://playsunrise.net";

/// The separately branded arismoko fork launcher installer's product home.
pub const FORK_LAUNCHER_ABOUT: &str = "https://github.com/arismoko/horizon";

/// Cargo's canonical source identity for the crates.io registry.
pub const CARGO_CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The pinned horizon-mixins archive the launcher downloads for `[[native.mixin]]` builds.
pub const HORIZON_MIXINS_SOURCE: &str =
    "https://codeload.github.com/horizon-rust/horizon-mixins/tar.gz";

/// Where the fork releases the prebuilt horizon-mixins tool; a release's tag and file follow.
pub const MIXINS_TOOL_RELEASES: &str = "https://github.com/arismoko/horizon/releases/download";

/// Where the fork publishes each game build's mod SDK (`sdk-<build id>`) and the game releases
/// whose `sdk.json` names their build; a release's tag and file follow. `SUNRISE_SDK_BASE`
/// overrides it.
pub const SDK_RELEASES: &str = "https://github.com/arismoko/horizon/releases/download";

/// The fork's newest releases, which `cargo sunrise-mod` reads to find the one SDK a short sha names.
pub const SDK_RELEASE_LIST: &str =
    "https://api.github.com/repos/arismoko/horizon/releases?per_page=100";
