use super::*;

impl Game<'_> {
    /// `Block.isPossibleToRespawnInThis` for a forced respawn column.
    pub(in crate::game) fn native_is_possible_to_respawn_in(
        &self,
        state: BlockStateId,
        answer: &mut Option<bool>,
    ) {
        self.fire_is_possible_to_respawn_in(state, answer);
    }
}
