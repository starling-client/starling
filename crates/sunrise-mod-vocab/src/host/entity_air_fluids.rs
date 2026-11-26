//! The `entity_air_fluids` area: the fluids an entity touches, and its air. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_air_fluids` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityAirFluidsBackend {
    /// Side: both.
    /// Visits the fluids the entity touched at its last fluid update, with their depths
    /// (NeoForge's `IEntityExtension.getFluidTypeHeight`), highest first, water before lava on a
    /// tie. `Break(())` stops at once and is returned; `Continue(())` means the visit completed,
    /// without a call if the entity touched no fluid. A general entity row's come from its last
    /// base tick on the server.
    ///
    /// The fluids tracked are `minecraft:water` and `minecraft:lava`. On a client this reads the
    /// predicted local player or the tracked mob's last client fluid update. A stale handle
    /// returns [`EntityReadError::Stale`]; another kind or a missing body returns
    /// [`EntityReadError::Unavailable`]. Neither refusal calls `visit`.
    fn entity_visit_fluid_heights(
        &self,
        entity: EntityRef,
        visit: &mut dyn for<'a> FnMut(&'a str, f64) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, EntityReadError>;
    /// Side: both.
    /// The canonical name of the fluid at the entity's eyes from its last fluid update
    /// (NeoForge's `IEntityExtension.getFirstEyeInFluidType`): `Some(None)` in air. Players,
    /// mobs and, on the server, general entity rows; `None` for other kinds and stale handles. On a client this reads the same cached
    /// body as `visit_fluid_heights`.
    fn entity_eye_fluid(&self, entity: EntityRef) -> Option<Option<&'static str>>;
    /// Side: both.
    /// Whether the entity touched water at its last fluid update
    /// (`Entity.isInWater`): players, mobs and, on the server, general entity rows.
    /// On a client, the local player's own fluid state, or a kept entity's (a tracked player
    /// or mob) last client update. None for a stale handle or an entity whose body the client
    /// doesn't keep.
    fn entity_is_in_water(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity is in lava (`Entity.isInLava`): players, mobs and, on the server,
    /// general entity rows.
    /// On a client, the local player's own predicted state, or a kept entity's (a tracked player
    /// or mob) last client update. None for a stale handle or an entity whose body the client
    /// doesn't keep.
    fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `Entity.isUnderWater`: the previous eye-in-water sample AND current water contact,
    /// not just the current eye fluid. The local client player instead follows
    /// `LocalPlayer.isUnderWater`: the saved eye sample alone, even without current water contact.
    /// Players and mobs, using the client's own cached state on a client, and general entity
    /// rows on the server.
    /// None for other kinds or stale handles.
    fn entity_is_under_water(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The entity's air (`Entity.getAirSupply`): players and mobs.
    /// On a client this is the last server-synced air, never predicted. None for a stale
    /// handle or an entity whose body the client doesn't keep.
    fn entity_air_supply(&self, entity: EntityRef) -> Option<i32>;
    /// Side: both.
    /// The most air the entity holds (`Entity.getMaxAirSupply`): 300, a
    /// dolphin's 4800, an axolotl's 6000.
    /// On a client this is the maximum for its kept copy's kind, not a new server query.
    /// None for a stale handle or an entity whose body the client doesn't keep.
    fn entity_max_air_supply(&self, entity: EntityRef) -> Option<i32>;
}
/// The handles' writes of the `entity_air_fluids` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityAirFluidsBackendMut {
    /// Side: server.
    /// Sets the entity's air (`Entity.setAirSupply`).
    fn entity_set_air_supply(&mut self, entity: EntityRef, air: i32) -> bool;
}
