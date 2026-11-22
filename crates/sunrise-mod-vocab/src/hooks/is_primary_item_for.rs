//! The item's install-time primary enchantment eligibility answer.

pub trait Hooks {
    /// NeoForge's `IItemExtension.isPrimaryItemFor`: is `stack`'s item primary
    /// for `enchantment` (an id such as `minecraft:sharpness`), so enchantment
    /// selection may offer it? Selection chooses the enchanting table's offers
    /// and clues, loot `enchant_with_levels`, trades and spawn gear.
    /// `primary` arrives true for a plain book; otherwise it is the resolved
    /// `supports_enchantment` answer and membership in `primary_items` when that
    /// set is present. Set it true to offer the pair or false to withhold it,
    /// including for a plain book.
    ///
    /// The answer is per item and enchantment: each item's parts are asked once
    /// per registered enchantment after the world's tags and support answers are installed,
    /// with `stack` the item's default stack, and the engines read the answers
    /// stored on that side's enchantment registry until its next install or its
    /// world closes. This is an
    /// install-time snapshot, not a query of the held stack's components.
    /// Only the `ItemPart`s attached to `stack`'s item are asked, on the logical
    /// server with a read-only `Host`. The snapshot limit is Sunrise's contract;
    /// a component-dependent answer is not supported by this hook or its
    /// `supports_enchantment` pair.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        version = 2,
        neoforge = "IItemExtension.isPrimaryItemFor"
    )]
    fn is_primary_item_for(stack: &ItemStack, enchantment: &str, primary: &mut bool);
}
