use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// `IItemExtension.canContinueUsing`: does the use of `stack` go on with
    /// `held` in the used `hand`? `entity` is the user's id, a player's included.
    /// `can_continue` arrives with vanilla's `isSameItem`; the parts of
    /// `stack`'s item may set it either way. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_can_continue_using(
        &self,
        stack: &ItemStack,
        held: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        can_continue: &mut bool,
    ) {
        self.fire_can_continue_using(stack, held, entity, hand, can_continue);
    }
}
