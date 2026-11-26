use super::{ItemStack, StackWriteError};
use crate::{LevelRef, PlayerRef, ServerHost};

impl ItemStack {
    /// Wears this stack by `amount` points as vanilla wears an item
    /// (`ItemStack.hurtAndBreak(amount, level, player, onBreak)`), for a stack nobody wears: one a
    /// hook hands over by value, one in a container or a block entity. Enchantments' durability
    /// effects apply (`EnchantmentHelper.processDurabilityChange`: Unbreaking and any other),
    /// drawing from `level`'s random; an item that is not damageable, or is unbreakable, is left
    /// as it is; at its most damage one item of the stack breaks away. With `player`, a player
    /// with infinite materials (creative) wears nothing, and the player's durability advancement
    /// trigger fires as vanilla's does; the player must be in `level`. An item's
    /// `ItemPart::damage_item` answers during the wear, as for any wear. A living entity's
    /// equipped stack wears through
    /// [`LivingRef::hurt_and_break`](crate::LivingRef::hurt_and_break) instead, which also
    /// writes the slot back and tells the trackers.
    ///
    /// Returns the stack as it is after the wear, `None` once its last item broke away, so the
    /// caller writes it back where the stack lives. `StackWriteError::Unavailable` on a client,
    /// for a level that is not the one in place, or for a player who is not here or is in another
    /// level; the stack's own validation at the write boundary can refuse it with the other
    /// `StackWriteError` variants.
    pub fn hurt_and_break(
        &self,
        host: &mut dyn ServerHost,
        level: LevelRef,
        amount: i32,
        player: Option<PlayerRef>,
    ) -> Result<Option<ItemStack>, StackWriteError> {
        let Some(backend) = crate::backend_mut_of(host) else {
            return Err(StackWriteError::Unavailable);
        };
        backend.item_stack_hurt_and_break(self, level, amount, player)
    }
}
