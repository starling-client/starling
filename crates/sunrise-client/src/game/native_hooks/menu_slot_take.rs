use super::*;

impl Game<'_> {
    /// A player (`player`, their UUID) took `taken` out of slot `slot` (the menu's index) of
    /// the mod menu type `menu` open on the live resident mod block entity (`kind`, `subject`),
    /// which holds `left` after it: `Slot.onTake`, on the server, after the take.
    pub(in crate::game) fn native_menu_slot_take(
        kind: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityScope<'_>,
        player: u128,
        menu: &str,
        slot: usize,
        taken: &mod_api::ItemStack,
        left: Option<&mod_api::ItemStack>,
    ) {
        Self::fire_menu_slot_take(kind, subject, player, menu, slot, taken, left);
    }
}
