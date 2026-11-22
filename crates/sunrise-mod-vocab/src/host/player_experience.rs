//! The `player_experience` area: experience points and levels. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_experience` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerExperienceBackend {
    /// Side: server.
    /// `Player.takeXpDelay`: the ticks before the player can take the next
    /// experience orb.
    fn player_exp_cooldown(&self, player: PlayerRef) -> Option<u32>;
    /// Side: both.
    /// `Player.experienceLevel`, `experienceProgress` and `totalExperience`.
    fn player_experience(&self, player: PlayerRef) -> Option<PlayerExperience>;
    /// Side: both.
    /// `Player.getXpNeededForNextLevel`: the points the current level needs
    /// to fill.
    fn player_xp_needed_for_next_level(&self, player: PlayerRef) -> Option<i32>;
}
/// The handles' writes of the `player_experience` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerExperienceBackendMut {
    /// Side: server.
    /// Sets `Player.takeXpDelay`: no orb is taken for `ticks` ticks. The
    /// change is not reported to `player_exp_cooldown_change`.
    fn player_set_exp_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperiencePoints`: sets the progress toward the next
    /// level to `points` of the points it needs, kept below a full bar. The
    /// level and the total are kept.
    fn player_set_experience_points(&mut self, player: PlayerRef, points: i32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperienceLevels`: sets the level, keeping the
    /// progress and the total.
    fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// Sets `Player.totalExperience`, the `total` that `PlayerRef::experience`
    /// reports, keeping the level and the progress toward the next level.
    /// Generated as `PlayerRef::set_total_experience`. A mod that moves
    /// experience between a player and a block sets the level and progress
    /// with `PlayerRef::set_experience_levels` and
    /// `PlayerRef::set_experience_points`, and the total with this. False,
    /// and nothing written, for a negative `total` or a stale or unavailable
    /// player.
    fn player_set_total_experience(&mut self, player: PlayerRef, total: i32) -> bool;
    /// Side: server.
    /// `ExperienceOrb.playerTouch`: Mending repairs the player's damaged
    /// items with `points` first (`ExperienceOrb.repairPlayerItems`), then
    /// only the rest goes to their experience
    /// (`Player.giveExperiencePoints`), with the level-up sound. No orb
    /// pickup sound. False, and nothing given, as the other writes; `points`
    /// above `i32::MAX` is refused before Mending repairs anything.
    fn player_give_experience_points(&mut self, player: PlayerRef, points: u32) -> bool;
    /// Side: server.
    /// `Player.giveExperienceLevels`: adds `levels`, or takes them off when
    /// negative, as the anvil and the enchanting table charge with. Below 0
    /// the level, the total and the progress toward the next level are all 0.
    fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
}
