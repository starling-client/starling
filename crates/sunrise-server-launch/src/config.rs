//! `server.toml`, the one file an owner edits (docs/server.md, section 4).
//!
//! It has typed sections for what owners change and `[properties]` for any other vanilla key. Before every
//! start the supervisor writes them into `server.properties`, which the game reads exactly as vanilla's
//! server does; what the game changes there itself (`/whitelist on`) is copied back afterwards.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};
use sunrise_platform::java_properties::{Properties, date_string, now_millis};
#[cfg(test)]
use sunrise_platform::server_files::biome_regions::MissingRegions;
use sunrise_platform::server_files::properties::{
    DIFFICULTIES, GAME_MODES, ServerProperties, SystemEntropy,
};
use sunrise_platform::server_files::{biome_regions::BiomeRegions, missing_mods::MissingMods};

pub const FILE: &str = "server.toml";
pub const PROPERTIES: &str = "server.properties";

/// What `init` writes: every typed setting, with vanilla 26.3's defaults and what it means.
pub const TEMPLATE: &str = r##"# Sunrise server settings (docs/server.md). The server writes them into server.properties before
# every start: edit this file, not that one.

[server]
motd = "A Minecraft Server"  # the line under the server's name in the server list
port = 25565                 # the port players connect to
bind = ""                    # listen on every address, or only on this one (server-ip)
max_players = 20             # how many players can be online at once
online_mode = true           # check players' Minecraft accounts (false: anyone joins under any name)
whitelist = true             # only whitelisted players and operators can join
enforce_whitelist = false    # kick online players taken off the whitelist
view_distance = 10           # chunks sent around each player
simulation_distance = 10     # chunks around each player where mobs, crops and redstone tick
max_viewed_loaded_chunks = 2048             # chunks a mod may keep loaded for one player in another level (0: none; at most 4489, higher values lowered with a warning; an older Sunrise client gets at most 2048)
max_viewed_ticking_chunks = 441             # of them, ticking (one player's simulation area at distance 10)
max_viewed_loaded_chunks_per_level = 16384  # the same for all players together, per level
max_viewed_ticking_chunks_per_level = 3528  # ticking, all players together, per level
admin_feedback_to_ops = true # operators see each other's commands ([name: ...], vanilla); false: only the log

[world]
name = "world"               # the world folder
seed = ""                    # empty: a random seed on the first start
game_mode = "survival"       # survival, creative, adventure or spectator
difficulty = "easy"          # peaceful, easy, normal or hard
hardcore = false             # one life: a player who dies becomes a spectator

# Any other server.properties key, spelled as vanilla spells it, for example:
# spawn-protection = 16
[properties]

[engine]
path = ""                    # the game program; empty: the one installed beside sunrise-server
resources = ""               # the prepared game files; empty: resources/ here, else the usual places
restart_on_crash = true      # start the game again when it crashes
low_memory = false           # true: no transparent huge pages, ~60 MB less idle for ~1 % more tick time
parallel_tick = false        # reserved: parallel phases are currently disabled in this fork
droppable_clientbound = 262144  # bytes a second of droppable mod payloads (voice) sent to each player; 0 turns them off
droppable_serverbound = 16384   # the same, accepted from each player

[backups]
every = "6h"                 # automatic backups while the server runs: 30m, 6h, 1d...; "0" turns them off
keep = 12                    # automatic backups kept (backups made by hand are never deleted)

[updates]
keep = 1                     # other releases kept by updates, besides the one in use and the one rollback goes to

[panel]
bind = "127.0.0.1:8130"      # the web panel: this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false         # true lets bind be any address; never put the panel on the open internet

[api]
bind = ""                    # the control API: "" a free port on 127.0.0.1; or a fixed one, like 127.0.0.1:8131 or a tailnet address
token = ""                   # its token (16 characters or more); "" a new random one at every start
allow_public = false         # true lets bind be any address; the token is then all that guards the server

# A proxy handing players over between servers (docs/server.md). Not vanilla's accepts-transfers (the transfer packet).
[transfer]
enabled = false              # true: take a proxy's player-* requests; refused at start since Velocity forwarding was removed
expect_seconds = 10          # how long a player the proxy announced is waited for, when it names no time
max_expect_seconds = 60      # the longest the proxy may ask for
max_expected = 64            # players waited for at once
max_freeze_seconds = 30      # the longest a player stays frozen while it is handed over
max_hold_seconds = 60        # the longest a player arriving from another server is held before it is dropped
stop_wait_seconds = 10       # a stop with players on waits this long for the proxy to move them out (0 to 60; 0: no wait)

# Post the chat, joins, leaves and deaths to a Discord channel, drawn like the in-game chat (docs/hosting.md)
[discord]
webhook = ""                 # the channel's webhook URL (a secret: anyone with it can post there); "" off
events = ["chat", "join", "leave", "death"]   # what is posted; "kick" too if you like
enabled = true               # false pauses it and keeps the webhook

# A world's biome regions from mods are fixed when it is made (docs/design/mod-api/mod-biome-regions.md).
# When a mod with one is gone, the server refuses to start until you choose. To change it:
# [biome_regions]
# missing = "vanilla"        # generate vanilla's biomes where the missing mods' regions were; "refuse": don't start
# adopt_installed = true     # new chunks use the installed mods' regions (the world keeps its saved weights)
"##;

/// Each typed setting: its section, its name, and the vanilla key it writes.
pub const TYPED: [(&str, &str, &str); 19] = [
    ("server", "motd", "motd"),
    ("server", "port", "server-port"),
    ("server", "bind", "server-ip"),
    ("server", "max_players", "max-players"),
    ("server", "online_mode", "online-mode"),
    ("server", "whitelist", "white-list"),
    ("server", "enforce_whitelist", "enforce-whitelist"),
    ("server", "view_distance", "view-distance"),
    ("server", "simulation_distance", "simulation-distance"),
    (
        "server",
        "max_viewed_loaded_chunks",
        "sunrise-max-viewed-loaded-chunks",
    ),
    (
        "server",
        "max_viewed_ticking_chunks",
        "sunrise-max-viewed-ticking-chunks",
    ),
    (
        "server",
        "max_viewed_loaded_chunks_per_level",
        "sunrise-max-viewed-loaded-chunks-per-level",
    ),
    (
        "server",
        "max_viewed_ticking_chunks_per_level",
        "sunrise-max-viewed-ticking-chunks-per-level",
    ),
    (
        "server",
        "admin_feedback_to_ops",
        "sunrise-admin-feedback-to-ops",
    ),
    ("world", "name", "level-name"),
    ("world", "seed", "level-seed"),
    ("world", "game_mode", "gamemode"),
    ("world", "difficulty", "difficulty"),
    ("world", "hardcore", "hardcore"),
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub world: World,
    #[serde(default)]
    pub properties: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub engine: Engine,
    #[serde(default)]
    pub backups: Backups,
    #[serde(default)]
    pub updates: Updates,
    #[serde(default)]
    pub panel: Panel,
    #[serde(default)]
    pub api: Api,
    /// A proxy's hand-overs (docs/server.md), passed to the game as `--transfers`.
    #[serde(default)]
    pub transfer: Transfer,
    /// Chat and players posted to a Discord channel (discord.rs).
    #[serde(default)]
    pub discord: Discord,
    /// A one-time prune of barely visited chunks before a start (prune.rs); written only when set.
    #[serde(default, skip_serializing_if = "crate::prune::Settings::is_default")]
    pub prune_chunks: crate::prune::Settings,
    /// The game reads this section itself (game/interface/dedicated/biome_regions.rs); written only when it
    /// isn't the default.
    #[serde(default, skip_serializing_if = "BiomeRegions::is_default")]
    pub biome_regions: BiomeRegions,
    /// The game reads this opt-in itself before opening a world with missing mods (docs/hosting.md).
    #[serde(default, skip_serializing_if = "MissingMods::is_default")]
    pub missing_mods: MissingMods,
}

const EVENTS: [&str; 6] = ["chat", "join", "leave", "death", "kick", "announce"];

/// The `[discord]` table of server.toml.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Discord {
    pub enabled: bool,
    pub webhook: String,
    pub events: Vec<String>,
}

impl Default for Discord {
    fn default() -> Self {
        Self {
            enabled: true,
            webhook: String::new(),
            events: ["chat", "join", "leave", "death"].map(String::from).into(),
        }
    }
}

impl Discord {
    pub fn check(&self) -> Result<(), String> {
        let hook = self.webhook.trim();
        let discord = [
            "discord.com",
            "discordapp.com",
            "ptb.discord.com",
            "canary.discord.com",
        ];
        if !hook.is_empty()
            && !discord.iter().any(|host| {
                hook.strip_prefix("https://")
                    .and_then(|h| h.strip_prefix(host))
                    .is_some_and(|path| path.starts_with("/api/webhooks/"))
            })
        {
            return Err("discord.webhook must be a Discord webhook URL (https://discord.com/api/webhooks/...)".into());
        }
        match self.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
            Some(e) => Err(format!(
                "discord.events has \"{e}\": use {}",
                EVENTS.join(", ")
            )),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    pub motd: String,
    pub port: u16,
    pub bind: String,
    pub max_players: u32,
    pub online_mode: bool,
    pub whitelist: bool,
    pub enforce_whitelist: bool,
    pub view_distance: u32,
    pub simulation_distance: u32,
    /// Viewed levels (docs/hosting.md): the chunks mods may keep loaded for one player in one other
    /// level (at most 4489 columns; a higher value is lowered with a warning, and an older
    /// Sunrise client gets at most 2048), the ticking ones among them, and the same for all
    /// players together in one level.
    pub max_viewed_loaded_chunks: u32,
    pub max_viewed_ticking_chunks: u32,
    pub max_viewed_loaded_chunks_per_level: u32,
    pub max_viewed_ticking_chunks_per_level: u32,
    /// Whether operators get each other's command feedback (`[name: ...]`, vanilla's broadcastToAdmins);
    /// false leaves it to the console's log, so nobody in game sees an operator's commands.
    pub admin_feedback_to_ops: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct World {
    pub name: String,
    /// A number or any text (vanilla hashes text); empty for a random seed.
    #[serde(deserialize_with = "text_or_number")]
    pub seed: String,
    pub game_mode: String,
    pub difficulty: String,
    pub hardcore: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Engine {
    pub path: String,
    pub resources: String,
    pub restart_on_crash: bool,
    /// The game without transparent huge pages (docs/hosting.md): less memory, slightly slower ticks.
    pub low_memory: bool,
    /// The game's parallel tick phases (`SUNRISE_PARALLEL_TICK=1`), for hosts that cannot set the variable.
    pub parallel_tick: bool,
    /// Bytes a second of `Droppable` mod payloads the game sends to each player (docs/hosting.md); 0 turns
    /// them off.
    pub droppable_clientbound: u32,
    /// Bytes a second of `Droppable` mod payloads the game accepts from each player; 0 turns them off.
    pub droppable_serverbound: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Panel {
    pub bind: String,
    pub allow_public: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8130".into(),
            allow_public: false,
        }
    }
}

impl Panel {
    /// Where the panel listens; `None` when it is off.
    pub fn address(&self) -> Result<Option<std::net::SocketAddr>, String> {
        listen_address("panel", &self.bind, self.allow_public, 8130)
    }
}

/// The control API's fixed address and token (docs/api/server.md); by default neither: a free port on
/// 127.0.0.1 and a random token, both written to `.sunrise/api.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Api {
    pub bind: String,
    pub token: String,
    pub allow_public: bool,
}

impl Api {
    /// Where the API listens: `bind`, or a free port on 127.0.0.1.
    pub fn address(&self) -> Result<std::net::SocketAddr, String> {
        Ok(listen_address("api", &self.bind, self.allow_public, 8131)?
            .unwrap_or_else(|| (std::net::Ipv4Addr::LOCALHOST, 0).into()))
    }

    fn check(&self) -> Result<(), String> {
        self.address()?;
        let token = self.token.trim();
        if !token.is_empty() && (token.len() < 16 || token.chars().any(|c| !c.is_ascii_graphic())) {
            return Err(
                "api.token must be 16 characters or more, without spaces (or \"\": a random one)"
                    .into(),
            );
        }
        Ok(())
    }
}

/// A `bind` setting: `None` when empty; anything but this computer or the tailnet needs `allow_public`.
fn listen_address(
    what: &str,
    bind: &str,
    allow_public: bool,
    example_port: u16,
) -> Result<Option<std::net::SocketAddr>, String> {
    if bind.trim().is_empty() {
        return Ok(None);
    }
    let address: std::net::SocketAddr = bind.trim().parse().map_err(|_| {
        format!(
            "{what}.bind is \"{bind}\": use an address and a port, like 127.0.0.1:{example_port}"
        )
    })?;
    if !allow_public && !private(address.ip()) {
        return Err(format!(
            "{what}.bind is \"{bind}\": the {what} listens only on this computer (127.0.0.1) or a tailnet \
             address (100.64.0.0/10, fd7a:115c:a1e0::/48); reach it from elsewhere over Tailscale or SSH"
        ));
    }
    Ok(Some(address))
}

/// Loopback, or a Tailscale address: reachable only from this computer or the owner's tailnet.
pub fn private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0]
        }
    }
}

/// `[transfer]`: a proxy moving players between this server and others (docs/server.md, the game's
/// `lan/arrivals.rs`). Off by default. The supervisor passes it to the game as `--transfers`, not as a
/// server.properties key, which stays vanilla's; and it is not vanilla's `accepts-transfers`, which is about the
/// transfer packet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transfer {
    pub enabled: bool,
    pub expect_seconds: u32,
    pub max_expect_seconds: u32,
    pub max_expected: u32,
    pub max_freeze_seconds: u32,
    pub max_hold_seconds: u32,
    pub stop_wait_seconds: u32,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            enabled: false,
            expect_seconds: 10,
            max_expect_seconds: 60,
            max_expected: 64,
            max_freeze_seconds: 30,
            max_hold_seconds: 60,
            stop_wait_seconds: 10,
        }
    }
}

impl Transfer {
    fn check(&self) -> Result<(), String> {
        for (name, n) in [
            ("expect_seconds", self.expect_seconds),
            ("max_expect_seconds", self.max_expect_seconds),
            ("max_expected", self.max_expected),
            ("max_freeze_seconds", self.max_freeze_seconds),
            ("max_hold_seconds", self.max_hold_seconds),
        ] {
            if n == 0 {
                return Err(format!("transfer.{name} must be above zero"));
            }
        }
        if self.expect_seconds > self.max_expect_seconds {
            return Err("transfer.expect_seconds is above transfer.max_expect_seconds".into());
        }
        // The supervisor kills a game that has not stopped within STOP_TIMEOUT: the wait must leave
        // time for the saves (the game caps the wait with a freeze at 90 s).
        if self.stop_wait_seconds > 60 {
            return Err("transfer.stop_wait_seconds is above 60".into());
        }
        // The game cuts the wait at 90 s: past it, no hand-over could start during a stop.
        if self.stop_wait_seconds > 0
            && u64::from(self.stop_wait_seconds) + u64::from(self.max_freeze_seconds) > 90
        {
            return Err(
                "transfer.stop_wait_seconds and transfer.max_freeze_seconds add up to more than 90: no hand-over could start during a stop (lower one, or set stop_wait_seconds = 0)"
                    .into(),
            );
        }
        Ok(())
    }

    /// The game's `--transfers` value, when the section turns transfers on.
    pub fn engine_arg(&self) -> Option<String> {
        self.enabled.then(|| {
            format!(
                "expect_seconds={},max_expect_seconds={},max_expected={},max_freeze_seconds={},max_hold_seconds={},stop_wait_seconds={}",
                self.expect_seconds,
                self.max_expect_seconds,
                self.max_expected,
                self.max_freeze_seconds,
                self.max_hold_seconds,
                self.stop_wait_seconds
            )
        })
    }
}

/// Accepted values for `backups.keep` in the file and the configuration command.
pub const BACKUP_KEEP_RANGE: std::ops::RangeInclusive<i64> = 1..=1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Backups {
    pub every: String,
    pub keep: u32,
}

impl Backups {
    /// The automatic backup period, or `None` when disabled.
    pub fn interval(&self) -> Result<Option<std::time::Duration>, String> {
        let every = self.every.trim();
        if every == "0" || every.is_empty() {
            return Ok(None);
        }
        let unit_at = every.char_indices().last().map_or(0, |(i, _)| i);
        let (number, unit) = every.split_at(unit_at);
        let n: u64 = number.trim().parse().map_err(|_| {
            format!("backups.every is {every:?}: use a number and m, h or d, like \"6h\"")
        })?;
        let secs = match unit {
            "m" => 60,
            "h" => 3600,
            "d" => 86_400,
            _ => {
                return Err(format!(
                    "backups.every is {every:?}: use a number and m, h or d, like \"6h\""
                ));
            }
        };
        let total = n
            .checked_mul(secs)
            .ok_or_else(|| format!("backups.every is {every:?}: too long"))?;
        Ok((n > 0).then(|| std::time::Duration::from_secs(total)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Releases kept in `releases/` after an update besides the one in use and the one `rollback` goes
    /// back to (the supervisor's update pruning policy).
    pub keep: u32,
}

impl Default for Updates {
    fn default() -> Self {
        Self { keep: 1 }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            motd: "A Minecraft Server".into(),
            port: 25565,
            bind: String::new(),
            max_players: 20,
            online_mode: true,
            whitelist: true,
            enforce_whitelist: false,
            view_distance: 10,
            simulation_distance: 10,
            max_viewed_loaded_chunks: 2048,
            max_viewed_ticking_chunks: 441,
            max_viewed_loaded_chunks_per_level: 2048 * 8,
            max_viewed_ticking_chunks_per_level: 441 * 8,
            admin_feedback_to_ops: true,
        }
    }
}
impl Default for World {
    fn default() -> Self {
        Self {
            name: "world".into(),
            seed: String::new(),
            game_mode: "survival".into(),
            difficulty: "easy".into(),
            hardcore: false,
        }
    }
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            path: String::new(),
            resources: String::new(),
            restart_on_crash: true,
            low_memory: false,
            parallel_tick: false,
            droppable_clientbound: 256 << 10,
            droppable_serverbound: 16 << 10,
        }
    }
}

impl Default for Backups {
    fn default() -> Self {
        Self {
            every: "6h".into(),
            keep: 12,
        }
    }
}

/// Keys no longer read, as (section, field, the server.properties key they wrote).
pub const RETIRED: [(&str, &str, &str); 1] =
    [("server", "max_view_distance", "sunrise-max-view-distance")];

/// The `section.field` names of `RETIRED` keys that `text` still sets.
pub fn retired_in(text: &str) -> Vec<String> {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    RETIRED
        .iter()
        .filter(|(section, field, _)| doc.get(section).and_then(|s| s.get(field)).is_some())
        .map(|(section, field, _)| format!("{section}.{field}"))
        .collect()
}

/// Whether a `section.field=value` change names a retired key, typed or as its property.
pub fn retired_change(change: &str) -> bool {
    let key = change.split_once('=').map_or(change, |(key, _)| key).trim();
    RETIRED.iter().any(|(section, field, property)| {
        key == format!("{section}.{field}") || key == format!("properties.{property}")
    })
}

/// `text` without its `RETIRED` keys, or `None` when it has none.
fn without_retired(text: &str) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().ok()?;
    let mut removed = false;
    for (section, field, _) in RETIRED {
        if let Some(table) = doc.get_mut(section).and_then(|s| s.as_table_like_mut()) {
            removed |= table.remove(field).is_some();
        }
    }
    removed.then(|| doc.to_string())
}

fn text_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seed {
        Text(String),
        Number(i64),
    }
    Ok(match Seed::deserialize(d)? {
        Seed::Text(s) => s,
        Seed::Number(n) => n.to_string(),
    })
}

/// Sections an older `server.toml` may still have, for features Sunrise no longer has: `parse` says
/// which to delete instead of serde's "unknown field".
const REMOVED_SECTIONS: [(&str, &str); 2] = [
    ("anticheat", "anticheat"),
    ("chat_filter", "the chat filter"),
];

fn removed(section: &str, feature: &str) -> String {
    format!("{feature} was removed from Sunrise; delete the [{section}] section from server.toml")
}
/// What `parse` answers for a `server.toml` that still sets `[server] accept_older_clients`.
const OLDER_CLIENTS_REMOVED: &str = "accept_older_clients was removed from Sunrise; delete it from the [server] section of server.toml";
/// What `parse` answers for a `server.toml` that still has an `[entities]` section (DAB, villager
/// lobotomy, the collision cap: mob settings that traded vanilla's behaviour for speed).
const ENTITIES_REMOVED: &str = "the mob settings dab, villager_lobotomize and max_collisions were removed from Sunrise; delete the [entities] section from server.toml";

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let stripped = without_retired(text);
        let text = stripped.as_deref().unwrap_or(text);
        let config: Self = toml::from_str(text).map_err(|e| {
            let table = toml::from_str::<toml::Table>(text).unwrap_or_default();
            REMOVED_SECTIONS
                .iter()
                .find(|(section, _)| table.contains_key(*section))
                .map_or_else(
                    || {
                        if table
                            .get("server")
                            .and_then(toml::Value::as_table)
                            .is_some_and(|s| s.contains_key("accept_older_clients"))
                        {
                            OLDER_CLIENTS_REMOVED.to_owned()
                        } else if table.contains_key("entities") {
                            ENTITIES_REMOVED.to_owned()
                        } else {
                            e.to_string()
                        }
                    },
                    |(section, feature)| removed(section, feature),
                )
        })?;
        config.check()?;
        Ok(config)
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e} (run `sunrise-server init` first)", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        let w = &self.world;
        if !GAME_MODES.contains(&w.game_mode.as_str()) {
            return Err(format!(
                "world.game_mode is \"{}\": use {}",
                w.game_mode,
                GAME_MODES.join(", ")
            ));
        }
        if !DIFFICULTIES.contains(&w.difficulty.as_str()) {
            return Err(format!(
                "world.difficulty is \"{}\": use {}",
                w.difficulty,
                DIFFICULTIES.join(", ")
            ));
        }
        // Not the server folder itself, nor one of the supervisor's own folders in it.
        if w.name.is_empty()
            || w.name.contains(['/', '\\'])
            || matches!(
                w.name.as_str(),
                "." | ".." | "backups" | "logs" | ".sunrise"
            )
        {
            return Err(format!(
                "world.name \"{}\" must be a folder name, without slashes",
                w.name
            ));
        }
        self.backups.interval()?;
        let keep = self.backups.keep;
        if !BACKUP_KEEP_RANGE.contains(&i64::from(keep)) {
            return Err(format!(
                "backups.keep is {keep}: use {} to {}",
                BACKUP_KEEP_RANGE.start(),
                BACKUP_KEEP_RANGE.end()
            ));
        }
        self.panel.address()?;
        self.api.check()?;
        self.transfer.check()?;
        self.discord.check()?;
        self.prune_chunks.check()?;
        if self.server.port == 0 {
            return Err("server.port must be a port number from 1 to 65535".into());
        }
        if let Some((section, field, key)) =
            TYPED.iter().find(|t| self.properties.contains_key(t.2))
        {
            return Err(format!(
                "[properties] sets {key}: set {section}.{field} instead"
            ));
        }
        Ok(())
    }

    /// The vanilla keys and values this configuration sets, typed settings first.
    pub fn vanilla(&self) -> Vec<(String, String)> {
        let table = toml::Table::try_from(self).expect("a configuration is a table");
        let mut out: Vec<(String, String)> = TYPED
            .iter()
            .map(|(section, field, key)| {
                ((*key).to_owned(), property_text(&table[*section][*field]))
            })
            .collect();
        out.extend(
            self.properties
                .iter()
                .map(|(k, v)| (k.clone(), property_text(v))),
        );
        out
    }

    /// Writes this configuration into `server.properties`, keeping the keys it doesn't set. Returns what it
    /// wrote and which keys cannot safely be copied back after the game runs.
    pub fn write_properties(&self, dir: &Path) -> std::io::Result<WrittenProperties> {
        let path = dir.join(PROPERTIES);
        // Only a missing file starts empty. One that cannot be read is left as it is and the start stops
        // with why, so the owner's other keys (RCON, command blocks...) are not reset to defaults.
        let mut properties = match std::fs::read(&path) {
            Ok(bytes) => Properties::load_file(&bytes)
                .map_err(|e| std::io::Error::other(format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Properties::default(),
            Err(e) => return Err(e),
        };
        let mut syncable = self.vanilla();
        for (key, value) in &syncable {
            properties.put(key, value.clone());
        }
        // Remember collisions while the original keys are still available.
        // A later read cannot tell which of the Writer's identical rows won.
        let aliases: HashSet<_> = properties
            .iter()
            .filter_map(|(key, _)| key.utf8_writer_alias())
            .collect();
        let mut writer_collisions = Vec::new();
        syncable.retain(|(key, _)| {
            if aliases.contains(key) {
                writer_collisions.push(key.clone());
                false
            } else {
                true
            }
        });
        // Use the Java OutputStream form: escapes preserve every unit in
        // keys we didn't set. The game, not the overlay, does the Writer pass.
        let text = properties.store(
            Some("Minecraft server properties"),
            &date_string(now_millis()),
            true,
        );
        replace(&path, &text)?;
        Ok(WrittenProperties {
            syncable,
            writer_collisions,
        })
    }

    /// The level `op` gives a new operator: `op-permission-level` as the game reads it at its next start,
    /// that is `server.properties` with this configuration's keys written over it (as
    /// [`Config::write_properties`] does). 4, vanilla's default, when the keys cannot be read.
    pub fn op_permission_level(&self, dir: &Path) -> i32 {
        let mut properties = read_properties(&dir.join(PROPERTIES)).unwrap_or_default();
        for (key, value) in self.vanilla() {
            properties.put(&key, value);
        }
        ServerProperties::new(properties, &mut SystemEntropy::default())
            .map_or(4, |p| p.op_permission_level)
    }

    /// A configuration from a vanilla `server.properties`: typed settings from their keys, and every other
    /// key the file sets to something other than vanilla's default under `[properties]`.
    pub fn import(source: &Properties) -> Result<Self, String> {
        // Normalization can replace an invalid typed value with a valid default.
        // Refuse unrepresentable source entries before that can hide them.
        let mut entries: Vec<_> = source.iter().collect();
        entries.sort_unstable_by_key(|(key, _)| *key);
        let mut keys = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key = key.to_utf8().map_err(|_| {
                format!("Cannot import server.properties key {:04X?}: TOML cannot represent an unpaired UTF-16 surrogate", key.as_units())
            })?;
            if value.has_unpaired_surrogate() {
                return Err(toml_value_refusal(&key));
            }
            keys.push(key);
        }
        let parsed = ServerProperties::new(source.clone(), &mut SystemEntropy::default())?;
        let defaults = ServerProperties::new(Properties::default(), &mut SystemEntropy::default())?;
        let mut table = toml::Table::try_from(Self::default()).map_err(|e| e.to_string())?;
        for (section, field, key) in TYPED {
            let (Some(text), Some(slot)) = (
                parsed.raw.get(key),
                table
                    .get_mut(section)
                    .and_then(|s| s.as_table_mut())
                    .and_then(|s| s.get_mut(field)),
            ) else {
                continue;
            };
            if let Some(value) = typed_value(&text, slot) {
                *slot = value;
            }
        }
        let mut extra = toml::Table::new();
        for key in keys {
            if TYPED.iter().chain(&RETIRED).any(|t| t.2 == key) {
                continue;
            }
            let value = parsed.raw.get(&key);
            if value.is_some() && defaults.raw.get(&key) == value {
                continue;
            }
            extra.insert(key, loose_value(&value.unwrap_or_default()));
        }
        table.insert("properties".into(), toml::Value::Table(extra));
        let config: Self = table.try_into().map_err(|e| e.to_string())?;
        config.check()?;
        Ok(config)
    }
}

fn toml_value_refusal(key: &str) -> String {
    format!(
        "Cannot copy server.properties value for {key:?}: TOML cannot represent an unpaired UTF-16 surrogate"
    )
}

/// `server.toml` for a configuration: the template's comments with this configuration's values.
pub fn render(config: &Config) -> String {
    let mut doc: toml_edit::DocumentMut = TEMPLATE.parse().expect("the template parses");
    let table = toml::Table::try_from(config).expect("a configuration is a table");
    for (section, field, _) in TYPED {
        set(&mut doc, section, field, &table[section][field]);
    }
    for (key, value) in &config.properties {
        set(&mut doc, "properties", key, value);
    }
    for (section, field) in [
        ("engine", "path"),
        ("engine", "resources"),
        ("engine", "restart_on_crash"),
        ("engine", "low_memory"),
        ("engine", "parallel_tick"),
        ("engine", "droppable_clientbound"),
        ("engine", "droppable_serverbound"),
        ("backups", "every"),
        ("backups", "keep"),
        ("updates", "keep"),
        ("panel", "bind"),
        ("panel", "allow_public"),
        ("api", "bind"),
        ("api", "token"),
        ("api", "allow_public"),
        ("transfer", "enabled"),
        ("transfer", "expect_seconds"),
        ("transfer", "max_expect_seconds"),
        ("transfer", "max_expected"),
        ("transfer", "max_freeze_seconds"),
        ("transfer", "max_hold_seconds"),
        ("transfer", "stop_wait_seconds"),
    ] {
        set(&mut doc, section, field, &table[section][field]);
    }
    let mut text = doc.to_string();
    // Serde omits optional sections at their defaults. Append the ones it kept that have
    // no table in the template, so accepting a new section is enough to preserve it here.
    for (section, settings) in table {
        if doc.contains_key(section.as_str()) {
            continue;
        }
        let mut only = toml::Table::new();
        only.insert(section, settings);
        text.push('\n');
        text.push_str(&toml::to_string(&only).expect("a section's settings are a table"));
    }
    text
}

/// A supervisor overlay's baseline for shutdown sync-back.
#[derive(Debug)]
pub struct WrittenProperties {
    syncable: Vec<(String, String)>,
    writer_collisions: Vec<String>,
}

impl WrittenProperties {
    /// Managed keys that the game's Writer cannot distinguish from another key.
    pub fn writer_collisions(&self) -> &[String] {
        &self.writer_collisions
    }
}

/// After the game stopped: copies into `server.toml` the unambiguous keys the game changed in
/// `server.properties` (`/whitelist on` stores `white-list`), keeping the owner's comments. Returns the keys
/// copied.
pub fn sync_back(dir: &Path, written: &WrittenProperties) -> Result<Vec<String>, String> {
    let Ok(now) = read_properties(&dir.join(PROPERTIES)) else {
        return Ok(Vec::new());
    };
    let mut changed = Vec::new();
    for (key, previous) in &written.syncable {
        if let Some(value) = now.get_java(key) {
            let value = value.to_utf8().map_err(|_| toml_value_refusal(key))?;
            if &value != previous {
                changed.push((key.as_str(), value));
            }
        }
    }
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let defaults = toml::Table::try_from(Config::default()).map_err(|e| e.to_string())?;
    for (key, value) in &changed {
        match TYPED.iter().find(|t| t.2 == *key) {
            Some((section, field, _)) => {
                if let Some(v) = typed_value(value, &defaults[*section][*field]) {
                    set(&mut doc, section, field, &v);
                }
            }
            None => set(&mut doc, "properties", key, &loose_value(value)),
        }
    }
    let updated = doc.to_string();
    Config::parse(&updated)?;
    replace(&path, &updated).map_err(|e| e.to_string())?;
    Ok(changed.iter().map(|(k, _)| (*k).to_owned()).collect())
}

/// Held while `server.toml` is read, changed and written back, so two of this process's writers
/// (management clients, the API, the supervisor's [`sync_back`]) don't drop each other's change.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Writes `text` to `path` through a new file renamed over it: a reader (or a crash) sees the old
/// file or the new one, never an empty or half-written one. An empty `server.toml` reads as every
/// default, and the next start would write those into `server.properties`. Another process (the
/// CLI's `config set`) has its own temporary file.
pub(crate) fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Sets one typed setting in `server.toml` (`whitelist on`), keeping the owner's comments.
pub fn update(dir: &Path, section: &str, field: &str, value: toml::Value) -> Result<(), String> {
    update_all(dir, &[(section.to_owned(), field.to_owned(), value)])
}

/// Sets settings in `server.toml`, keeping the owner's comments; nothing is written unless the result is
/// a valid configuration.
pub fn update_all(dir: &Path, changes: &[(String, String, toml::Value)]) -> Result<(), String> {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    for (section, field, value) in changes {
        set(&mut doc, section, field, value);
    }
    let updated = doc.to_string();
    Config::parse(&updated)?;
    replace(&path, &updated).map_err(|e| e.to_string())
}

pub fn read_properties(path: &Path) -> Result<Properties, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Properties::load_file(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Sets one value in a TOML document, keeping the comment after the old value.
fn set(doc: &mut toml_edit::DocumentMut, section: &str, key: &str, value: &toml::Value) {
    let Some(table) = doc
        .entry(section)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
    else {
        return;
    };
    let mut new = match value {
        toml::Value::String(s) => toml_edit::value(s.as_str()),
        toml::Value::Integer(n) => toml_edit::value(*n),
        toml::Value::Float(f) => toml_edit::value(*f),
        toml::Value::Boolean(b) => toml_edit::value(*b),
        other => toml_edit::value(other.to_string()),
    };
    if let (Some(old), Some(v)) = (
        table.get(key).and_then(|i| i.as_value()),
        new.as_value_mut(),
    ) {
        *v.decor_mut() = old.decor().clone();
    }
    table.insert(key, new);
}

/// How a TOML value reads in server.properties.
fn property_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A server.properties value as the TOML type of `like`.
fn typed_value(text: &str, like: &toml::Value) -> Option<toml::Value> {
    Some(match like {
        toml::Value::Boolean(_) => toml::Value::Boolean(text.eq_ignore_ascii_case("true")),
        toml::Value::Integer(_) => toml::Value::Integer(text.trim().parse().ok()?),
        _ => toml::Value::String(text.to_owned()),
    })
}

/// A server.properties value for `[properties]`: a boolean or a number when it reads back the same.
pub fn loose_value(text: &str) -> toml::Value {
    match text {
        "true" => toml::Value::Boolean(true),
        "false" => toml::Value::Boolean(false),
        _ => match text.parse::<i64>() {
            Ok(n) if n.to_string() == text => toml::Value::Integer(n),
            _ => toml::Value::String(text.to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(pairs: &[(&str, &str)]) -> Properties {
        let mut p = Properties::default();
        for (k, v) in pairs {
            p.put(k, (*v).to_owned());
        }
        p
    }

    #[test]
    fn intervals() {
        let interval = |every: &str| {
            Backups {
                every: every.into(),
                ..Backups::default()
            }
            .interval()
        };
        assert_eq!(
            interval("6h").unwrap(),
            Some(std::time::Duration::from_secs(21_600))
        );
        assert_eq!(
            interval("30m").unwrap(),
            Some(std::time::Duration::from_secs(1800))
        );
        assert_eq!(interval("0").unwrap(), None);
        assert!(interval("6 hours").is_err());
        assert!(interval("6é").is_err());
        assert!(interval("é").is_err());
        assert!(interval(&format!("{}d", u64::MAX)).is_err());
    }

    #[test]
    fn backup_retention_must_be_between_1_and_1000() {
        for keep in [0, 1001] {
            let error = Config::parse(&format!("[backups]\nkeep = {keep}\n")).unwrap_err();
            assert!(error.contains("backups.keep"), "{error}");
            assert!(error.contains("1 to 1000"), "{error}");
        }
        for keep in [1, 1000] {
            let config = Config::parse(&format!("[backups]\nkeep = {keep}\n")).unwrap();
            assert_eq!(config.backups.keep, keep);
        }
        let config = Config::parse("[backups]\nevery = \"0\"\n").unwrap();
        assert_eq!(config.backups.interval().unwrap(), None);
    }

    #[test]
    fn template_is_the_default() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
        assert_eq!(render(&Config::default()), TEMPLATE);
    }

    /// A server.toml from before the far ring was removed still loads, its `max_view_distance` unread.
    #[test]
    fn an_old_max_view_distance_is_ignored() {
        let old = TEMPLATE.replace(
            "simulation_distance = 10 ",
            "max_view_distance = 24\nsimulation_distance = 10 ",
        );
        assert_ne!(old, TEMPLATE);
        assert_eq!(Config::parse(&old).unwrap(), Config::default());
        assert_eq!(retired_in(&old), ["server.max_view_distance"]);
        assert!(retired_in(TEMPLATE).is_empty());
    }

    /// `[transfer]` is off by default, passes nothing to the game then, and is checked when it is read.
    #[test]
    fn transfers_are_off_by_default_and_checked() {
        assert_eq!(Config::default().transfer.engine_arg(), None);
        let on = TEMPLATE.replace(
            "enabled = false              # true: take",
            "enabled = true               # true: take",
        );
        let config = Config::parse(&on).unwrap();
        assert_eq!(
            config.transfer.engine_arg().as_deref(),
            Some(
                "expect_seconds=10,max_expect_seconds=60,max_expected=64,max_freeze_seconds=30,max_hold_seconds=60,stop_wait_seconds=10"
            )
        );
        assert_eq!(Config::parse(&render(&config)).unwrap(), config);
        for (from, to) in [
            ("expect_seconds = 10 ", "expect_seconds = 0  "),
            ("max_expect_seconds = 60", "max_expect_seconds = 5 "),
            ("max_freeze_seconds = 30", "max_freeze_seconds = 0 "),
            ("stop_wait_seconds = 10", "stop_wait_seconds = 61"),
            ("max_freeze_seconds = 30", "max_freeze_seconds = 81"),
        ] {
            let bad = on.replace(from, to);
            assert_ne!(bad, on, "{from} is in the template");
            assert!(Config::parse(&bad).is_err(), "{to} was taken");
        }
    }

    #[test]
    fn a_leftover_section_of_a_removed_feature_says_what_to_do() {
        let old = format!("{TEMPLATE}\n[anticheat]\nenabled = true\n");
        assert_eq!(
            Config::parse(&old),
            Err(
                "anticheat was removed from Sunrise; delete the [anticheat] section from server.toml"
                    .to_owned()
            )
        );
        let old = format!("{TEMPLATE}\n[chat_filter]\nenabled = false\n");
        assert_eq!(
            Config::parse(&old),
            Err(
                "the chat filter was removed from Sunrise; delete the [chat_filter] section from server.toml"
                    .to_owned()
            )
        );
        // Other unknown keys keep serde's own message.
        let other = Config::parse("[nonsense]\nenabled = true\n").unwrap_err();
        assert!(!other.contains("was removed from Sunrise"), "{other}");
    }

    #[test]
    fn a_leftover_accept_older_clients_key_says_what_to_do() {
        let old = TEMPLATE.replace("[server]\n", "[server]\naccept_older_clients = false\n");
        assert_ne!(old, TEMPLATE);
        assert_eq!(Config::parse(&old), Err(OLDER_CLIENTS_REMOVED.to_owned()));
        // Another unknown key in the section keeps serde's own message.
        let other = TEMPLATE.replace("[server]\n", "[server]\nnonsense = false\n");
        assert_ne!(Config::parse(&other).unwrap_err(), OLDER_CLIENTS_REMOVED);
    }

    #[test]
    fn missing_mods_opt_in_survives_rendering() {
        let on = Config::parse("[missing_mods]\nallow = true\n").unwrap();
        let rendered = render(&on);
        assert_eq!(Config::parse(&rendered).unwrap(), on);
        let table: toml::Table = rendered.parse().unwrap();
        assert_eq!(table["missing_mods"]["allow"].as_bool(), Some(true));

        for text in ["", "[missing_mods]\n", "[missing_mods]\nallow = false\n"] {
            let config = Config::parse(text).unwrap();
            assert!(!config.missing_mods.allow);
            let table: toml::Table = render(&config).parse().unwrap();
            assert!(!table.contains_key("missing_mods"));
        }
        let typo = Config::parse("[missing_mods]\nalow = true\n").unwrap_err();
        assert!(typo.contains("`alow`"), "{typo}");
    }

    #[test]
    fn a_fixed_api_address_takes_an_empty_token_and_refuses_one_with_spaces() {
        let fixed = Config::parse("[api]\nbind = \"127.0.0.1:8131\"\ntoken = \"\"\n").unwrap();
        assert_eq!(
            fixed.api.address(),
            Ok((std::net::Ipv4Addr::LOCALHOST, 8131).into())
        );
        let spaced = "[api]\nbind = \"127.0.0.1:8131\"\ntoken = \"a long random string\"\n";
        assert!(Config::parse(spaced).unwrap_err().contains("api.token"));
    }

    #[test]
    fn a_prune_chunks_section_survives_rendering_and_refuses_what_it_cannot_keep() {
        let set = Config::parse(
            "[prune_chunks]\nmode = \"dry-run\"\nmin_inhabited_ticks = 600\nspawn_radius = 16\nring = 2\nkeep_files = [\"claims.json\"]\n",
        )
        .unwrap();
        let prune = &set.prune_chunks;
        assert_eq!(prune.mode, crate::prune::Mode::DryRun);
        assert_eq!(
            (prune.min_inhabited_ticks, prune.spawn_radius, prune.ring),
            (600, 16, 2)
        );
        assert_eq!(prune.keep_files, ["claims.json"]);
        assert_eq!(Config::parse(&render(&set)).unwrap(), set);
        for bad in [
            "ring = 9",
            "min_inhabited_ticks = -1",
            "keep_files = [\"../x\"]",
            "keep_files = [\"/x\"]",
        ] {
            let error = Config::parse(&format!("[prune_chunks]\n{bad}\n")).unwrap_err();
            assert!(error.contains("prune_chunks"), "{bad}: {error}");
        }
    }

    #[test]
    fn a_discord_section_takes_a_discord_webhook_and_known_events_only() {
        let hook = "webhook = \"https://discord.com/api/webhooks/1/x\"";
        let set = Config::parse(&format!(
            "[discord]\n{hook}\nevents = [\"chat\", \"kick\"]\n"
        ))
        .unwrap();
        assert_eq!(set.discord.events, ["chat", "kick"]);
        let elsewhere = "[discord]\nwebhook = \"https://example.com/api/webhooks/1/x\"\n";
        assert!(
            Config::parse(elsewhere)
                .unwrap_err()
                .contains("discord.webhook")
        );
        let unknown = format!("[discord]\n{hook}\nevents = [\"chat\", \"bogus\"]\n");
        assert!(Config::parse(&unknown).unwrap_err().contains("bogus"));
    }

    #[test]
    fn a_leftover_entities_section_says_what_to_do() {
        let old = format!("{TEMPLATE}\n[entities]\ndab = true\nmax_collisions = 8\n");
        assert_eq!(Config::parse(&old), Err(ENTITIES_REMOVED.to_owned()));
        let old = format!("{TEMPLATE}\n[entities]\nvillager_lobotomize = false\n");
        assert_eq!(Config::parse(&old), Err(ENTITIES_REMOVED.to_owned()));
        let other = Config::parse("[nonsense]\nenabled = true\n").unwrap_err();
        assert_ne!(other, ENTITIES_REMOVED);
    }

    #[test]
    fn biome_regions_refuse_missing_mods_unless_their_section_says() {
        assert_eq!(
            Config::default().biome_regions.missing,
            MissingRegions::Refuse
        );
        assert!(!Config::default().biome_regions.adopt_installed);
        let set = Config::parse(&format!(
            "{TEMPLATE}\n[biome_regions]\nmissing = \"vanilla\"\nadopt_installed = true\n"
        ))
        .unwrap();
        assert_eq!(set.biome_regions.missing, MissingRegions::Vanilla);
        assert!(set.biome_regions.adopt_installed);
        // Kept when the file is written again, and left out while default.
        assert_eq!(Config::parse(&render(&set)).unwrap(), set);
        let off = render(&Config::default());
        assert!(!off.lines().any(|l| l.trim() == "[biome_regions]"), "{off}");
        assert!(Config::parse("[biome_regions]\nmissing = \"skip\"\n").is_err());
    }

    #[test]
    fn defaults_are_vanilla() {
        // Every typed default is what vanilla 26.3 writes for a missing key.
        let vanilla =
            ServerProperties::new(Properties::default(), &mut SystemEntropy::default()).unwrap();
        for (key, value) in Config::default().vanilla() {
            // Sunrise's own, which vanilla doesn't have: their defaults behave as vanilla does
            // (operators told of each other's commands, and caps on viewed areas, which only a
            // mod asks for).
            if key.starts_with("sunrise-") {
                let vanilla_like = match key.as_str() {
                    "sunrise-admin-feedback-to-ops" => "true",
                    "sunrise-max-viewed-loaded-chunks" => "2048",
                    "sunrise-max-viewed-ticking-chunks" => "441",
                    "sunrise-max-viewed-loaded-chunks-per-level" => "16384",
                    "sunrise-max-viewed-ticking-chunks-per-level" => "3528",
                    _ => "0",
                };
                assert_eq!(value, vanilla_like, "{key}");
                continue;
            }
            assert_eq!(
                vanilla.raw.get(&key).as_deref(),
                Some(value.as_str()),
                "{key}"
            );
        }
    }

    #[test]
    fn import_refuses_properties_toml_cannot_represent() {
        for input in [
            "?=literal\n\\uD800=surrogate\n",
            "level-seed=\\uD800\n",
            "server-port=\\uD800\n",
        ] {
            let source = Properties::load_str(input).unwrap();
            assert!(Config::import(&source).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn import_keeps_what_differs() {
        let source = properties(&[
            ("motd", "Hello"),
            ("server-port", "25600"),
            ("white-list", "false"),
            ("gamemode", "1"),
            ("level-seed", "12345"),
            ("spawn-protection", "0"),
            ("allow-flight", "false"),
            ("rate-limit", "10"),
            ("sunrise-max-view-distance", "24"),
        ]);
        let c = Config::import(&source).unwrap();
        assert_eq!(c.server.motd, "Hello");
        assert_eq!(c.server.port, 25600);
        assert!(!c.server.whitelist);
        assert_eq!(c.world.game_mode, "creative");
        assert_eq!(c.world.seed, "12345");
        // allow-flight=false is vanilla's default, and sunrise-max-view-distance is retired: dropped.
        assert_eq!(
            c.properties,
            BTreeMap::from([
                ("rate-limit".to_owned(), toml::Value::Integer(10)),
                ("spawn-protection".to_owned(), toml::Value::Integer(0)),
            ])
        );
        let text = render(&c);
        assert!(text.contains("spawn-protection = 0"), "{text}");
        assert!(text.contains("port = 25600"), "{text}");
        assert_eq!(Config::parse(&text).unwrap(), c);
    }

    #[test]
    fn the_panel_stays_private() {
        let panel = |bind: &str, allow_public| Panel {
            bind: bind.into(),
            allow_public,
        };
        assert!(panel("127.0.0.1:8130", false).address().unwrap().is_some());
        assert!(panel("[::1]:8130", false).address().unwrap().is_some());
        assert!(
            panel("100.101.102.103:8130", false)
                .address()
                .unwrap()
                .is_some()
        );
        assert!(
            panel("[fd7a:115c:a1e0::1]:8130", false)
                .address()
                .unwrap()
                .is_some()
        );
        assert_eq!(panel("", false).address().unwrap(), None);
        assert!(panel("0.0.0.0:8130", false).address().is_err());
        assert!(panel("100.128.0.1:8130", false).address().is_err());
        assert!(panel("192.168.1.2:8130", false).address().is_err());
        assert!(panel("192.168.1.2:8130", true).address().unwrap().is_some());
        assert!(panel("localhost", false).address().is_err());
    }

    #[test]
    fn seed_may_be_a_number() {
        let c = Config::parse("[world]\nseed = -42\n").unwrap();
        assert_eq!(c.world.seed, "-42");
    }

    #[test]
    fn rejects_mistakes() {
        assert!(Config::parse("[world]\ngame_mode = \"creatve\"\n").is_err());
        assert!(Config::parse("[server]\nprot = 1\n").is_err());
        assert!(Config::parse("[properties]\nmotd = \"x\"\n").is_err());
        assert!(Config::parse("[world]\nname = \"../x\"\n").is_err());
        for folder in [".", "..", "backups", "logs", ".sunrise"] {
            let text = format!("[world]\nname = \"{folder}\"\n");
            assert!(Config::parse(&text).is_err(), "{folder}");
        }
    }

    #[test]
    fn settings_changed_at_once_are_all_kept() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-server-at-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), TEMPLATE).unwrap();
        // Eight writers at once, as eight management clients each setting another key.
        std::thread::scope(|scope| {
            for n in 0..8i64 {
                let dir = &dir;
                scope.spawn(move || {
                    update(
                        dir,
                        "properties",
                        &format!("sunrise-test-{n}"),
                        toml::Value::Integer(n),
                    )
                    .unwrap()
                });
            }
        });
        let c = Config::load(&dir).unwrap();
        for n in 0..8i64 {
            assert_eq!(
                c.properties.get(&format!("sunrise-test-{n}")),
                Some(&toml::Value::Integer(n)),
                "sunrise-test-{n} kept"
            );
        }
        // No temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(FILE)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_server_properties_that_cannot_be_read_is_left_alone() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-server-config-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), render(&Config::default())).unwrap();
        // The owner's own keys, and a line the properties parser refuses (a bad \u escape).
        let own = "enable-rcon=true\nrcon.password=secret\nresource-pack=C:\\uZZZZ\\pack.zip\n";
        std::fs::write(dir.join(PROPERTIES), own).unwrap();
        let error = Config::default().write_properties(&dir).unwrap_err();
        assert!(error.to_string().contains(PROPERTIES), "{error}");
        assert_eq!(std::fs::read_to_string(dir.join(PROPERTIES)).unwrap(), own);
        // No file at all is a first start: it is written from server.toml.
        std::fs::remove_file(dir.join(PROPERTIES)).unwrap();
        Config::default().write_properties(&dir).unwrap();
        assert!(dir.join(PROPERTIES).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_properties_and_syncs_back() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-server-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Config::default();
        c.properties
            .insert("spawn-protection".into(), toml::Value::Integer(0));
        c.properties
            .insert("?".into(), toml::Value::String("managed".into()));
        std::fs::write(dir.join(FILE), render(&c)).unwrap();
        std::fs::write(
            dir.join(PROPERTIES),
            "rcon.port=25576\nmotd=old\n?=literal\n\\uD800=surrogate\ncustom=\\uDC00\n",
        )
        .unwrap();
        let written = c.write_properties(&dir).unwrap();
        let p = read_properties(&dir.join(PROPERTIES)).unwrap();
        assert_eq!(p.get("motd").as_deref(), Some("A Minecraft Server"));
        assert_eq!(
            p.get("rcon.port").as_deref(),
            Some("25576"),
            "other keys are kept"
        );
        assert_eq!(p.get("spawn-protection").as_deref(), Some("0"));
        assert_eq!(p.get("?").as_deref(), Some("managed"));
        assert!(
            p.iter().any(|(key, value)| key.as_units() == [0xd800]
                && value.to_utf8().unwrap() == "surrogate")
        );
        assert_eq!(p.get_java("custom").unwrap().as_units(), [0xdc00]);
        // The dedicated server rewrites its settings using the Writer path.
        let path = dir.join(PROPERTIES);
        ServerProperties::from_file(&path, &mut SystemEntropy::default())
            .unwrap()
            .update("white-list", "false".into())
            .unwrap()
            .update("spawn-protection", "4".into())
            .unwrap()
            .store(&path)
            .unwrap();
        let after_writer = read_properties(&path).unwrap();
        assert_eq!(after_writer.get("?").as_deref(), Some("surrogate"));
        assert_eq!(written.writer_collisions(), ["?"]);
        let mut changed = sync_back(&dir, &written).unwrap();
        changed.sort();
        assert_eq!(changed, ["spawn-protection", "white-list"]);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(
            text.contains("whitelist = false             # only whitelisted"),
            "the comment stays: {text}"
        );
        let back = Config::parse(&text).unwrap();
        assert!(!back.server.whitelist);
        assert_eq!(back.properties["spawn-protection"], toml::Value::Integer(4));
        assert_eq!(back.properties["?"].as_str(), Some("managed"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
