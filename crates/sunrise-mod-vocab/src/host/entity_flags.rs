//! The `entity_flags` area: an entity's persistent flags: silent, glowing, no gravity, invisible. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_flags` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFlagsBackend {
    /// Side: server.
    /// Whether the entity makes no sounds (`Entity.isSilent`, its `Silent` tag), for every
    /// kind of entity.
    fn entity_is_silent(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the persistent glowing tag (`Entity.hasGlowingTag`), for every kind of entity.
    fn entity_glowing(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the no-gravity flag (`Entity.isNoGravity`), for every kind of entity.
    fn entity_no_gravity(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity's model is hidden (`Entity.isInvisible`): the
    /// invisibility effect on a player or a mob, the flag of a mob, an armor
    /// stand or an item frame.
    fn entity_is_invisible(&self, entity: EntityRef) -> Option<bool>;
}
/// The handles' writes of the `entity_flags` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFlagsBackendMut {
    /// Side: server.
    /// Silences the entity or lets it make sounds again (`Entity.setSilent`), for every
    /// kind of entity. Saved as `Silent`. False for a stale handle.
    fn entity_set_silent(&mut self, entity: EntityRef, silent: bool) -> bool;
    /// Side: server.
    /// Sets the persistent glowing tag (`Entity.setGlowingTag`), saved as `Glowing`, for
    /// every kind of entity. False for a stale handle.
    fn entity_set_glowing(&mut self, entity: EntityRef, glowing: bool) -> bool;
    /// Side: server.
    /// Sets the no-gravity flag (`Entity.setNoGravity`), saved as `NoGravity`, for every
    /// kind of entity. False for a stale handle.
    fn entity_set_no_gravity(&mut self, entity: EntityRef, no_gravity: bool) -> bool;
    /// Side: server.
    /// Sets the entity's shared invisibility flag (`Entity.setInvisible`) on a mob, an armor
    /// stand or an item frame. Returns false for players, guests and other kinds.
    fn entity_set_invisible(&mut self, entity: EntityRef, invisible: bool) -> bool;
}
