//! The item's install-time enchantment support answer.

pub trait Hooks {
    /// NeoForge's `IItemExtension.supportsEnchantment`: may this item's stack
    /// receive `enchantment` (an id such as `minecraft:sharpness`)? The first
    /// `Some(bool)` replaces support; `None` tries the next part and then
    /// keeps the caller's vanilla rule. Anvils accept enchanted books; `/enchant`
    /// and compatibility-filtered loot use `supported_items`. Creative anvil use
    /// still bypasses support, and enchantment conflicts remain separate.
    /// Both anvil calculations, `/enchant` and compatibility-filtered
    /// `enchant_randomly` read the answer. The default for `is_primary_item_for`
    /// reads it before testing the enchantment's `primary_items`.
    ///
    /// The answer is per item and enchantment: each item's parts are asked once
    /// per registered enchantment after the current tags are installed (server
    /// world open or `/reload`, or client tag packets), with `stack` the item's
    /// default stack. The engines read the answers stored on that side's enchantment
    /// registry until the next install or world close. This is an
    /// install-time snapshot, not a query of the held stack's components.
    /// Only the `ItemPart`s attached to `stack`'s item are asked, on both sides
    /// with a read-only `Host`. The snapshot limit is Sunrise's contract;
    /// a component-dependent answer is not supported by this hook or its
    /// `is_primary_item_for` pair.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.supportsEnchantment"
    )]
    fn supports_enchantment(stack: &ItemStack, enchantment: &str, answer: &mut Option<bool>);
}
