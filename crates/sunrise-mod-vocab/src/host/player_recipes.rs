//! The `player_recipes` area: the recipe book: discovered recipes and custom book settings. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_recipes` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerRecipesBackend {
    /// Side: server.
    /// Read one registered custom book's authoritative flags. A fresh book
    /// returns the default pair; an unavailable player or unregistered key
    /// returns None. This never reads a vanilla ordinal or pending client intent.
    fn player_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings>;
    /// Side: server.
    /// Whether the player's recipe book knows `recipe`
    /// (`ServerRecipeBook.contains`).
    fn player_has_discovered_recipe(&self, player: PlayerRef, recipe: &str) -> Option<bool>;
    /// Side: both.
    /// The recipes in the player's recipe book (`ServerRecipeBook.recipes`), as qualified ids in
    /// order (`minecraft:oak_planks`). None for a player it can't reach.
    fn player_discovered_recipes(&self, player: PlayerRef) -> Option<Vec<String>>;
}
/// The handles' writes of the `player_recipes` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerRecipesBackendMut {
    /// Side: server.
    /// Set a registered custom book's authoritative pair and synchronize an
    /// accepted change. Returns false for an unavailable player, unknown key
    /// or exhausted storage. An unchanged pair succeeds without notification.
    fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
    /// Side: server.
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    fn player_award_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
    /// Side: server.
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    fn player_reset_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
}
