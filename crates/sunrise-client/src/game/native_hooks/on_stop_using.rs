use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// `IItemExtension.onStopUsing`: the use of `stack` by `entity` (`None`
    /// for a player) in `hand` is over, `time_left` ticks before its end (0
    /// for a finished use). After the use is cleared. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_on_stop_using(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        time_left: i32,
    ) {
        self.fire_on_stop_using(stack, entity, hand, time_left);
    }
}
