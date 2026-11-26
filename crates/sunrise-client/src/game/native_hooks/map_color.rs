use super::*;

impl Game<'_> {
    /// `IBlockExtension.getMapColor` for the block `state` at `pos`, asked by `MapItem.update`:
    /// `color` starts with the state's own `MapColor` id and the block's parts may change it. The
    /// level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_map_color(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        color: &mut u8,
    ) {
        self.fire_map_color(dimension, pos, state, color);
    }
}
