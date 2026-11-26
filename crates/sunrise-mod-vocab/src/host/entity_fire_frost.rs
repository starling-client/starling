//! The `entity_fire_frost` area: burning and freezing. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_fire_frost` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFireFrostBackend {
    /// Side: both.
    /// `Entity.isOnFire`: not fire immune, and positive remaining fire ticks OR the
    /// client's synced shared flag 0. Every kind of entity, and every entity a client
    /// tracks. Reads this side's own immunity and fire state. None for a stale handle.
    fn entity_is_on_fire(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `Entity.fireImmune`: type immunity or the live grant on this side. Separate
    /// species-specific damage rules do not turn on the grant. Every kind of entity:
    /// the type's own immunity (`EntityType.fireImmune`, a general entity's
    /// `fire_immune` declaration) counts. `None` for a stale handle.
    fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The ticks the entity burns for (`Entity.getRemainingFireTicks`), for every kind of
    /// entity. `None` for a stale handle, or a kind that keeps no entity data.
    fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Returns the entity's frozen ticks (`Entity.getTicksFrozen`).
    fn entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32>;
}
/// The handles' writes of the `entity_fire_frost` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFireFrostBackendMut {
    /// Side: server. Grants a player, mob or armor stand fire immunity or clears its grant.
    /// The last write wins; `false` cannot remove built-in immunity. Granting silently
    /// clears any current burn. The grant belongs to this live body, is not saved, and
    /// is lost when the body is replaced, including on respawn. It survives a level
    /// transfer that carries the same body. Mods should recompute it from worn equipment.
    /// Returns `false` for a stale or unsupported entity, or on a client.
    fn entity_set_fire_immunity(&mut self, entity: EntityRef, immune: bool) -> bool;
    /// Side: server.
    /// Sets the remaining fire ticks (`Entity.setRemainingFireTicks`), for every kind of
    /// entity. Kinds saved as entity data keep it as the short `Fire`, so a value beyond
    /// 16 bits does not survive a save. False for a stale handle, or a kind that keeps no
    /// entity data.
    fn entity_set_remaining_fire_ticks(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// Clears positive fire ticks while retaining fire immunity (`Entity.clearFire`), for
    /// every kind of entity.
    fn entity_clear_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// `Entity.extinguishFire`: plays the ordinary extinguish sound when the
    /// entity is burning, then clears positive fire ticks. Fire-immune mobs
    /// and fire-resistant dropped stacks do not play a sound; negative
    /// fire-immunity countdowns are retained.
    /// Uses the engine's entity/cosmetic random and normal sound route.
    /// Every kind of entity; other kinds than players, mobs, armor stands and dropped
    /// items use the neutral sound source. False for a stale handle.
    /// Unlike this operation, `clear_fire` is always silent and draws no random.
    fn entity_extinguish_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Sets the entity's frozen ticks (`Entity.setTicksFrozen`).
    fn entity_set_ticks_frozen(&mut self, entity: EntityRef, ticks: i32) -> bool;
}

// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityFireFrostClientBackendMut {
    /// Side: client. Sets or clears the same additive grant on this client's own copy
    /// of a player (including its local player), mob or armor stand. It affects local
    /// prediction and drawing only, preserving remote entities' synced fire state so it
    /// is visible again if the grant is revoked. The grant is not sent to the server and
    /// ends when the client discards that entity copy. Returns `false` for unsupported entities,
    /// stale handles and outside a client world.
    fn entity_set_client_fire_immunity(&mut self, entity: EntityRef, immune: bool) -> bool;
}
