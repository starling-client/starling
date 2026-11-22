//! The `player_recipe_discovered` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0306

pub trait Hooks {
    /// A recipe is about to be unlocked for a server player. Set `cancel` to keep it locked. The
    /// player's state is not swapped in. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_player_recipe_discovered",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerRecipeDiscoverEvent)",
        cancellable = true
    )]
    fn player_recipe_discovered(player: u128, name: &str, recipe: &str, cancel: &mut bool);
}
