//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `raids` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait RaidsBackend {
    /// Side: server.
    /// `Raid.getCenter`.
    fn raid_center(&self, raid: RaidRef) -> Option<[i32; 3]>;
    /// Side: server.
    /// `Raid.getRaidOmenLevel`: 0 to 5 in play, any value a mod set.
    fn raid_omen_level(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// `Raid.getGroupsSpawned`: the waves spawned so far.
    fn raid_groups_spawned(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// `Raid.numGroups`, set from `getNumGroups(difficulty)` when the raid
    /// began: 0, 3, 5 or 7. A raid of omen level 2 or more adds a bonus wave
    /// (`hasBonusWave`).
    fn raid_num_groups(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// Whether the omen level is above 1, which adds a wave after the last
    /// (`Raid.hasBonusWave`).
    fn raid_has_bonus_wave(&self, raid: RaidRef) -> Option<bool>;
    /// Side: server.
    /// `Raid.isStarted`.
    fn raid_is_started(&self, raid: RaidRef) -> bool;
    /// Side: server.
    /// `Raid.isActive`: false once stopped.
    fn raid_is_active(&self, raid: RaidRef) -> bool;
    /// Side: server.
    /// `Raid.RaidStatus`.
    fn raid_status(&self, raid: RaidRef) -> Option<RaidStatus>;
    /// Side: server.
    /// `Raid.getTotalHealth`: the health of every raider that joined, which
    /// the boss bar divides by.
    fn raid_total_health(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// `Raid.getHealthOfLivingRaiders`.
    fn raid_living_health(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// The boss bar's progress, 0 to 1 (`Raid.raidEvent.getProgress`).
    fn raid_bar_progress(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// `Raid.getAllRaiders`: visits the living raiders that joined, in mob-list order,
    /// as entity handles. A stopped raid remains readable until the raid map removes it.
    /// Return `ControlFlow::Break(())` to stop. An empty raid completes with
    /// `Ok(Continue(()))`; a refusal does not call the visitor.
    fn raid_visit_raiders(
        &self,
        raid: RaidRef,
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, RaidReadError>;
    /// Side: server.
    /// Visits the living raiders of one wave (`groupRaiderMap.get(wave)`), in mob-list order.
    /// Return `ControlFlow::Break(())` to stop. A wave with no living raiders completes
    /// with `Ok(Continue(()))`; a refusal does not call the visitor.
    fn raid_visit_wave_raiders(
        &self,
        raid: RaidRef,
        wave: i32,
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, RaidReadError>;
    /// Side: server.
    /// `Raid.heroesOfTheVillage`: visits the UUIDs of the players who got Hero of the Village,
    /// online or not, in the order vanilla's set iterates them. Return `ControlFlow::Break(())`
    /// to stop. An empty set completes with `Ok(Continue(()))`; a refusal does not call the visitor.
    fn raid_visit_heroes(
        &self,
        raid: RaidRef,
        visit: &mut dyn FnMut(u128) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, RaidReadError>;
}
/// The handles' writes of the `raids` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait RaidsBackendMut {
    /// Side: server.
    /// `Raid.setRaidOmenLevel`: any value, as vanilla assigns it (only
    /// `absorbRaidOmen` clamps, to 0 to 5). The raid is saved with it.
    fn raid_set_omen_level(&mut self, raid: RaidRef, level: i32) -> bool;
    /// Side: server.
    /// `Raid.stop`: the raid ends as `stopped` and its boss bar leaves the
    /// players; the `raid_state_change` hook fires. False if the raid is
    /// already stopped.
    fn raid_stop(&mut self, raid: RaidRef) -> bool;
}
