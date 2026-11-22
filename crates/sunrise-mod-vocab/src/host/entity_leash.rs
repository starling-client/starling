//! The `entity_leash` area: leads between an entity and its holder. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_leash` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityLeashBackend {
    /// Side: server.
    /// The live holder of a mob's or boat's lead (`Leashable.getLeashHolder`), or `Unleashed`
    /// when no lead exists. `None` means an entity that takes no lead, or that the entity,
    /// holder or saved holder is unavailable, stale or in a parked level. Does not load a level.
    fn entity_leash_holder(&self, entity: EntityRef) -> Option<LeashHolder>;
}
/// The handles' writes of the `entity_leash` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityLeashBackendMut {
    /// Side: server.
    /// Gives the holder the mob's or boat's lead (`Leashable.setLeashedTo`): the entity lets go
    /// of any other holder and gets off its vehicle, where a mod's `mob_ride` lets it. As
    /// there, neither `canBeLeashed` nor the distance is asked: a holder too far away snaps
    /// the lead on the next tick. False for a stale handle, the entity itself, a holder other
    /// than a player, mob or boat, or an entity that takes no lead.
    fn entity_leash_to(&mut self, entity: EntityRef, holder: EntityRef) -> bool;
    /// Side: server.
    /// Ties the mob's or boat's lead to the fence at `pos`, as `entity_leash_to` does to
    /// an entity; the knot is there while a lead hangs on it. False for a stale handle, an
    /// entity that takes no lead, or no fence at `pos`.
    fn entity_leash_to_knot(&mut self, entity: EntityRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Takes the lead off a mob or boat: the lead falls as an item with `drop_lead`
    /// (`Leashable.dropLeash`), and is gone without it (`removeLeash`). A mob's removal is heard
    /// by `entity_unleash`. False when no holder has the lead, for a stale handle, or for an
    /// entity that takes no lead. A holder still unresolved from a save is left alone.
    fn entity_detach_leash(&mut self, entity: EntityRef, drop_lead: bool) -> bool;
}
