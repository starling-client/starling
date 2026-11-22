//! A moving collider: boxes an entity carries that other entities collide with, and why one is refused.
/// One box of a [`Collider`], axis-aligned in the collider's own frame.
///
/// `min` and `max` are `[x, y, z]` in blocks from the entity's position, before the collider's
/// rotation; a box whose minimum is above its maximum on an axis is read with the two swapped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal, and the two corners are all there is to a box
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ColliderBox {
    /// The corner with the smaller coordinates, `[x, y, z]`.
    pub min: [f64; 3],
    /// The corner with the larger coordinates, `[x, y, z]`.
    pub max: [f64; 3],
}

/// Boxes an entity carries, turned by a rotation, that every other entity collides with.
///
/// Set with `EntityRef::set_collider`. The boxes are in the entity's local frame: its position
/// is the origin, and `rotation` (a unit quaternion `[x, y, z, w]`, any yaw, pitch and roll)
/// turns the frame in the world. Other entities collide with the boxes as solid ground and
/// walls: an entity's own box is swept against each turned box (a separating-axis test with the
/// time of impact), it steps up onto a box's top within its step height, and the server's
/// movement check sees the same boxes. A collider is solid to every mover except spectators,
/// entities with no physics and the entities in its own vehicle stack.
///
/// A collider that moves or turns carries the bodies standing on it: once a tick, after the
/// level's entities ticked, each moves by the displacement of its contact point as a collided
/// move with `MoveCause::Carried`. After carrying, a collider that translated without turning
/// shoves bodies in the path of its boxes, in its direction of travel, with the same move cause.
/// The shove does not cancel velocity into the collider; ordinary collision handles velocity.
/// A body caught between the collider and a wall stays where the move leaves it, and a living
/// one takes `in_wall` damage while a box covers its eyes. Passengers follow their vehicle.
///
/// A vanilla client has none of this: the server sends a collider only to a Sunrise client that
/// tracks the entity, and a vanilla client sees the entity and walks through it.
///
/// Checked when it is set, and never clamped or truncated; a collider that fails a check is
/// refused by name with a [`ColliderError`] and the old one stays:
/// - every box value and every rotation value is finite, and the rotation is a unit quaternion
///   within 1e-3 of length 1;
/// - at most [`Collider::MAX_BOXES`] boxes;
/// - every box within [`Collider::MAX_REACH`] blocks of the entity on each axis.
///
/// The limits bound what the game keeps and sends; vanilla has none. A box of 0 thickness is a
/// box with no volume that nothing collides with.
///
/// **Contact effects are not applied yet.** A box is solid ground and walls and nothing more:
/// standing or walking on one has no block's step sound, sprint particles, friction or bounce
/// (tracked in sunrise-org/sunrise#1909).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods build it with a literal, and the boxes and their rotation are all there is to a collider
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Collider {
    /// The boxes, in the entity's frame.
    pub boxes: Vec<ColliderBox>,
    /// A unit quaternion `[x, y, z, w]`: `[0.0, 0.0, 0.0, 1.0]` is no turn.
    pub rotation: [f32; 4],
}

impl Default for Collider {
    /// No boxes and no turn.
    fn default() -> Self {
        Self {
            boxes: Vec::new(),
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Collider {
    /// The most boxes one collider holds: 2048 blocks of 16 boxes each, above the box count of
    /// the heaviest shape a block has.
    pub const MAX_BOXES: usize = sunrise_core::geometry::oriented::MAX_BOXES;
    /// The farthest a box may lie from the entity on any axis, in blocks.
    pub const MAX_REACH: f64 = sunrise_core::geometry::oriented::MAX_REACH;
    /// How far from length 1 a rotation may be.
    pub const ROTATION_TOLERANCE: f32 = sunrise_core::geometry::oriented::ROTATION_TOLERANCE as f32;
}

/// Why `EntityRef::set_collider` or `set_collider_transform` refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ColliderError {
    /// The handle no longer identifies a server entity that can carry a collider.
    StaleEntity,
    /// A value is not finite, or the rotation is not a unit quaternion.
    NonFinite {
        /// The box's place in the list, or None when the rotation is the one refused.
        index: Option<u64>,
    },
    /// The collider has more boxes than [`Collider::MAX_BOXES`].
    TooManyBoxes {
        /// How many boxes were given, without clamping.
        count: u64,
        /// The most boxes a collider may hold.
        limit: u64,
    },
    /// A box lies farther than [`Collider::MAX_REACH`] from the entity on some axis.
    OutOfRange {
        /// The box's place in the list.
        index: u64,
    },
    /// `set_collider_transform` on an entity that has no collider.
    NoCollider,
}

impl crate::NoBackend for ColliderError {
    fn no_backend() -> Self {
        Self::StaleEntity
    }
}
