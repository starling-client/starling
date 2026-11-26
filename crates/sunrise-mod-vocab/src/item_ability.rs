//! Call-scoped context for [`ItemPart::can_perform_action`](crate::ItemPart::can_perform_action).

use crate::Tags;

/// Call-scoped context for [`ItemPart::can_perform_action`](crate::ItemPart::can_perform_action):
/// the merged [`Tags`] of the side asking, and nothing else. NeoForge's
/// `IItemExtension.canPerformAction(ItemStack, ItemAbility)` gets no level either: the answer
/// depends only on the stack and the ability, so gameplay action sites and loot predicates
/// (`neoforge:can_item_perform_ability`) get the same answer for the same stack.
// no wire: lends the asking side's merged tags for one call.
// exhaustive: constructed only at the gameplay and loot ability boundaries.
pub struct ItemAbilityContext<'a> {
    tags: &'a dyn Tags,
}

impl<'a> ItemAbilityContext<'a> {
    /// Lend the asking side's merged tags.
    // hidden: constructed by the gameplay and loot ability bridges and tests.
    #[doc(hidden)]
    pub fn new(tags: &'a dyn Tags) -> Self {
        Self { tags }
    }

    /// The merged tags of the side asking: the live game's in gameplay, the loot evaluator's
    /// during a loot predicate. Both hold the same data packs' tags.
    pub fn tags(&self) -> &dyn Tags {
        self.tags
    }
}
