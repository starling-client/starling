//! The `player_cooldowns` area: item cooldowns and the warden warning. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_cooldowns` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerCooldownsBackend {
    /// Side: server.
    /// Every group the player waits on and its ticks left, ordered by
    /// group.
    fn player_item_cooldowns(&self, player: PlayerRef) -> Option<Vec<ItemCooldown>>;
    /// Side: server.
    /// `ItemCooldowns.isOnCooldown`: whether `group` still has time to run.
    fn player_has_item_cooldown(&self, player: PlayerRef, group: &str) -> Option<bool>;
    /// Side: server.
    /// `ItemCooldowns.getCooldownPercent(group, 0)`: what is left of the
    /// group's cooldown, from 1 (just started) to 0 (none).
    fn player_item_cooldown_percent(&self, player: PlayerRef, group: &str) -> Option<f32>;
}
/// The handles' writes of the `player_cooldowns` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerCooldownsBackendMut {
    /// Side: server.
    /// `ItemCooldowns.addCooldown(group, ticks)`: starts (or restarts) the
    /// group's cooldown, and the player's client draws it on every item of the
    /// group (`ServerItemCooldowns.onCooldownStarted`).
    fn player_set_item_cooldown(&mut self, player: PlayerRef, group: &str, ticks: i32) -> bool;
    /// Side: server.
    /// `ItemCooldowns.removeCooldown(group)`: ends the group's cooldown now,
    /// on the client too (`ServerItemCooldowns.onCooldownEnded`).
    fn player_remove_item_cooldown(&mut self, player: PlayerRef, group: &str) -> bool;
    /// Side: server.
    /// Sets the player's `WardenSpawnTracker.cooldownTicks`: sculk shriekers
    /// they set off don't warn for `ticks` ticks. The level's players share
    /// one tracker here, the integrated server owner's (the shriekers read it
    /// for whoever sets them off), so this is every player's.
    /// False, changing nothing, for anyone but the tracker's owner until each player has their own.
    fn player_set_warden_warning_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
}
