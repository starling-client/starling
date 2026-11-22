use super::*;

impl Game<'_> {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override: a pickup click on a menu
    /// slot, with the cursor and both stacks frozen in `event`. A listener sets `canceled` to skip
    /// the bundle override and answers in `result`: `handled` for a cancelled click that is done
    /// (nothing else runs), and the stacks the click leaves in the cursor and the slot. The click
    /// validates the answer once and applies it once. Authoritative server side only; only the
    /// `ItemPart` callback below runs on both sides.
    #[inline(never)]
    pub(in crate::game) fn native_item_stacked_on_other(
        &mut self,
        event: &mod_api::item::menu::StackedOnOther,
        result: &mut mod_api::item::menu::StackedOnOtherResult,
        canceled: &mut bool,
    ) {
        self.fire_item_stacked_on_other(event, result, canceled);
    }

    /// `ItemPart::item_stacked_on_other` for `item` (either the carried stack in
    /// `CarriedOnOther` or the clicked stack in `OtherStackedOnMe`), sharing `HookId`, event, and
    /// transaction with `native_item_stacked_on_other`. Both sides (server clicks, client
    /// prediction, and creative inventory-tab/hotbar pickups).
    #[inline(never)]
    pub(in crate::game) fn item_stacked_on_other_part(
        &mut self,
        item: &str,
        event: &mod_api::item::menu::StackedOnOther,
        result: &mut mod_api::item::menu::StackedOnOtherResult,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_item_stacked_on_other_part(
            execution_role::host_side(self),
            item,
            event,
            result,
            canceled,
        );
    }
}
