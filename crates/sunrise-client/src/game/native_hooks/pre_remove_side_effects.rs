use super::*;

impl GameLevel<'_> {
    /// Collect removal outputs without world writes.
    #[inline(always)]
    pub(in crate::game) fn native_pre_remove_side_effects(
        &mut self,
        call: super::fire::PreRemoveSideEffectsCall<'_>,
    ) {
        self.fire_pre_remove_side_effects(call);
    }
}
