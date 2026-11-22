use super::*;

impl Game<'_> {
    /// An applied item consumption or durability break, with the complete stack
    /// from before the action and its hand (None for non-hand equipment/ammo).
    /// Each logical side reports its own action, including the local player.
    pub(in crate::game) fn native_player_destroy_item(
        &mut self,
        player: u128,
        player_entity_id: i32,
        original: &ItemStack,
        hand: Option<Hand>,
    ) {
        self.fire_player_destroy_item(player, player_entity_id, original, hand);
    }
}
