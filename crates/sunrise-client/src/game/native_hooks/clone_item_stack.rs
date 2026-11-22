use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// The block's clone answer for the requesting player, before block-entity data
    /// is copied onto the chosen stack (`handlePickItemFromBlock`).
    #[inline(always)]
    pub(in crate::game) fn native_clone_item_stack(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        include_data: bool,
        player: PlayerRef,
        stack: &mut Option<ItemStack>,
    ) {
        let block = &self.res.facts().get(state).name;
        self.fire_clone_item_stack(dimension, pos, block, state, include_data, player, stack);
    }
}
