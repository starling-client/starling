use super::*;

impl Game<'_> {
    /// Asks a block entity's subscribed part for its admission rule. The transfer loop
    /// already prepared the API snapshots, so candidate slots reuse them.
    pub(in crate::game) fn native_can_place_item(
        entity: &ModBlockEntity,
        pos: BlockPos,
        state: mod_api::BlockState,
        slot: usize,
        stack: &mod_api::ItemStack,
        allowed: &mut Option<bool>,
    ) {
        Self::fire_can_place_item(entity, pos, state, slot, stack, allowed);
    }
}
