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
    /// Side: both.
    /// Ticks the entity has lived (`Entity.tickCount`). Wraps as vanilla's `int` does.
    /// On a server: living entities, item entities, experience orbs, minecarts and general
    /// entity rows.
    /// On a client: every entity of the level it shows, from the client copy's own
    /// counter. It counts each tick the client level gives the entity
    /// (`Entity.commonTick`) from when the server added it: a frozen level counts only
    /// the players and what carries them, and the local player counts before its own
    /// update, so also before it has loaded and while it sleeps. A root counts at its own
    /// visit, after its `entity_tick_pre` lets it through (not when canceled); a passenger
    /// counts before its own tick, whatever its Pre answers. The new local player of a
    /// respawn or a dimension change starts at 0 when it is admitted, before the server's
    /// first teleport.
    fn entity_tick_count(&self, entity: EntityRef) -> Option<i32>;
    /// Side: both.
    /// Whether the entity is alive (`Entity.isAlive`): not removed, and for a living entity
    /// (`LivingEntity.isAlive`: a player, a mob, an armor stand, the dragon) its health above
    /// 0, so a dying mob reads false from the tick its health reaches 0 while its death
    /// animation still plays. NeoForge mods call the same vanilla method. On a client it reads
    /// the client's copy: the local player's health from the server, another living entity's
    /// synced health (`LivingEntity.DATA_HEALTH_ID`). None, not false, for a stale handle and
    /// for an entity whose body the client doesn't keep.
    fn entity_is_alive(&self, entity: EntityRef) -> Option<bool>;
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
    /// Kills the entity as `/kill` does (vanilla `Entity.kill(ServerLevel)` and
    /// `LivingEntity.kill(ServerLevel)` -> `hurtServer(level, damageSources().genericKill(), Float.MAX_VALUE)`):
    /// a living entity dies and drops its loot, and any other entity is removed
    /// and posts `minecraft:entity_die`. Reaches entities in parked dimensions
    /// as well as the level in place. Returns `Ok(true)` when killed or staged
    /// for the transaction's outermost commit; staging makes the handle stale
    /// until commit or abort, without running death or removal effects.
    /// Returns `Ok(false)` for a live target that refuses the kill (including
    /// disembodied dragon parts or a mob already dying). Returns
    /// `Err(LevelWriteError::Unavailable)` on a client or outside the world phase,
    /// `Err(LevelWriteError::Stale)` for a stale, held-out, or missing handle,
    /// and `Err(LevelWriteError::InTransaction)` when the entity lives in a
    /// parked dimension while a Host transaction is open.
    fn entity_kill(&mut self, entity: EntityRef) -> Result<bool, LevelWriteError>;
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
