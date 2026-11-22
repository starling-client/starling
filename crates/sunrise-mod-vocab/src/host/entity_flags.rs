//! The `entity_flags` area: an entity's persistent flags: silent, glowing, no gravity, invisible. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_flags` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFlagsBackend {
    /// Side: server.
    /// Whether the mob makes no sounds (`Entity.isSilent`, its `Silent`
    /// tag); false for a player.
    fn entity_is_silent(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the mob's persistent glowing tag (`Entity.hasGlowingTag`).
    fn entity_glowing(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the mob's no-gravity flag (`Entity.isNoGravity`).
    fn entity_no_gravity(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity's model is hidden (`Entity.isInvisible`): the
    /// invisibility effect on a player or a mob, a mob's or an armor stand's
    /// flag.
    fn entity_is_invisible(&self, entity: EntityRef) -> Option<bool>;
}
/// The handles' writes of the `entity_flags` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFlagsBackendMut {
    /// Side: server.
    /// Silences the mob or lets it make sounds again (`Entity.setSilent`).
    /// False for the other kinds.
    fn entity_set_silent(&mut self, entity: EntityRef, silent: bool) -> bool;
    /// Side: server.
    /// Sets the mob's persistent glowing tag (`Entity.setGlowingTag`). Returns false for players and guests.
    fn entity_set_glowing(&mut self, entity: EntityRef, glowing: bool) -> bool;
    /// Side: server.
    /// Sets the mob's no-gravity flag (`Entity.setNoGravity`). Returns false for players and guests.
    fn entity_set_no_gravity(&mut self, entity: EntityRef, no_gravity: bool) -> bool;
    /// Side: server.
    /// Sets the entity's shared invisibility flag (`Entity.setInvisible`). Returns false for players and guests.
    fn entity_set_invisible(&mut self, entity: EntityRef, invisible: bool) -> bool;
}
