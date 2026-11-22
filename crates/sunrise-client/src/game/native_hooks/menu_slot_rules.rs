use super::*;

impl Game<'_> {
    /// Slot placement, pickup and stack limits for registered vanilla and mod menus: `menu` is the
    /// menu type, `slot` the backing container index, `stack` the stack being asked about.
    /// `may_place`, `may_pickup` and `max_stack` arrive holding the slot's own answers, which the menu type's
    /// parts may change; the click bounds the final maximum. A read-only fire queried directly at
    /// each live slot check. `context` gives the player, the menu's block position and a scoped
    /// read of its container slots.
    #[inline(never)]
    pub(in crate::game) fn native_menu_slot_rules(
        &self,
        call: mod_api::__private::MenuSlotRulesCall<'_>,
    ) {
        self.fire_menu_slot_rules(call);
    }
}
