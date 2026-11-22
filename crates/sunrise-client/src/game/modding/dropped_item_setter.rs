use super::Game;
use sunrise_client::items::ItemStack;

impl Game<'_> {
    /// Sets the stack in a validated item-entity slot, as a hopper's `addItem`
    /// leaves what it couldn't take (`ItemEntity.setItem`). An empty stack
    /// (count 0) discards the entity. Players see it on their next tracker update.
    pub(super) fn native_set_item_entity(&mut self, i: usize, stack: ItemStack) -> bool {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        if stack.count == 0 {
            self.remove_dropped(i);
        } else {
            self.dropped[i].stack = stack;
        }
        true
    }
}
