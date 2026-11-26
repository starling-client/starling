//! The `level_time` area: the level's game, day and clock time. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_time` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTimeBackend {
    /// Side: client.
    /// The displayed level's configured ticks per second (`ClientLevel.tickRateManager().tickrate()`),
    /// unchanged by freezing or slow frames. None for any other level, a stale handle, outside a
    /// client level, or on the server. Returns `Err(LevelReadError::Unavailable)` when the
    /// host cannot read a client level.
    fn level_tick_rate(&self, level: LevelRef) -> Result<Option<f32>, LevelReadError>;
    /// Side: both.
    /// The level's game time (`Level.getGameTime`). On clients this is the
    /// synchronized time advanced by the local level tick. API 381.
    fn level_game_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: both.
    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000. On a client, the synchronized
    /// Overworld clock, advanced at its server-supplied rate, even when the
    /// displayed dimension has a different default clock. API 381.
    fn level_day_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The time of the level's own clock in ticks: the clock its dimension type names as
    /// `default_clock`, which its sky, spawning and beds go by and `/time` sets there. A mod's
    /// level with a `day_cycle` clock of its own counts it apart from the Overworld's; a level
    /// without a default clock, like the Nether, is None, as is a stale handle and a client.
    fn level_clock_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The next clock time after [`level_clock_time`](Self::level_clock_time) at which the
    /// level's own clock reaches `marker`, a time marker of its timelines (as `/time set <marker>`
    /// moves to it). Today only the Overworld's clock defines markers, so a level following any
    /// other clock (a mod dimension's own) answers None for every marker. None too for a marker
    /// the clock does not define, a time past `i64::MAX`, a level without a default clock, a stale
    /// handle and a client.
    fn level_clock_marker_time(&self, level: LevelRef, marker: &Identifier) -> Option<i64>;
    /// Side: server.
    /// The clock the level follows (`DimensionType.defaultClock`), whose time
    /// [`level_clock_time`](Self::level_clock_time) reads: to set it, run it at a rate of its
    /// own or pause it. None for a level without a default clock (the Nether), a stale handle
    /// and a client.
    fn level_clock(&self, level: LevelRef) -> Option<WorldClockRef>;
}
/// The handles' writes of the `level_time` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTimeBackendMut {
    /// Side: server.
    /// Sets the time of the level's own clock (its dimension type's `default_clock`, the one
    /// `level_clock_time` reads) to `time` ticks, as `/time set <time>` run in that level does:
    /// `time_skip` (cause `mod`) fires first and a mod can keep the old time, and the clock's
    /// fraction of a tick restarts. Players see the new time with the next time sync. A time
    /// the clock already shows is true and changes nothing. False for a negative time, a level
    /// without a default clock, a stale handle, a client, and when a mod kept the old time.
    fn level_set_clock_time(&mut self, level: LevelRef, time: i64) -> bool;
    /// Side: server.
    /// `ServerClockManager.setTotalTicks`: the clock's time becomes `ticks` and its fraction of a
    /// tick restarts, in every level that follows it. Unlike `level_set_clock_time`, no
    /// `time_skip` is heard, as vanilla's own call tells nothing. Players see the new time with
    /// the next time sync. False for a negative time, a stale handle and a client.
    fn world_clock_set_total_ticks(&mut self, clock: WorldClockRef, ticks: i64) -> bool;
    /// Side: server.
    /// `ServerClockManager.setRate`: the clock advances `rate` ticks a game tick from now on
    /// (`ServerClockInstance.tick` adds it to the fraction it keeps), as `/time rate` sets it.
    /// False for a rate that is not finite or below zero, a stale handle and a client.
    fn world_clock_set_rate(&mut self, clock: WorldClockRef, rate: f32) -> bool;
    /// Side: server.
    /// `ServerClockManager.setPaused`: a paused clock keeps its time, a fixed time for the levels
    /// that follow it, until it is resumed, as `/time pause` and `/time resume` do. False for a
    /// stale handle and a client.
    fn world_clock_set_paused(&mut self, clock: WorldClockRef, paused: bool) -> bool;
}
