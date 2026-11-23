//! The `recipes_received` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0124

pub trait Hooks {
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): from now on the recipe helpers
    /// (`native_recipes`, `native_recipes_of_type`) give the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe the
    /// window's own recipe book doesn't know has `book_id` -1. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RecipesReceivedEvent"
    )]
    fn recipes_received();
}
