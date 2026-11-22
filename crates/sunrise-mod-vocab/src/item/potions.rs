//! The registered potions a client mod lists (`Registries.POTION`), for recipe viewers that show a
//! potion, splash potion, lingering potion or tipped arrow per potion.

/// One registered potion (`Registries.POTION`), as [`ItemsClientHost::potions`](crate::ItemsClientHost::potions)
/// lists it. A mod builds the stack itself: an item such as `minecraft:potion` with a
/// `minecraft:potion_contents` component whose `potion` is [`id`](Self::id), through
/// [`ItemStack::set_components`](crate::ItemStack::set_components).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PotionInfo {
    /// Registry name, including namespace (`minecraft:long_swiftness`).
    pub id: String,
}

impl PotionInfo {
    /// A registered potion named `id` (`minecraft:water`).
    pub fn new(id: String) -> Self {
        Self { id }
    }
}
