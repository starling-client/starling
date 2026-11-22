use super::*;

impl Game<'_> {
    /// Resolve the item's support answer with its default stack when parts install.
    pub(in crate::game) fn native_supports_enchantment(
        &self,
        stack: &ItemStack,
        enchantment: &str,
        answer: &mut Option<bool>,
    ) {
        self.fire_supports_enchantment(stack, enchantment, answer);
    }
}
