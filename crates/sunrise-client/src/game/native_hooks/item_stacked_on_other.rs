use super::*;

impl Game<'_> {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override: a pickup click on a menu
    /// slot, with its cursor and clicked slot lent in `click`. A listener sets `canceled` to skip
    /// both item overrides and the bundle override; `click.handled` then says whether the click is
    /// done or plain pickup still runs. Authoritative server side only; only the `ItemPart`
    /// callback below runs on both sides.
    #[inline(never)]
    pub(in crate::game) fn native_item_stacked_on_other(
        &mut self,
        click: &mut mod_api::item::menu::StackedOnOther<'_>,
        canceled: &mut bool,
    ) {
        self.fire_item_stacked_on_other(click, canceled);
    }

    /// `ItemPart::item_stacked_on_other` for `item` (either the carried stack in
    /// `CarriedOnOther` or the clicked stack in `OtherStackedOnMe`), sharing `HookId`, the lent
    /// click, and transaction with `native_item_stacked_on_other`. Both sides (server clicks,
    /// client prediction, and creative inventory-tab/hotbar pickups).
    #[inline(never)]
    pub(in crate::game) fn item_stacked_on_other_part(
        &mut self,
        item: &str,
        click: &mut mod_api::item::menu::StackedOnOther<'_>,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_item_stacked_on_other_part(
            execution_role::host_side(self),
            item,
            click,
            canceled,
        );
    }
}
