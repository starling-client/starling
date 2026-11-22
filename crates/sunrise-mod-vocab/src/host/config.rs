//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
use std::path::PathBuf;
/// The host's methods both sides share of the `config` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ConfigHost {
    /// Physical process, not the logical side of this hook.
    fn dist(&self) -> Dist;
    /// Exact Minecraft version supported by this process.
    fn game_version(&self) -> &'static str;
    /// The engine's `DataVersion` written into level data and chunks.
    /// Constant on both sides, including before a world is open; this describes this
    /// game build, not the version of a remote server or an older save.
    fn server_data_version(&self) -> i32;
    /// Sunrise version printed in crash reports.
    fn sunrise_version(&self) -> &'static str;
    /// Side: both.
    /// Whether the player left mod `mod_id` on in Options > Mods. A mod
    /// turned off does nothing (check this first in each hook) until the
    /// launcher's next build leaves it out.
    fn on(&self, mod_id: &str) -> bool;
    /// Side: both.
    /// A mod's setting `key` (one file per key in `native_data_dir`). A mod reads only its own:
    /// another mod's id or a key not registered by that owner gives None.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
    /// Side: both.
    /// Keeps a mod's registered setting `key`. Another mod's id or an unregistered key keeps
    /// nothing. A successful write invalidates the typed read cache for this data folder and key;
    /// a failed write leaves it unchanged. This raw write does not fire `setting_changed`.
    fn set_setting(&self, mod_id: &str, key: &str, value: &str);
    /// Side: both.
    /// The calling mod's own files in the game's `config/` folder, beside
    /// `options.txt` (beside `server.properties` on a dedicated server):
    /// its folder `config/<mod id>/` and the files beside it named
    /// `<mod id>.<ext>` or `<mod id>-<rest>`, as Fabric and NeoForge mods
    /// keep theirs. The service is bound to the mod whose hook or part is
    /// running, so a mod never names its id and can't reach another mod's
    /// files; a world's `serverconfig/` is read only through
    /// `config_loading`.
    #[static_call]
    fn config_files(&self) -> &'static dyn ConfigFiles;
    /// Side: both. The calling mod's bulk files in
    /// `sunrise-mod-data/<mod id>/` in the game folder, never scanned as
    /// settings or config. Paths omit the mod id; `ConfigPath::root()` names
    /// its root. Sunrise allows at most `DATA_FILE_MAX` bytes per file.
    #[static_call]
    fn data_files(&self) -> &'static dyn ConfigFiles;
    /// Side: both.
    /// The level in place (`minecraft:overworld`), for a hook that isn't told
    /// which it runs in.
    fn dimension(&self) -> &'static str;
    /// Side: both.
    /// Whether the server's current tick still has time before the next one is
    /// due (`MinecraftServer.haveTime`, NeoForge's `ServerTickEvent.hasTime` and
    /// `LevelTickEvent.hasTime`): a mod doing optional work in `server_tick` or
    /// `level_tick_pre`/`level_tick_post` skips it when this is false. Always
    /// true on a client, and outside a scheduled tick.
    fn server_has_time(&self) -> bool;
    /// Side: both.
    /// The instance (game) folder, which its players share: `saves/`,
    /// `resourcepacks/`, `shaderpacks/` and `sunrise-mods/` are in it, and the
    /// title's screens create worlds and read packs there. A world's own
    /// `datapacks/` is in that world's folder (`saves/<world>/datapacks`).
    /// The folder the game runs in (`--player-dir` moves a player's options
    /// and servers only, not the instance). Nothing is created.
    fn instance_dir(&self) -> PathBuf;
}
/// The server host's methods of the `config` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ConfigServerHost {
    /// Startup opt-in for test-only content. Test registration runs regardless.
    fn game_tests_enabled(&self) -> bool;
    /// Side: server.
    /// The server's tick rate (20 unless `/tick rate` changed it) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower.
    fn tick_stats(&self) -> (f32, f64);
    /// Side: server.
    /// The last 100 server ticks' durations in nanoseconds, as
    /// `MinecraftServer.getTickTimesNanos` lays them out: the tick numbered `n` at index
    /// `n % 100`, 0 where none was recorded yet. Sort a copy for percentiles; `tick_stats`
    /// has the average.
    fn tick_times(&self) -> Vec<u64>;
    /// Side: server.
    /// The server's view distance in chunks (`PlayerList.getViewDistance`, `view-distance`):
    /// how far players are sent chunks at most. None off a server open to players.
    fn server_view_distance(&self) -> Option<i32>;
    /// Side: server.
    /// The server's simulation distance in chunks (`PlayerList.getSimulationDistance`,
    /// `simulation-distance`): how far around each player chunks tick. None off a server open
    /// to players.
    fn server_simulation_distance(&self) -> Option<i32>;
    /// Side: server.
    /// Sets the server's view distance live, as `PlayerList.setViewDistance` (and the
    /// management protocol's `serversettings/view_distance/set`) do: every player is sent
    /// `ClientboundSetChunkCacheRadiusPacket` and their chunks follow, each up to what their
    /// client asked for. `server.properties` is not written, so a restart goes back to it.
    /// False for a distance outside 2 to 32, and off a dedicated server (an integrated
    /// server follows its window's option).
    fn server_set_view_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the server's simulation distance live, as `PlayerList.setSimulationDistance`
    /// does: every player is sent `ClientboundSetSimulationDistancePacket`, and the chunks
    /// around the players tick to the new distance from the next tick. `server.properties`
    /// is not written. False for a distance outside 2 to 32, and off a dedicated server.
    fn server_set_simulation_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the server's tick rate, as `/tick rate` does (`ServerTickRateManager.setTickRate`):
    /// the game ticks `rate` times a second from the next tick on, an autosave wait already
    /// running is shortened when the new rate asks for less, and `server_tick_rate_changed`
    /// fires. The players' clients follow on the next sync. False, with nothing changed, for a
    /// rate outside 1 to 10000 (the command's range), one that isn't finite, a call from
    /// inside `server_tick_rate_changed` (which would hear itself again without end), or on a
    /// client.
    fn set_tick_rate(&mut self, rate: f32) -> bool;
    /// Side: server.
    /// Whether the server's ticking is frozen (`TickRateManager.isFrozen`, `/tick freeze`). A
    /// frozen server still runs the ticks `/tick step` asks for, so `server_tick_start`'s
    /// `world_running` can be true while this is true. None on a client.
    fn tick_rate_is_frozen(&self) -> Option<bool>;
    /// Side: server.
    /// The open world's folder (`world/`, beside its `level.dat`): a server
    /// mod keeps its data in a folder of its own there, so the world's
    /// backups carry it. None when no world is open.
    fn world_dir(&self) -> Option<PathBuf>;
    /// Side: server. The calling mod's bulk files in the current save's
    /// `sunrise-mod-data/<mod id>/`, copied, backed up and deleted with that
    /// save. Paths omit the mod id. Calls refuse with `NoWorld` before a
    /// save is open; this service never outlives its borrowed world host.
    fn world_data_files(&self) -> &dyn ConfigFiles;
    /// Side: server.
    /// Every current rule in the one server-wide store, by namespaced id.
    /// Readable from `server_about_to_start`, before any level exists.
    /// Level game-rule reads use this same store once their level exists.
    fn game_rules(&self) -> Vec<(String, GameRuleValue)>;
    /// Side: server.
    /// Read the server-wide rule from the first server hook onward.
    /// Returns `UnknownRule` for an unknown id; level reads use the same store.
    fn game_rule(&self, id: &str) -> Result<GameRuleValue, GameRuleReadError>;
    /// Side: server.
    /// The retained server reload listeners of the last successful reload.
    fn reload_resources(&self) -> &dyn ServerReloadResources;
}
/// The client host's methods of the `config` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ConfigClientHost {
    /// Side: client.
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    fn translate(&self, key: &str) -> Option<String>;
    /// Side: client.
    /// The calling mod's toggle `key` (`Registrar::register_setting`): the player's choice, else
    /// its default. None when the mod registered no toggle by that key.
    fn setting_bool(&self, key: &str) -> Option<bool>;
    /// Side: client.
    /// The index of the option the player picked for the calling mod's choice `key`, else its
    /// default. None when the mod registered no choice by that key.
    fn setting_choice(&self, key: &str) -> Option<u32>;
    /// Side: client.
    /// The calling mod's range `key`: the player's value, else its default; a kept value outside
    /// the range reads as the default. None when the mod registered no range by that key.
    fn setting_f64(&self, key: &str) -> Option<f64>;
    /// Side: client.
    /// The calling mod's colour `key`, `0xRRGGBB`: the player's choice, else its default. None
    /// when the mod registered no colour by that key.
    fn setting_color(&self, key: &str) -> Option<u32>;
}
