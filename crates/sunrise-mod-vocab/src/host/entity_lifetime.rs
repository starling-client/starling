//! The `entity_lifetime` area: how long an entity has lived, and removing it. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_lifetime` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityLifetimeBackend {
    /// Side: server.
    /// Whether the entity is in its level: from when `entity_join_level` let
    /// it in (false while that hook runs for it and before) until it
    /// leaves or a mod removes it. Players, and the kinds that hook doesn't
    /// tell (displays, markers, interactions, evoker fangs, area effect
    /// clouds, ominous item spawners), are in it while the handle names them.
    /// False for a stale handle or on a client.
    fn entity_added_to_level(&self, entity: EntityRef) -> bool;
    /// Side: server.
    /// `Entity.getPortalCooldown`: a player's own portal state, the falling
    /// block's, a pearl's End gateway state, or the level's for the other
    /// kinds its portal walk carries (zero when it keeps no state for one).
    fn entity_portal_cooldown(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Ticks the entity has lived (`Entity.tickCount`): players, mobs, item
    /// entities, experience orbs and minecarts. Wraps as vanilla's `int` does.
    fn entity_tick_count(&self, entity: EntityRef) -> Option<i32>;
}
/// The handles' writes of the `entity_lifetime` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityLifetimeBackendMut {
    /// Side: server.
    /// Removes the entity with no death and no loot (`Entity.discard`): a
    /// chest boat or a minecart with a chest still drops what it holds, and
    /// a boat its lead, as vanilla's `remove` does. False for a player, a dragon
    /// part (it leaves with its dragon), or a stale handle. Also returns `false`
    /// if this entity's removal effects are already running, even if its handle still resolves.
    /// A discarded dropped item immediately stops resolving. In an open
    /// transaction the removal is journaled; abort restores the item and
    /// commit delivers its removal notice. Non-item removal is immediate and
    /// is not rolled back by this operation.
    fn entity_discard(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Removes the entity for `reason` (`Entity.remove`): `Discarded`, as
    /// `discard`, or `Killed`, which also runs a mob's effects that act on
    /// removal (Oozing's slimes, Weaving's cobwebs, Wind Charged's burst),
    /// still with no death and no loot. False for a player, a dragon part (it
    /// leaves with its dragon), or a stale handle. Also returns `false` if this
    /// entity's removal effects are already running, even if its handle still resolves.
    /// Removing a general entity dismounts its passengers without removing them.
    /// For dropped items, the same transaction rules apply as for
    /// `entity_discard`.
    fn entity_remove(&mut self, entity: EntityRef, reason: RemovalReason) -> bool;
    /// Side: server.
    /// Sets whether this entity may be written as a root in future world saves,
    /// including chunk unload and a player's saved vehicle. `false` skips the
    /// whole passenger tree when set on its root; setting it on a rider does not
    /// omit that rider from a saved vehicle. `true` restores normal save rules
    /// (the default), not the ability to save types that never save.
    ///
    /// An accepted write establishes the entity's save UUID and returns `Ok(true)`,
    /// even if the flag was already set to the same value. It does not change
    /// existing snapshots, despawn the entity, or keep it loaded. The flag lives
    /// only as long as the entity, including through live dimension travel.
    ///
    /// Returns `Ok(false)` for a player, a dragon part, a handle that does not
    /// resolve in the current level, or a client-side call. Returns
    /// `Err(LevelWriteError::Unavailable)` outside the world phase or without
    /// a world backend.
    fn entity_set_persistent(
        &mut self,
        entity: EntityRef,
        persistent: bool,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Sets the entity's portal cooldown (`Entity.setPortalCooldown`), the
    /// value `entity_portal_cooldown` reads, where that read keeps it: the
    /// level's portal state for the kinds its portal walk carries (mobs,
    /// items, experience orbs, boats, minecarts, primed TNT, fireworks,
    /// tridents, arrows and the thrown items and fireballs a chunk saves).
    /// Server-only; a negative value or a client-side write is refused. Takes
    /// effect on the next tick: a positive value blocks portal entry (and is
    /// refilled while the entity stands in a portal), and 0 lets it enter at
    /// once. Not synced except in the spawn information at login, respawn and
    /// dimension change. Refused by name for a player, an acting guest, a
    /// falling block and an ender pearl, whose cooldown lives in a store of
    /// its own that this write does not own. False for a kind the read
    /// refuses, a stale handle, or on a client.
    fn entity_set_portal_cooldown(&mut self, entity: EntityRef, ticks: i32) -> bool;
}
