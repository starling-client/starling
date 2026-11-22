//! The `recipe_book_place` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player clicked a recipe in the recipe book of a mod menu whose
    /// `RecipeBookLayout` is `Mod`, on the server
    /// (`RecipeBookMenu.handlePlacement`, the method a Java menu overrides).
    /// The part of the open menu's type moves the recipe's ingredients itself
    /// and sets `ghost` when items are missing, as vanilla's answer
    /// `PLACE_GHOST_RECIPE` does: the client is then told to draw the recipe's
    /// ghosts (`recipe_book_ghost`). `use_max_items` is the Shift click and
    /// `creative` the player's mode. `player` is swapped in, so
    /// `host.players().acting_player()` names them.
    ///
    /// The game has checked what vanilla checks before `handlePlacement`: the
    /// player is not a spectator, the window is the open menu of this type,
    /// the menu is still valid, the player's book holds `recipe`, and its
    /// `PlacementInfo` is not `NOT_PLACEABLE`. Nothing is asked of a part for
    /// a menu with another layout; the game places those itself.
    ///
    /// To place, call `ServerHost::place_recipe_in_grid` once with the grid's
    /// width and height, its slots and the slots to clear, as vanilla's
    /// `ServerPlaceRecipe.placeRecipe` takes them: it returns whether items
    /// were missing, which is the answer to give in `ghost`. Everything the
    /// part and that call write is one action: it is applied once or, when the
    /// player, the menu or a block behind it changed meanwhile (a write the
    /// part made through the `Host` to the player's inventory, cursor or the
    /// menu's storage counts), refused and undone. When no part listens, the
    /// click is refused.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (vanilla RecipeBookMenu.handlePlacement, which each menu overrides; no event)"
    )]
    fn recipe_book_place(
        menu: &str,
        recipe: &str,
        use_max_items: bool,
        creative: bool,
        ghost: &mut bool,
    );
}
