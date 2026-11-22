//! The `level_world_border` area: the level's world border. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_world_border` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldBorderBackend {
    /// Side: server.
    /// The level's world border as [min_x, min_z, max_x, max_z], using the
    /// previous size while it moves. Sunrise currently shares one border
    /// across its dimensions. None for a stale handle or a closed level.
    fn level_world_border(&self, level: LevelRef) -> Option<[f64; 4]>;
}
/// The handles' writes of the `level_world_border` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldBorderBackendMut {
    /// Side: server.
    /// Moves the world border's centre (`WorldBorder.setCenter`), as `/worldborder center`
    /// does. Sunrise keeps one border for every level, so this moves it everywhere; the
    /// players see it after the next sync. The centre it already has is true, with nothing
    /// heard. False for a centre that isn't finite or lies past 2.9999984E7 on x or z, a call
    /// from inside `world_border_center_changed`, a stale handle, a level the server has not
    /// opened or a client.
    fn level_set_world_border_center(&mut self, level: LevelRef, center: [f64; 2]) -> bool;
    /// Side: server.
    /// Sets the world border's size, as `/worldborder set`: at once when `ticks` is 0
    /// (`setSize`), otherwise growing or shrinking in a straight line over `ticks` game ticks
    /// from its size now (`lerpSizeBetween`). The one border every level shares. A still
    /// border set to the size it has at once is true, with nothing heard. False for a size
    /// outside 1 to 5.9999968E7, a negative `ticks`, a call from inside
    /// `world_border_bounds_changed`, a stale handle, a level the server has not opened or a
    /// client.
    fn level_set_world_border_size(&mut self, level: LevelRef, size: f64, ticks: i64) -> bool;
    /// Side: server.
    /// The world border's damage (`setDamagePerBlock`, `setSafeZone`), as `/worldborder
    /// damage amount` and `buffer`: a player outside the border and past `safe_zone` blocks
    /// takes `per_block` damage for each block beyond it. False for a value that is
    /// negative or not finite, a stale handle, a level the server has not opened or a client.
    fn level_set_world_border_damage(
        &mut self,
        level: LevelRef,
        per_block: f64,
        safe_zone: f64,
    ) -> bool;
    /// Side: server.
    /// The world border's warning (`setWarningBlocks`, `setWarningTime`), as `/worldborder
    /// warning distance` and `time`: a player's screen tints red within `distance` blocks of
    /// the border, or when a shrinking border will reach them within `ticks` game ticks.
    /// False for a negative value, a stale handle, a level the server has not opened or a
    /// client.
    fn level_set_world_border_warning(
        &mut self,
        level: LevelRef,
        distance: i32,
        ticks: i32,
    ) -> bool;
}
