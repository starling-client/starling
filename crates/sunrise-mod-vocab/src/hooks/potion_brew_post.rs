//! The `potion_brew_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `PotionBrewEvent.Post` in `BrewingStandBlockEntity.doBrew`.
    /// `stacks` contains the renewed bottles and the as-yet unconsumed ingredient.
    /// The stand takes the edited slots before consuming the original ingredient
    /// and producing its remainder and brew sound. If Pre cancels and at least
    /// one slot matches its prior value, Post sees the replacement slots; a
    /// canceled Pre never consumes the ingredient. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "PotionBrewEvent.Post",
        cancellable = false,
        version = 3
    )]
    fn potion_brew_post(pos: [i32; 3], stacks: &mut [Option<ItemStack>; 5]);
}
