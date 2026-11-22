//! The `entity_sight` area: what an entity sees: eyes, view and picking. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_sight` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntitySightBackend {
    /// Side: both.
    /// The height of the entity's eyes above its feet (`Entity.getEyeHeight`).
    /// On a client, the local player's predicted dimensions or a tracked entity's own copy.
    fn entity_eye_height(&self, entity: EntityRef) -> Option<f64>;
    /// Side: both.
    /// Where the entity's eyes are (`Entity.getEyePosition`). On a client, the local player's
    /// predicted position and dimensions or a tracked entity's own copy.
    fn entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// The unit vector the entity looks along (`Entity.getViewVector`, from
    /// its pitch and yaw). API 379.
    fn entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// The unit vector that points up out of the entity's head
    /// (`Entity.getUpVector`): its view vector turned a quarter of a turn
    /// toward the zenith, so a level look gives straight up and a look down
    /// leans it forward. From the entity's pitch and yaw.
    fn entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// A ray from the entity's eyes along its view vector, `distance` blocks
    /// long, against block outlines with an empty collision context and no
    /// fluids (`Entity.pick(distance, 1.0F, false)`): what it looks at. A
    /// miss is the hit with `miss` set. This clips blocks only. For entities, use
    /// [`LevelRef::ray_trace_nearest_entity`] for one accepted hit or
    /// [`LevelRef::ray_trace_entities`] for all hits in order. Returns `None` for a
    /// stale handle, a negative or non-finite `distance`, or when `LevelRef::clip`
    /// returns `None`.
    fn entity_pick(&self, entity: EntityRef, distance: f64) -> Option<ClipHit>;
}
