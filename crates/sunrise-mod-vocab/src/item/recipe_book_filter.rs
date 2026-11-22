//! How a custom book's craftable-only filter button looks.
use super::RecipeBookTypeKey;

/// The look of one custom book's filter button, declared with
/// [`RecipeBooksRegistrar::recipe_book_filter_look`](crate::RecipeBooksRegistrar::recipe_book_filter_look)
/// (vanilla's `RecipeBookComponent.getFilterButtonTextures` and `getRecipeFilterName`).
///
/// The four sprites are GUI sprite ids (`namespace:path`, as the GUI atlas lists them) of a 26 by 16 button. The tooltip is a translation key. It names the state the
/// button switches to when it is on; the off state keeps vanilla's `gui.recipebook.toggleRecipes.all`.
/// A book without a declared look keeps the crafting book's button and tooltip.
#[derive(Clone, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the four sprites and the tooltip are all that vanilla's filter button varies
pub struct RecipeBookFilterLook {
    /// Filtering on, the cursor away.
    pub enabled: String,
    /// Filtering off, the cursor away.
    pub disabled: String,
    /// Filtering on, the cursor over the button.
    pub enabled_highlighted: String,
    /// Filtering off, the cursor over the button.
    pub disabled_highlighted: String,
    /// Translation key of the tooltip shown while filtering is on.
    pub tooltip: String,
}

impl RecipeBookFilterLook {
    /// The declared look of `book`, or none when it keeps the crafting book's. Read by the
    /// engine when it draws the button; it borrows from the frozen registry, so a frame costs no copy.
    pub fn registered(book: &RecipeBookTypeKey) -> Option<&'static Self> {
        crate::parts::installed()?
            .recipe_book_filters
            .get(book.as_str())
    }
}
