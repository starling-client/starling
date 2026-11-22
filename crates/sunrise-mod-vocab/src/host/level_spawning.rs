//! The `level_spawning` area: natural spawning limits and flags, and spawning an entity. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_spawning` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSpawningBackend {
    /// Side: server.
    /// The level's natural spawning limits by category ([`SpawnLimits`]):
    /// [`SpawnLimits::VANILLA`] until a mod sets others. Reads parked levels.
    /// None on a client and for a stale handle.
    fn level_spawn_limits(&self, level: LevelRef) -> Option<SpawnLimits>;
    /// Side: server.
    /// Which categories the level's natural spawning may spawn ([`SpawnFlags`]):
    /// both on until a mod turns one off. Reads parked levels. None on a
    /// client and for a stale handle.
    fn level_spawn_flags(&self, level: LevelRef) -> Option<SpawnFlags>;
    /// Side: server.
    /// `ServerLevel.canSpawnEntitiesInChunk`: whether natural spawning may place
    /// a mob in chunk `pos` (`[x, z]`), the question it asks of every spawn
    /// position outside the chunk it started in. Sunrise's answer is the one
    /// its spawner uses: the chunk is loaded. Reads parked levels and loads
    /// nothing. None on a client and for a stale handle.
    fn level_can_spawn_entities_in_chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<bool>;
}
/// The handles' writes of the `level_spawning` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSpawningBackendMut {
    /// Side: server.
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos` in the level as
    /// `EntityType.spawn` does with `reason`: the entity type creates it (its
    /// Parts and attachments with it, and `entity_constructing`), with a random
    /// yaw from the level random, then `finalize_spawn` and `finalizeSpawn`
    /// with that reason (equipment, variants, jockeys), and it joins the level
    /// (`entity_join_level`), not persistent, as a natural spawn joins. The same
    /// living route as [`SpawnBody::Living`] through `level_spawn_entity`, at rest. The
    /// level's spawn limits, flags and spawn rules don't apply, as they don't
    /// to `/summon`. The new mob as a handle. None, with nothing spawned, for a
    /// kind that isn't a mob or is disallowed in Peaceful at that difficulty,
    /// a position that isn't finite or is outside the
    /// spawnable bounds (`Level.isInSpawnableBounds`), an unloaded chunk, a
    /// stale handle, a level not in place, a client, a `finalize_spawn` that
    /// cancelled the spawn or a mob a mod kept out.
    /// Inside a transaction its join waits for the root commit, so a mod keeping the mob out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    fn level_spawn_mob(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        reason: SpawnReason,
    ) -> Option<EntityRef>;
    /// Side: server.
    /// Spawns an entity of `kind` at `pos` in the level, as `spawn` describes
    /// ([`EntitySpawn`]): a thrown item projectile (`minecraft:snowball` or a
    /// content thrown type), a mob (`minecraft:zombie` or a content living
    /// type) or a code-declared general type, whichever `spawn.body` says.
    /// Each goes the way the game's own spawns of that family go:
    ///
    /// - [`SpawnBody::Thrown`]: `Projectile.spawnProjectile`, the route a
    ///   thrown snowball takes. The owner is set before it joins, as
    ///   `Projectile.setOwner` (hit credit, `Owner`, and the shot keeps clear
    ///   of its owner until it leaves the owner's box); `projectile_launch`
    ///   fires. It carries `item` (`ThrowableItemProjectile.setItem`), checked
    ///   as `ServerHost::spawn_projectile_from_rotation` checks its stack. The
    ///   owner is a player, a mob, a boat or a minecart: the shot needs its
    ///   collision identity to keep clear of it, and any other owner refuses
    ///   the spawn.
    /// - [`SpawnBody::Living`]: `EntityType.spawn` with `reason`, the route
    ///   `/summon` takes: the entity type creates it (its Parts and
    ///   attachments with it, and `entity_constructing`), with a random yaw
    ///   from the level random unless `spawn.rotation` names one, then
    ///   `finalize_spawn` and `finalizeSpawn` with that reason (equipment,
    ///   variants, jockeys), and it joins the level (`entity_join_level`), not
    ///   persistent. In Peaceful, a type disallowed at that difficulty is refused
    ///   before construction. A mob takes no owner. `kind` resolves as `/summon`'s
    ///   (`cow` is `minecraft:cow`); the ender dragon takes its own creation,
    ///   facing `spawn.rotation`'s yaw, and is refused with a velocity or a
    ///   pitch, which it has no state for.
    /// - [`SpawnBody::General`]: a new row of the general type, with a fresh
    ///   UUID and the type's declared payload version, through the insertion
    ///   `/summon` and chunk loading share. The owner is kept on the row and
    ///   saved (`Owner`); [`EntityRef::owner`] reads it back, for the type's
    ///   own tick to skip its shooter and credit its damage. The row's state is
    ///   its attachments: write them on the returned handle in the same
    ///   transaction, and the join and the first tracking snapshot see them.
    ///
    /// Velocity is set before the entity joins (`Entity.setDeltaMovement`).
    /// The level's spawn limits, flags and spawn rules don't apply, as they
    /// don't to `/summon`; nor does a general type's `summonable`, which is
    /// `/summon`'s alone (`EntityType.canSummon`). The new entity as a handle.
    /// None, with nothing spawned and no random drawn, for a kind that isn't
    /// of `spawn.body`'s family, a position, velocity or rotation that isn't
    /// finite, a position outside the spawnable bounds
    /// (`Level.isInSpawnableBounds`), an unloaded chunk, an owner that is
    /// stale, in another level, given for a mob or without a collision identity
    /// for a thrown spawn, an invalid thrown item, a
    /// stale handle, a level not in place, a client, or exhausted entity ids;
    /// and, with nothing left in the level, a `finalize_spawn` that cancelled
    /// the spawn or an entity a mod kept out. Inside a transaction its join
    /// waits for the root commit, so a mod keeping the entity out is not known
    /// at the call: the handle is returned and reads stale from the commit on.
    fn level_spawn_entity(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        spawn: &EntitySpawn,
    ) -> Option<EntityRef>;
    /// Side: server.
    /// Sets the level's natural spawning limits ([`SpawnLimits`]), from the next
    /// spawn pass on. They hold until the world closes, while the level is
    /// parked and across its closing and opening again, and are not saved: the
    /// world opens with vanilla's, so a mod sets its own in `level_load`. Mobs
    /// already in the level stay. False, with nothing set, on a client and for
    /// a stale handle or a level that isn't open.
    fn level_set_spawn_limits(&mut self, level: LevelRef, limits: SpawnLimits) -> bool;
    /// Side: server.
    /// Sets which categories the level's natural spawning may spawn
    /// ([`SpawnFlags`]), from the next spawn pass on. Kept and refused as
    /// `set_spawn_limits`.
    fn level_set_spawn_flags(&mut self, level: LevelRef, flags: SpawnFlags) -> bool;
}
