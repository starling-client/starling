//! The `level_weather` area: rain, thunder and lightning. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_weather` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWeatherBackend {
    /// Side: both.
    /// The rain and thunder levels, each 0.0 to 1.0 (`Level.getRainLevel`,
    /// `getThunderLevel` at partial tick 1.0): both 0 in a level with no weather.
    /// Reads any open server level or the client's current synchronized level;
    /// None for a stale handle, an unavailable level or a client not logged in.
    fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)>;
    /// Side: server.
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    fn level_is_raining(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    fn level_is_thundering(&self, level: LevelRef) -> Option<bool>;
}
/// The handles' writes of the `level_weather` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWeatherBackendMut {
    /// Side: server.
    /// Sets the weather as `/weather` does (`ServerLevel.setWeatherParameters`): clear for
    /// `clear_ticks`, or raining (and thundering) for `weather_ticks`, after which the
    /// weather cycle goes on as usual. Rain and thunder then fade in or out as vanilla's do;
    /// `set_weather_levels` sets their strength at once. The `weather_change` hook may cancel
    /// all of it and `thunder_change` the thunder. The players see it after the next sync.
    /// False for a negative time, a call from inside `weather_change` or `thunder_change`, a
    /// parked level, a stale handle or a client.
    fn level_set_weather(
        &mut self,
        level: LevelRef,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
    ) -> bool;
    /// Side: server.
    /// How strong the rain and thunder are now (`Level.setRainLevel`, `setThunderLevel`),
    /// each clamped to 0 to 1 and set without fading. They still drift toward the weather
    /// the level has (`set_weather`) by 0.01 a tick, as vanilla's do. False for a value that
    /// isn't finite, a parked level, a stale handle or a client.
    fn level_set_weather_levels(&mut self, level: LevelRef, rain: f32, thunder: f32) -> bool;
    /// Side: server.
    /// `ServerLevel.findLightningTargetAround`: the closest lightning rod near the column at
    /// `pos`'s `x` and `z`, else a living entity under open sky within 3 blocks of it, picked
    /// with the level random as vanilla does (so this moves later draws), else the top of the
    /// column. Strikes nothing. None for a refused level, a stale or parked handle, a client and
    /// an unloaded column.
    fn level_lightning_target(&mut self, level: LevelRef, pos: [i32; 3]) -> Option<[i32; 3]>;
    /// Side: server.
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    fn level_strike_lightning(&mut self, level: LevelRef, at: [f64; 3], visual_only: bool) -> bool;
}
