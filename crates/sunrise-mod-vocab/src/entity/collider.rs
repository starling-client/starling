//! A moving collider: boxes an entity carries that other entities collide with, and why one is refused.

/// One box of a [`Collider`], axis-aligned in the collider's own frame.
///
/// `min` and `max` are `[x, y, z]` in blocks from the entity's position, before the collider's
/// pose; a box whose minimum is above its maximum on an axis is read with the two swapped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal, and the two corners are all there is to a box
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ColliderBox {
    /// The corner with the smaller coordinates, `[x, y, z]`.
    pub min: [f64; 3],
    /// The corner with the larger coordinates, `[x, y, z]`.
    pub max: [f64; 3],
}

/// How a collider's boxes sit in the world: turned by `rotation` about `pivot`.
///
/// World point = `entity_position + pivot + rotation * (local_point - pivot)`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ColliderPose {
    /// A quaternion `[x, y, z, w]`; normalized when set. `[0.0, 0.0, 0.0, 1.0]` is no turn.
    pub rotation: [f32; 4],
    /// The point the boxes turn about, in blocks from the entity's position.
    pub pivot: [f64; 3],
}

impl Default for ColliderPose {
    /// No turn and pivot at the entity's position.
    fn default() -> Self {
        Self {
            rotation: [0.0, 0.0, 0.0, 1.0],
            pivot: [0.0, 0.0, 0.0],
        }
    }
}

impl ColliderPose {
    /// A pose with `rotation` about `pivot`.
    pub fn new(rotation: [f32; 4], pivot: [f64; 3]) -> Self {
        Self { rotation, pivot }
    }

    /// Replaces the rotation.
    pub fn with_rotation(mut self, rotation: [f32; 4]) -> Self {
        self.rotation = rotation;
        self
    }

    /// Replaces the pivot.
    pub fn with_pivot(mut self, pivot: [f64; 3]) -> Self {
        self.pivot = pivot;
        self
    }
}

/// Boxes an entity carries, turned by a pose, that every other entity collides with.
///
/// Set with `EntityRef::set_collider`. The boxes are in the entity's local frame, positioned
/// and turned by [`ColliderPose`] (`entity_position + pivot + rotation * (local - pivot)`).
/// Other entities collide with the boxes as solid ground and walls: an entity's own box is swept
/// against each turned box (a separating-axis test with the time of impact), it steps up onto a
/// box's top within its step height, and the server's movement check sees the same boxes. A
/// collider is solid to every mover except spectators, entities with no physics and the entities
/// in its own vehicle stack.
///
/// A collider that moves or turns carries the bodies standing on it: once a tick, after the
/// level's entities ticked, each moves by the displacement of its contact point as a collided
/// move with `MoveCause::Carried`. After carrying, a collider that moved or turned shoves
/// bodies in the simultaneous swept path of its boxes, with the same move cause: a turn is swept
/// along with any translation, not resolved at its final pose alone. Riders already carried are
/// not shoved again, and a body caught between the collider and a wall stays where the move
/// leaves it.
///
/// A shove that actually moves a body removes only the component of its velocity that points
/// into the shove; tangential velocity stays. An upward shove never lowers vertical velocity
/// below the collider's own vertical speed: a body already rising faster keeps its speed.
/// Vanilla pistons and shulkers keep their velocity behaviour.
///
/// The server shoves non-player entities and never a player; each Sunrise client predicts the
/// shove of its own player through the ordinary movement checks. A living body still takes `in_wall`
/// damage while a box covers its eyes. Passengers follow their vehicle.
///
/// A vanilla client has none of this: the server sends a collider only to a Sunrise client that
/// tracks the entity, and a vanilla client sees the entity and walks through it.
///
/// Checked when it is set, and never clamped or truncated; a collider that fails a check is
/// refused by name with a [`ColliderError`] and the old one stays:
/// - every box value and pivot coordinate is finite;
/// - the rotation is a finite, non-zero quaternion (normalized when set);
/// - the collider's wire encoding fits in one clientbound custom payload
///   (`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`, 1,048,576 bytes).
///
/// A box of 0 thickness is a box with no volume that nothing collides with.
///
/// **Contact effects are not applied yet.** A box is solid ground and walls and nothing more:
/// standing or walking on one has no block's step sound, sprint particles, friction or bounce
/// (tracked in sunrise-org/sunrise#1909).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Collider {
    /// The boxes, in the entity's frame.
    pub boxes: Vec<ColliderBox>,
    /// How the boxes sit in the world: their rotation and pivot.
    pub pose: ColliderPose,
}

impl Default for Collider {
    /// No boxes and default pose.
    fn default() -> Self {
        Self {
            boxes: Vec::new(),
            pose: ColliderPose::default(),
        }
    }
}

impl Collider {
    /// A collider with `boxes` and the default pose (no turn, pivot at the entity's position).
    pub fn new(boxes: Vec<ColliderBox>) -> Self {
        Self {
            boxes,
            pose: ColliderPose::default(),
        }
    }

    /// Replaces the pose.
    pub fn with_pose(mut self, pose: ColliderPose) -> Self {
        self.pose = pose;
        self
    }

    /// Replaces the pose's rotation.
    pub fn with_rotation(mut self, rotation: [f32; 4]) -> Self {
        self.pose.rotation = rotation;
        self
    }

    /// Replaces the pose's pivot.
    pub fn with_pivot(mut self, pivot: [f64; 3]) -> Self {
        self.pose.pivot = pivot;
        self
    }
}

/// Why `EntityRef::set_collider` or `EntityRef::set_collider_pose` refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ColliderError {
    /// The handle no longer identifies a server entity that can carry a collider.
    StaleEntity,
    /// A box value or pivot coordinate is not finite, or the rotation is zero, NaN or infinite.
    NonFinite {
        /// The box's place in the list, or None when the pose (rotation or pivot) is refused.
        index: Option<u64>,
    },
    /// The collider's encoding is larger than one custom payload can carry
    /// (`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`, 1,048,576 bytes).
    TooLarge {
        /// The collider's encoded byte size, without truncation.
        bytes: u64,
        /// The maximum bytes one clientbound custom payload can carry.
        limit: u64,
    },
    /// `set_collider_pose` on an entity that has no collider.
    NoCollider,
}

impl crate::NoBackend for ColliderError {
    fn no_backend() -> Self {
        Self::StaleEntity
    }
}
