use super::*;

impl Game<'_> {
    /// The block's own `Block.setPlacedBy`, before the separate item notice.
    pub(in crate::game) fn native_set_placed_by(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        placer: Option<EntityRef>,
        stack: Option<&ItemStack>,
    ) {
        self.fire_set_placed_by(level, pos, state, placer, stack);
    }
}
