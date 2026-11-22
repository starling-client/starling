//! Report destruction only where an item action actually consumes or breaks a stack.
use super::*;

impl Game<'_> {
    /// A slot's hand, or no hand for armour and inventory ammunition.
    pub(super) fn item_slot_hand(&self, slot: usize) -> Option<Hand> {
        if slot == shield::OFFHAND_SLOT {
            Some(Hand::Off)
        } else if slot == self.selected {
            Some(Hand::Main)
        } else {
            None
        }
    }

    /// Report an applied destruction, not a hand edit made by a listener.
    pub(super) fn player_item_destroyed(&mut self, original: &ItemStack, hand: Option<Hand>) {
        if original.count == 0
            || &*original.item == "minecraft:air"
            || !mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
        {
            return;
        }
        let (player, player_entity_id) = self.native_clicker();
        self.native_player_destroy_item(player, player_entity_id, original, hand);
    }

    /// Consume from a player slot after the caller's creative-mode gate.
    /// Notify before a use remainder replaces the now-empty stack.
    pub(super) fn consume_player_slot(&mut self, slot: usize, amount: u32) {
        let original = mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
            .then(|| self.stack_in(slot).cloned())
            .flatten();
        let hand = self.item_slot_hand(slot);
        let edit = |cell: &mut Option<ItemStack>| {
            if let Some(stack) = cell.as_mut() {
                stack.count = stack.count.saturating_sub(amount);
                if stack.count == 0 {
                    *cell = None;
                }
            }
        };
        if slot == self.held_slot() {
            self.edit_block_use_stack(edit);
        } else {
            edit(self.stack_in_mut(slot));
        }
        if self.stack_in(slot).is_none_or(|stack| stack.count == 0)
            && let Some(original) = original
        {
            self.player_item_destroyed(&original, hand);
        }
    }

    pub(super) fn consume_held(&mut self) {
        if !self.creative {
            self.consume_player_slot(self.held_slot(), 1);
        }
    }
}
