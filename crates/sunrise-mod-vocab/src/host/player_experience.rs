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
    /// progress and the total. An actual change fires `player_level_change`;
    /// this absolute write does not ask `player_xp_level_change`.
    fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// Sets `Player.totalExperience`, the `total` that `PlayerRef::experience`
    /// reports, keeping the level, the progress toward the next level, and the player's score.
    /// Generated as `PlayerRef::set_total_experience`. A mod that moves
    /// experience between a player and a block sets the level and progress
    /// with `PlayerRef::set_experience_levels` and
    /// `PlayerRef::set_experience_points`, and the signed total with this.
    /// False, and nothing written, for a stale or unavailable player.
    fn player_set_total_experience(&mut self, player: PlayerRef, total: i32) -> bool;
    /// Side: server.
    /// `ServerPlayer.giveExperiencePoints`: adds signed points, or takes them when negative.
    /// The signed, hook-adjusted amount also changes the player's score; a level-up sound plays
    /// in the player's own level. Mending runs only at orb pickup. Returns false for a stale or
    /// unavailable player.
    fn player_give_experience_points(&mut self, player: PlayerRef, points: i32) -> bool;
    /// Side: server.
    /// `ExperienceOrb.repairPlayerItems`: spends `points` on the player's damaged equipment
    /// enchanted with `minecraft:repair_with_xp` (Mending), as an orb's pickup does before it
    /// gives the rest. A piece is drawn at random, repaired by its enchantments' xp rate and the
    /// `xp_repair_ratio` hook, `player_item_mend` fires for it, and what is left repairs again
    /// until no damaged piece is left. Returns the points left over, which a mod that took over
    /// the pickup passes to `PlayerRef::give_experience_points`; the player's experience itself
    /// is not changed. A value of zero or below repairs nothing. None for a stale or unavailable
    /// player.
    fn player_repair_items_with_xp(&mut self, player: PlayerRef, points: i32) -> Option<i32>;
    /// Side: server.
    /// `Player.giveExperienceLevels`: adds `levels`, or takes them off when
    /// negative, as the anvil and the enchanting table charge with. Below 0
    /// the level, the total and the progress toward the next level are all 0. A level-up sound
    /// plays in the player's own level; the player's score is unchanged.
    fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
}
