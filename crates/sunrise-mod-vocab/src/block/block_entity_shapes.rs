//! A block entity's published collision and outline shapes, and why one is refused.

/// The collision and outline shapes a block entity gives its block, in block units.
///
/// A block that declares `shapes_from_block_entity` has these in place of its declared
/// shapes while its block entity holds them (`BlockEntityRef::set_shapes`). Each box is
/// `[min_x, min_y, min_z, max_x, max_y, max_z]` as in `BlockShapes`, relative to the
/// block's own position. Both lists are empty-able: an empty list is an empty shape, which
/// collides with nothing or cannot be picked.
///
/// Checked when it is published, and never clamped:
/// - every coordinate finite, and every box's minimum at most its maximum on each axis,
///   refused by name otherwise;
/// - at most 64 boxes in each list, refused by name past that. Vanilla has no such limit;
///   this one bounds what a block entity makes every client receive and keep. Declared
///   shape tables (`BlockShapes`) sync nothing and have no box limit;
/// - a box under 1e-7 thick on any axis is an empty box, as vanilla's `Shapes.create`
///   makes it, and is accepted.
///
/// A box may lie anywhere. Collision finds boxes only within one block of the block, one axis
/// at a time; a collision box beyond that is stored and sent but never found, as in vanilla.
/// An outline can be hit past its block once the ray enters the block. In debug builds the
/// game warns once per block when a publish holds a box beyond one block.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal, and its two lists are all there is to a shape
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BlockEntityShapes {
    /// What the crosshair picks, and what the block's outline draws.
    pub outline: Vec<[f64; 6]>,
    /// What entities collide with, and what supports other blocks.
    pub collision: Vec<[f64; 6]>,
}

/// Why `BlockEntityRef::set_shapes` refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockEntityShapesError {
    /// The handle no longer identifies a loaded block entity that can be written here.
    Stale,
    /// The block entity's block does not declare `shapes_from_block_entity`.
    NotDeclared,
    /// A list has more boxes than the game accepts.
    TooManyBoxes {
        /// Whether the collision list is the one refused, not the outline list.
        collision: bool,
        /// How many boxes were given, without clamping.
        count: u64,
        /// The most boxes a list may hold.
        limit: u64,
    },
    /// A box has a coordinate that is not finite.
    NotFinite {
        /// Whether the collision list is the one refused, not the outline list.
        collision: bool,
        /// The box's place in its list.
        index: u64,
    },
    /// A box has a minimum above its maximum.
    InvertedBox {
        /// Whether the collision list is the one refused, not the outline list.
        collision: bool,
        /// The box's place in its list.
        index: u64,
    },
    /// The boxes together make a shape the game cannot build.
    Unbuildable,
}

impl crate::NoBackend for BlockEntityShapesError {
    fn no_backend() -> Self {
        Self::Stale
    }
}
