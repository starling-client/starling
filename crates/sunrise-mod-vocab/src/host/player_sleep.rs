//! The `player_sleep` area: sleeping: the timer and the night's sleep count. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_sleep` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerSleepBackend {
    /// Side: server.
    /// Whether `player_set_sleep_ignored` left the player out of the night's sleep count. False
    /// for a player never ignored. None for a stale handle or a player who isn't online.
    fn player_sleep_ignored(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// Whether the player counts in the night's sleep count (`SleepStatus.update`): not a
    /// spectator, not a fake player, and not left out through `player_set_sleep_ignored` while
    /// out of bed. None for a stale or unavailable player.
    fn player_counts_for_sleep(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `Player.getSleepTimer` (`sleepCounter`): the ticks the player has been in bed, 100 once
    /// asleep long enough (`isSleepingLongEnough`, what the night skip waits for), then
    /// counting up 101 to 109 and back to 0 once they are up. None for a stale or unavailable
    /// player.
    fn player_sleep_timer(&self, player: PlayerRef) -> Option<i32>;
}
/// The handles' writes of the `player_sleep` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerSleepBackendMut {
    /// Side: server.
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. False when the server list is unavailable, the handle is
    /// stale or the player isn't online.
    fn player_set_sleep_ignored(&mut self, player: PlayerRef, ignored: bool) -> bool;
    /// Side: server.
    /// Writes the signed `Player.sleepCounter` without lying down or waking up. The next
    /// `Player.tick` handles the counter as usual. False for a stale or unavailable player.
    fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: i32) -> bool;
}
