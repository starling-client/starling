//! The `entity_physics` area: an entity's box, motion and collision. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_physics` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPhysicsBackend {
    /// Side: both.
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Sunrise doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// How the entity reacts to a piston (`Entity.getPistonPushReaction`), including
    /// the current marker flag of an armor stand. `Ok(None)` for a stale handle;
    /// `Err(LevelReadError::Unavailable)` if the host does not support that read.

    fn entity_piston_push_reaction(
        &self,
        entity: EntityRef,
    ) -> Result<Option<PushReaction>, LevelReadError>;
    /// Side: both.
    /// Whether this is a vanilla `HangingEntity`: a painting, item frame or glow item frame.
    /// Leash knots and cushions are block-attached but do not inherit `HangingEntity`.
    /// `Ok(None)` for a stale handle; `Err(LevelReadError::Unavailable)` if the host
    /// does not support that read.

    fn entity_is_hanging(&self, entity: EntityRef) -> Result<Option<bool>, LevelReadError>;
    /// Side: both.
    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. Displays and markers have zero-sized physical boxes; interactions use
    /// their saved dimensions. End crystals have their 2 x 2 box. None for the dragon; a dragon part's is its own. On a client, reads
    /// its own copy of a tracked entity, or the local player's predicted box.
    fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]>;
    /// Side: server.
    /// The entity's width and height (`Entity.getBbWidth`, `getBbHeight`):
    /// its box's, so a baby's or a scaled entity's own.
    fn entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]>;
    /// Side: server.
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is false (a vex sets it only for
    /// the length of its own tick); an arrow's or a trident's is its
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. The
    /// ender dragon, an area-effect cloud, an ominous item spawner and a
    /// shulker bullet set it in their constructors and never clear it.
    fn entity_no_physics(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The collider the entity carries (`set_collider`), or None for none, a stale handle, or a
    /// client.
    fn entity_collider(&self, entity: EntityRef) -> Option<Collider>;
}
/// The handles' writes of the `entity_physics` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityPhysicsBackendMut {
    /// Side: server.
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Sets the entity's velocity (`Entity.setDeltaMovement`); clients see it
    /// as they see the entity move. False for a player (its client moves
    /// it), an entity Sunrise doesn't simulate, a value that isn't finite,
    /// or on a client. API 379.
    fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool;
    /// Side: server.
    /// Adds `delta` to the entity's velocity (`Entity.push`,
    /// `addDeltaMovement`). False for an unavailable player, an entity
    /// Sunrise doesn't simulate, or a value that isn't finite.
    fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool;
    /// Side: server.
    /// Sets the player's collision exclusions, boxes of its own level
    /// (`[min_x, min_y, min_z, max_x, max_y, max_z]`) in which that level's blocks don't stop it:
    /// the part of a block's collision shape inside a box doesn't collide, a block sharing volume
    /// with one is no wall to suffocate in and doesn't act on the player inside it (cobwebs, powder
    /// snow, damaging blocks), and the server's check of the player's moves reads
    /// the same boxes. Entity colliders and the world border still stop it. Each call replaces
    /// the last set; an empty slice clears it, and so do a change of level, a respawn and
    /// leaving. Nothing is saved, and nothing reaches the player's client: its own half sets the
    /// same boxes with `ClientHost::set_collision_exclusions`. Refused, keeping the set there
    /// was: more than 64 boxes, a value that isn't finite, a minimum above its maximum, an
    /// entity that isn't a player, a stale handle.
    fn entity_set_collision_exclusions(
        &mut self,
        entity: EntityRef,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError>;
    /// Side: server.
    /// Gives the entity boxes that every other entity collides with, or removes them with None.
    /// The boxes are in the entity's frame (its position is the origin), turned by the
    /// collider's rotation; see [`Collider`] for what collides, who is exempt, the limits and
    /// that a vanilla client does not see them. Contact effects (step sounds, sprint particles,
    /// friction, bounce) are not applied yet: a box is solid ground and walls only
    /// (sunrise-org/sunrise#1909). The collider is saved with the entity, sent to
    /// the Sunrise clients that track it when they start to (and then only what changed), and
    /// dropped with the entity. Replacing a collider sends the boxes that were added or removed
    /// by index. A shape edit does not directly displace an overlapping body. On its movement
    /// tick, a Sunrise local player checks solid boxes in the sampled collision scene for the
    /// normal closest-space velocity nudge. Refused by name, with the old collider kept, when
    /// the handle is stale, a value is not finite, the rotation is not a unit quaternion,
    /// there are too many boxes, or a box lies too far from the entity (see
    /// [`ColliderError`]).
    fn entity_set_collider(
        &mut self,
        entity: EntityRef,
        collider: Option<Collider>,
    ) -> Result<(), ColliderError>;
    /// Side: server.
    /// Turns the entity's collider (a unit quaternion `[x, y, z, w]`) without sending its boxes
    /// again: only the rotation reaches the clients that track it. Refused by name for a stale
    /// handle, a rotation that is not finite or not a unit quaternion, or an entity with no
    /// collider.
    fn entity_set_collider_transform(
        &mut self,
        entity: EntityRef,
        rotation: [f32; 4],
    ) -> Result<(), ColliderError>;
    /// Side: server.
    /// Sets the entity's no-physics flag (`Entity.noPhysics`), the value
    /// `entity_no_physics` reads: its moves pass through blocks. Refused, and
    /// nothing written, for a player (`Player.tick` rewrites the flag from the
    /// spectator state every tick), an item entity and a vex (`ItemEntity.tick`
    /// and `Vex.tick` both rewrite it every tick). An arrow's or a trident's
    /// write also updates the synced flag clients read. An armor stand
    /// recomputes the flag on load (`ArmorStand.readAdditionalSaveData`:
    /// `noPhysics = !hasPhysics()`), so its write is not persisted: it lasts
    /// only until the stand is saved and loaded again. The ender dragon, an
    /// area-effect cloud, an ominous item spawner and a shulker bullet take it
    /// too, from the `true` their constructors set. An entity Sunrise keeps
    /// unsimulated keeps the flag in its saved compound. False for those three
    /// kinds, a kind with no writable flag, a stale handle, or on a client.
    fn entity_set_no_physics(&mut self, entity: EntityRef, no_physics: bool) -> bool;
}
