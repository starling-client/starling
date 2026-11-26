//! The identity of one backing slot of a block container, for storage networks that count a
//! double chest's halves, a multiblock's blocks and a worldly container's faces once.
use crate::LevelRef;

/// Slot `slot` of the container owned by the block entity at `pos` in `level`
/// (`Container.getItem`'s index, unsided). Two handler indices, sides or places that report an
/// equal `StorageSlot` reach the same stack, and unequal ones reach different stacks: a furnace's
/// sides report the one slot they share, and each half of a double chest reports its own.
///
/// It is what [`MenuSlotBacking::BlockEntity`](crate::MenuSlotBacking::BlockEntity) holds,
/// [`ResourceHandler::backing`](crate::ResourceHandler::backing) reports for a handler index and
/// [`ItemPlace::backing`](crate::ItemPlace::backing) for a place.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: value identity of a block container slot (level, block pos, unsided slot)
pub struct StorageSlot {
    /// The level the block entity is in.
    pub level: LevelRef,
    /// The block entity's position.
    pub pos: [i32; 3],
    /// The slot's index in that block entity's container, whatever side reached it.
    pub slot: usize,
}
