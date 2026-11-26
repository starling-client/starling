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
    /// `getThunderLevel` at partial tick 1.0, with thunder scaled by rain): both 0
    /// in a level with no weather. Strengths are local to each level, including
    /// parked levels. Reads any open server level or the client's current
    /// synchronized level; None for a stale handle, an unavailable level or a
    /// client not logged in.
    fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)>;
    /// Side: server.
    /// Whether it rains (`Level.isRaining`: a rain level above the double value `0.2`,
    /// in a dimension that can have weather). This includes a strength of `0.2_f32`.
    fn level_is_raining(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// Whether it thunders in the named server level (`Level.isThundering`): its
    /// own rain-scaled thunder strength exceeds the double value `0.9` in a level
    /// that can have weather. Does not read the shared thunder flag.
    fn level_is_thundering(&self, level: LevelRef) -> Option<bool>;
}
/// The handles' writes of the `level_weather` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWeatherBackendMut {
    /// Side: server.
    /// Sets the server-wide weather parameters (`MinecraftServer.setWeatherParameters`).
    /// `level` names the origin and may be any resident level, including a parked one.
    /// The clear timer takes `clear_ticks`, and both rain and thunder timers take
    /// `weather_ticks`, as signed values without command parsing. Rain and thunder fade
    /// toward the given states; `set_weather_levels` sets their strength immediately.
    /// Returns `false` for an invalid level (including on a client) or when `weather_change`
    /// vetoes the whole write. Refusals and vetoes are logged. A nested write skips callbacks
    /// already running on this thread. A veto does not undo a listener's other writes.
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
    /// each clamped to 0 to 1 (including infinities) and set without fading on the
    /// named resident level, even if it is parked. `ServerLevel.advanceWeatherCycle`
    /// moves the strengths by 0.01 per normal tick toward the server-wide weather.
    /// Does not fire `weather_change`. Returns `false` and logs the refusal for NaN,
    /// a stale handle, a closed level or a client.
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
