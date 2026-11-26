use super::*;

impl Game<'_> {
    /// `BlockBehaviour.getDestroyProgress` as the block of `state` overrides it: a block Part's
    /// answer for the player's dig at `pos`, left unset to keep the game's math.
    pub(in crate::game) fn native_destroy_progress(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockStateId,
        progress: &mut Option<f32>,
    ) {
        self.fire_destroy_progress(player, pos, state, progress);
    }
}
