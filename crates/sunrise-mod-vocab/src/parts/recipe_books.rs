//! Custom book identities, independent of recipe categories and vanilla ordinals.

use crate::item::recipe_book::RecipeBookTypeKey;

/// Operations available during the existing shared mod registration phase.
pub trait RecipeBooksRegistrar {
    /// Register an explicit lowercase `namespace:path` in any manifest-owned namespace.
    /// IDs are limited to 256 bytes and 1024 books in total. Invalid or duplicate
    /// declarations refuse installation even if the caller ignores the error.
    /// The registry freezes when registration ends, before player state is loaded.
    fn recipe_book_type(&mut self, id: &str) -> Result<RecipeBookTypeKey, String>;
    /// Register a book whose flags use explicit, case-sensitive fields in the player's
    /// `recipeBook` compound, like vanilla's `RecipeBookSettings.TypeSettings` codec.
    /// Each true flag is saved as a byte; false flags are omitted. This book is not
    /// also saved in the engine extension. Names may contain any characters, must be
    /// nonempty and at most 256 UTF-8 bytes, and must differ. Vanilla flag fields,
    /// `recipes`, `toBeDisplayed`, `sunrise:book_settings`, and other books' fields
    /// are reserved. Invalid declarations refuse installation even if ignored.
    fn recipe_book_type_with_fields(
        &mut self,
        id: &str,
        open_field: &str,
        filtering_field: &str,
    ) -> Result<RecipeBookTypeKey, String>;
    /// Give a book of this mod its own filter button look: four sprites and a tooltip key. Each
    /// sprite is a `namespace:path` GUI sprite id and the tooltip a nonempty translation key of
    /// at most 256 bytes. A book takes one look; a second declaration, a book another mod
    /// registered and a malformed id refuse installation even if the caller ignores the error.
    fn recipe_book_filter_look(
        &mut self,
        book: &RecipeBookTypeKey,
        look: crate::RecipeBookFilterLook,
    ) -> Result<(), String>;
}
