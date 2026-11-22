//! The `entity_air_fluids` area: the fluids an entity touches, and its air. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_air_fluids` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityAirFluidsBackend {
    /// Side: both.
    /// The fluids the entity touched at its last fluid update, each with how deep its box is in
    /// it (NeoForge's `IEntityExtension.getFluidTypeHeight`), highest first; empty in no fluid.
    /// The one read behind `isInFluidType` in all its forms (any fluid, one type, a predicate
    /// over this list) and `getMaxHeightFluidType` (the first entry). Fluids are named
    /// `minecraft:water` and `minecraft:lava`, the ones the entity update tracks. Players and
    /// mobs; None for other kinds and stale handles. On a client, reads its own copy: predicted
    /// for the local player, the last client fluid update for entities the server showed it.
    fn entity_fluid_heights(&self, entity: EntityRef) -> Option<Vec<(String, f64)>>;
    /// Side: both.
    /// The fluid at the entity's eyes from its last fluid update (NeoForge's
    /// `IEntityExtension.getFirstEyeInFluidType`; `isEyeInFluidType` compares it):
    /// `Some(None)` in air. Players and mobs; None for other kinds and stale handles. On a
    /// client, reads its own copy as `entity_fluid_heights` does.
    fn entity_eye_fluid(&self, entity: EntityRef) -> Option<Option<String>>;
    /// Side: both.
    /// Whether the entity touched water at its last fluid update
    /// (`Entity.isInWater`): players and mobs.
    /// On a client, the local player's own fluid state, or a kept entity's (a tracked player
    /// or mob) last client update. None for a stale handle or an entity whose body the client
    /// doesn't keep.
    fn entity_is_in_water(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity is in lava (`Entity.isInLava`): players and mobs.
    fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `Entity.isUnderWater`: the previous eye-in-water sample AND current water contact,
    /// not just the current eye fluid. The local client player instead follows
    /// `LocalPlayer.isUnderWater`: the saved eye sample alone, even without current water contact.
    /// Players and mobs, using the client's own cached state on a client.
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
