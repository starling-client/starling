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
    /// The game's `config/` folder beside `options.txt` (beside
    /// `server.properties` on a dedicated server), as on NeoForge
    /// (`FMLPaths.CONFIGDIR`). Shared by every mod; mods read and write their
    /// own config files with ordinary `std::fs` calls. Nothing is created here.
    fn config_dir(&self) -> &std::path::Path;
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
    /// Side: both.
    /// Read this side's live value of an owner-bound setting registered through
    /// [`SettingsRegistrar::register_setting`](crate::SettingsRegistrar::register_setting).
    /// Writes are the side's own (`set_setting` on `ClientHost` or `ServerHost`).
    fn setting(&self, key: SettingKey) -> Result<SettingValue, SettingAccessError>;
    /// Side: both.
    /// Read a toggle setting through its owner-bound [`SettingKey`].
    fn setting_bool(&self, key: SettingKey) -> Result<bool, SettingAccessError>;
    /// Side: both.
    /// Read a choice setting's selected index through its owner-bound [`SettingKey`].
    fn setting_choice(&self, key: SettingKey) -> Result<u32, SettingAccessError>;
    /// Side: both.
    /// Read a range setting's number through its owner-bound [`SettingKey`].
    fn setting_f64(&self, key: SettingKey) -> Result<f64, SettingAccessError>;
    /// Side: both.
    /// Read a colour setting (`0xRRGGBB`) through its owner-bound [`SettingKey`].
    fn setting_color(&self, key: SettingKey) -> Result<u32, SettingAccessError>;
    /// Side: both.
    /// The packages enabled and admitted at startup, in dispatch order (NeoForge's
    /// `ModList.get()`): the view registration reads through
    /// [`RegistrationContext::mods`](crate::RegistrationContext::mods). `mods().get(id)` is one
    /// package's manifest facts, its [`name`](crate::PackageInfo::name) and
    /// [`version`](crate::PackageInfo::version) among them (`IModInfo.getVersion`); `None` for a
    /// package that isn't installed or was off at startup. `name` is empty when the manifest
    /// gives none: `mods().display_name(id)` is the name to show (`IModInfo.getDisplayName`),
    /// the id in that case, and `Minecraft` for `minecraft`. The game itself is not a package:
    /// `mods().get("minecraft")` is `None` and its version is
    /// [`game_version`](Self::game_version). The view is shared: cloning it copies no package.
    #[published(startup_mods)]
    fn mods(&self) -> LoadedMods;
}
/// The server host's methods of the `config` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ConfigServerHost {
    /// Persist and publish a setting on this server and synchronously notify its owner.
    /// Returns `Ok(true)` if the value changed (`Range` values compare by bits),
    /// or `Ok(false)` if it was already equal.
    fn set_setting(
        &mut self,
        key: SettingKey,
        value: SettingValue,
    ) -> Result<bool, SettingAccessError>;
    /// Startup opt-in for test-only content. Test registration runs regardless.
    fn game_tests_enabled(&self) -> bool;
    /// Side: server.
    /// The server's tick rate (20 by default) and the
    /// milliseconds its ticks took on average lately, as `/tick query`
    /// reports them. The ticks a second it keeps up is the rate, or
    /// 1000 / milliseconds when that is lower.
    fn tick_stats(&self) -> (f32, f64);
    /// Side: server.
    /// The last 100 server ticks' durations in nanoseconds, as
    /// `MinecraftServer.getTickTimesNanos` lays them out: the tick numbered `n` at index
    /// `n % 100`, 0 where none was recorded yet. The visit is in slot order; return `Break(())`
    /// to stop, or collect the values to sort them for percentiles. `tick_stats` has the average.
    /// A host without a server clock returns `Unavailable`.
    fn visit_tick_times(
        &self,
        visit: &mut dyn FnMut(u64) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, ServerReadError>;
    /// Side: server.
    /// The live view distance in chunks advertised by `PlayerList.getViewDistance`.
    /// Available on dedicated and integrated servers, including unpublished worlds;
    /// `None` when no logical server player list is available.
    fn server_view_distance(&self) -> Option<i32>;
    /// Side: server.
    /// The live simulation distance in chunks advertised by `PlayerList.getSimulationDistance`.
    /// Available on dedicated and integrated servers, including unpublished worlds;
    /// `None` when no logical server player list is available.
    fn server_simulation_distance(&self) -> Option<i32>;
    /// Side: server.
    /// Whether the server is open to players other than a single-player world's owner
    /// (`MinecraftServer.isPublished`): always true on a dedicated server; on a single-player
    /// world's server, true while it is open to LAN (the Open to LAN screen or `/publish`) and
    /// false again once that ends (`/unpublish`). No hook fires when it changes: read it when
    /// the answer matters.
    fn server_is_published(&self) -> bool;
    /// Side: server.
    /// Sets the live view distance through `PlayerList.setViewDistance` on either server,
    /// including unpublished worlds. Players get the new chunk radius, and chunk tracking
    /// and loading tickets use the new distance. Returns `false` and logs the refusal if
    /// the player list is unavailable or the input is outside `ChunkMap`'s 2–32 supported range.
    /// The integrated server follows the owner's render distance at the end of an unpaused tick.
    /// A view-distance write from a server-tick listener can be superseded at the end of that tick.
    /// This does not change `server.properties` or the owner's options.
    fn server_set_view_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the live simulation distance through `PlayerList.setSimulationDistance` on either
    /// server, including unpublished worlds. Players get the new distance and simulation tickets
    /// use it. Returns `false` and logs the refusal if the player list is unavailable or the input
    /// is outside the 2–32 supported endpoints of `Options.simulationDistance`.
    /// An unchanged owner option does not override this write; a changed one does.
    /// This does not change `server.properties` or the owner's options.
    fn server_set_simulation_distance(&mut self, distance: i32) -> bool;
    /// Side: server.
    /// Sets the logical server's tick rate (`ServerTickRateManager.setTickRate`).
    /// `TickRateManager.setTickRate` raises finite values below 1 to 1; `/tick rate`'s upper
    /// bound is only a command argument limit. Every accepted write, even an equal one, fires
    /// `server_tick_rate_changed`, shortens the autosave wait if needed, then sends the final
    /// rate and freeze state to clients. A nested write skips callbacks for this hook already
    /// running on this thread. A non-finite rate or a call off the logical server returns
    /// `false` and logs the refusal without changing the clock.
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
    /// Side: server.
    /// Visit the available rules in the server-wide store, with their borrowed namespaced IDs
    /// and typed values. Readable from `server_about_to_start`, before any level exists; level
    /// visits use the same store. Return `Break(())` to stop. An unavailable rule store returns
    /// `Unavailable`.
    fn visit_game_rules(
        &self,
        visit: &mut dyn FnMut(&str, GameRuleValue) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, ServerReadError>;
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
    /// Persist and publish a setting, update Options > Mods, and synchronously notify its owner.
    /// Returns `Ok(true)` if the value changed (`Range` values compare by bits),
    /// or `Ok(false)` if it was already equal.
    fn set_setting(
        &mut self,
        key: SettingKey,
        value: SettingValue,
    ) -> Result<bool, SettingAccessError>;
    /// Side: client.
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    fn translate(&self, key: &str) -> Option<String>;
    /// Side: client.
    /// Sets the calling mod's own translation entries for language `code` (`en_us`, `de_de`) from
    /// its own data, such as language files a pack keeps under `config/`: `translate` components,
    /// tooltips, chat and other mods' screens then resolve these keys like any pack's. They lie
    /// over the resource packs' entries (a key a pack also has shows the mod's text) and are
    /// applied again after every resource reload and language change; the selected language
    /// falls back to the mod's `en_us` entries for a key it lacks, as pack files do. Each call
    /// replaces this mod's earlier entries for `code`; an empty list drops them. Mods apply in id
    /// order, so of two mods setting one key the later id wins. Returns the number of entries
    /// now set, or `None`, with nothing changed, outside a mod's call, for a `code` that isn't
    /// 1 to 16 characters of `a`-`z`, `0`-`9` and `_`, or for more than 65,536 entries, a key over
    /// 256 bytes or a text over 32,767 bytes. Client only; a server resolves no translations.
    fn set_translations(&self, code: &str, entries: Vec<(String, String)>) -> Option<usize>;
    /// Side: client.
    /// Drops every translation entry the calling mod set with `set_translations`, in every
    /// language. Nothing happens outside a mod's call.
    fn clear_translations(&self);
}
