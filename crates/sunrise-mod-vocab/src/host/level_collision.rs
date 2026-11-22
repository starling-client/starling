//! The `level_collision` area: block shapes, collision and rays through them. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_collision` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelCollisionBackend {
    /// Side: both.
    /// `BlockGetter.getBlockFloorHeight` with the normal collision shapes and an empty
    /// entity context. The result is relative to the block: negative infinity means no floor.
    /// None for a stale level, an unloaded column or a position outside build height.
    /// Reads parked server levels and the current client level without loading chunks.
    fn level_block_floor_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f64>;
    /// Side: both.
    /// The supplied state's block-local outline or collision boxes at `pos`,
    /// with an empty context (`BlockState.getShape` / `getCollisionShape`).
    /// The state need not be the block stored there; level-dependent shapes
    /// still read the real level. Pass `block_state(host, pos)` to query the
    /// state at that position. Interaction, visual, occlusion and support shapes
    /// are not exposed. Reads open server levels and the current client level.
    /// Returns None for an unknown state, a stale handle or an unavailable level.
    /// Collision errors return no boxes.
    fn level_block_shape(
        &self,
        level: LevelRef,
        state: BlockState,
        pos: [i32; 3],
        kind: BlockShapeKind,
    ) -> Option<Vec<[f64; 6]>>;
    /// Side: both.
    /// Clip an empty-context ray against collider or outline shapes, without fluids.
    /// None on failed reads, dynamic shapes, invalid rays or traversal limits, and for a ray
    /// that enters a column the level doesn't hold. On a client, the level it shows or a
    /// viewed one (`host.level`), each through its own world.
    fn level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
    ) -> Option<ClipHit>;
    /// Side: both.
    /// `BlockState.isFaceSturdy` in an open server level or the current client level,
    /// including shapes published by local block entities. None for stale or unavailable
    /// levels, unloaded or out-of-height blocks, or a shape query error. Never loads chunks.
    fn level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: BlockFace,
        support: SupportType,
    ) -> Option<bool>;
    /// Side: both.
    /// World-space block collision boxes under an empty collision context, without entities or border. Refuses a stale or nonresident level and invalid or overlarge boxes.
    /// On a client, the level it shows or a viewed one (`host.level`), each through its own
    /// world; None where the walk reaches a column the client doesn't hold.
    fn level_collision_boxes(&self, level: LevelRef, aabb: [f64; 6]) -> Option<Vec<[f64; 6]>>;
    /// Side: server.
    /// Whether nothing solid lies on the segment `from`..`to`, as
    /// `LivingEntity.hasLineOfSight` clips between two eyes (`ClipContext.Block.COLLIDER`,
    /// `ClipContext.Fluid.NONE`): `clip`'s miss. Fluids, entities and the border don't block it.
    /// None as for `clip`.
    fn level_line_of_sight(&self, level: LevelRef, from: [f64; 3], to: [f64; 3]) -> Option<bool>;
    /// Side: both.
    /// Whether any block collision shape shares volume with `aabb` (`[min_x, min_y, min_z, max_x,
    /// max_y, max_z]`): the block half of `CollisionGetter.noCollision(aabb)`, negated. The same
    /// walk and limits as `collision_boxes`, stopping at the first shape it finds; entities and
    /// the world border are not counted. None as for `collision_boxes`.
    fn level_has_collisions(&self, level: LevelRef, aabb: [f64; 6]) -> Option<bool>;
}
