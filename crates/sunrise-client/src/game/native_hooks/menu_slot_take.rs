use super::*;

impl Game<'_> {
    /// A player (`player`, their UUID) took `taken` out of slot `slot` (the menu's index) of
    /// the mod menu type `menu`, which holds `left` after it: `Slot.onTake`, on the server,
    /// after the take. `subject` lends the live resident block entity of a block-backed menu,
    /// and no view on a blockless owned menu.
    pub(in crate::game) fn native_menu_slot_take(
        menu: &str,
        subject: mod_api::BlockEntityScope<'_>,
        player: u128,
        slot: usize,
        taken: &mod_api::ItemStack,
        left: Option<&mod_api::ItemStack>,
    ) {
        Self::fire_menu_slot_take(menu, subject, player, slot, taken, left);
    }
}
