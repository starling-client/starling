//! A block state mirrored as a structure template places it (vanilla's
//! `BlockBehaviour.mirror(BlockState, Mirror)`, which a block overrides to flip its own
//! properties), which the parts of the state's block may answer in `mirror`.
use crate::{BlockState, StructureMirror};

/// A structure template is about to place `state` at `pos` mirrored by `mirror`: `mirrored`
/// starts at the game's own mirrored state, and the parts of `state`'s block may set another
/// state of that block.
#[inline(never)]
pub fn native_mirror(
    context: &crate::OwningWorldgen<'_>,
    state: BlockState,
    pos: [i32; 3],
    mirror: StructureMirror,
    mirrored: &mut BlockState,
) {
    crate::dispatch_mirror(context, state, pos, mirror, mirrored);
}
