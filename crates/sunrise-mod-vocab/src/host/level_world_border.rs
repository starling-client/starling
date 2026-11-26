//! The `level_world_border` area: the level's world border. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_world_border` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldBorderBackend {
    /// Side: server.
    /// The named level's world border as `[min_x, min_z, max_x, max_z]`, using the
    /// previous size while it moves. Reads a resident level without switching to it.
    /// `None` for a stale handle, a closed level or on a client.
    fn level_world_border(&self, level: LevelRef) -> Option<[f64; 4]>;
}
/// The handles' writes of the `level_world_border` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldBorderBackendMut {
    /// Side: server.
    /// Sets the named level's border center (`WorldBorder.setCenter`). Finite centers
    /// aren't subject to `/worldborder center`'s argument range: the extent itself clips
    /// its edges to the absolute bounds. Every successful write notifies the level's
    /// watchers and fires `world_border_center_changed`, including equal-value writes.
    /// A nested write skips callbacks for this hook already running on this thread.
    /// Returns `false` and logs the refusal for invalid geometry, a stale handle,
    /// a closed level or a client.
    fn level_set_world_border_center(&mut self, level: LevelRef, center: [f64; 2]) -> bool;
    /// Side: server.
    /// Sets the named level's border size at once when `ticks <= 0` (`WorldBorder.setSize`),
    /// or starts a timed change from its current size (`WorldBorder.lerpSizeBetween`).
    /// Accepts finite sizes with a valid extent: after clipping the edges to the
    /// absolute bounds and rounding them out as in `WorldBorder.StaticBorderExtent`,
    /// `Shapes.box` requires the minima not to exceed the maxima. The command's size
    /// range does not apply.
    /// Every success notifies the level's watchers and fires `world_border_bounds_changed`,
    /// including equal-value writes. A timed write to the current size sends a lerp notice
    /// but does not start a moving extent. A nested write skips callbacks for this hook
    /// already running on this thread. Returns `false` and logs the refusal for invalid
    /// geometry, a stale handle, a closed level or a client.
    fn level_set_world_border_size(&mut self, level: LevelRef, size: f64, ticks: i64) -> bool;
    /// Side: server.
    /// Sets the named level's border damage and safe zone (`WorldBorder.setDamagePerBlock`
    /// and `setSafeZone`). Accepts signed finite values. These settings aren't sent to
    /// clients. Returns `false` and logs the refusal for a non-finite value, a stale handle,
    /// a closed level or a client.
    fn level_set_world_border_damage(
        &mut self,
        level: LevelRef,
        per_block: f64,
        safe_zone: f64,
    ) -> bool;
    /// Side: server.
    /// Sets the named level's border warning distance (blocks) and time (ticks) through
    /// `WorldBorder.setWarningBlocks` and `setWarningTime`. Accepts signed values and sends
    /// each setting to that level's watchers, even if the values are unchanged. Returns
    /// `false` and logs the refusal for a stale handle, a closed level or a client.
    fn level_set_world_border_warning(
        &mut self,
        level: LevelRef,
        distance: i32,
        ticks: i32,
    ) -> bool;
}
