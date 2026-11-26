//! The `recipe_book_ghost` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server answered a recipe click in a mod menu's recipe book with missing items, and the
    /// client is about to draw the recipe's ghosts: the place vanilla calls the component's
    /// `fillGhostRecipe(ghostSlots, display, context)`. `display` is the alternative the
    /// player clicked, sent directly by the server for the current opening. `ghosts` arrives
    /// holding the game's own ghosts for the menu's layout (a crafting, furnace, stonecutter or
    /// smithing layout maps the display to its slots; a `Mod` layout arrives empty); add, change
    /// or clear them. Following vanilla's `GhostSlots.setSlot`, each ghost's `display` resolves
    /// to stacks as the book's own displays do against the screen's actual shown slots: a
    /// nonempty resolved display replaces any prior ghost for the same physical slot in place
    /// (preserving slot order), while an unresolved empty display is dropped without erasing a
    /// prior ghost, and a slot the menu's screen hides or lacks is omitted. There is no separate
    /// ghost-count cap beyond the screen's actual shown slots. A listener that pushes nothing
    /// leaves what arrived. Asked for the open menu's book only, once per server ghost response,
    /// by the part of the menu's type; a menu with none skips the hook.
    /// Client-side, with the `ClientHost`.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "listen",
        scope = "menu",
        key = "menu",
        version = 2,
        neoforge = "none (vanilla RecipeBookComponent.fillGhostRecipe, which each book component overrides; no event)"
    )]
    fn recipe_book_ghost(
        menu: &str,
        recipe: &str,
        display: &RecipeBookDisplay,
        ghosts: &mut Vec<GhostSlot>,
    );
}
