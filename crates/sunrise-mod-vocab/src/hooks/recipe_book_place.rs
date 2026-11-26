//! The `recipe_book_place` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player clicked a recipe in the recipe book of a mod menu whose
    /// `RecipeBookLayout` is `Mod`, on the server
    /// (`RecipeBookMenu.handlePlacement`, the method a Java menu overrides).
    /// The part of the open menu's type answers by writing `placement` (which
    /// arrives as [`RecipeBookPlacement::Nothing`]):
    ///
    /// - Leave `RecipeBookPlacement::Nothing` to handle placement through
    ///   normal `Host` operations or do nothing.
    /// - Set `RecipeBookPlacement::Ghost` when a manual menu says ingredients
    ///   are missing (vanilla's `PLACE_GHOST_RECIPE`): while the same opening
    ///   remains current, the server sends the clicked display to the client
    ///   to draw the recipe's ghosts (`recipe_book_ghost`).
    /// - Set `RecipeBookPlacement::Grid { width, height, slots, clear }` to
    ///   have the engine run vanilla's canonical placer
    ///   (`ServerPlaceRecipe.placeRecipe`) once on the active menu and its
    ///   live backing storage with the click's current `use_max_items` and
    ///   `creative` flags, syncing normal slot changes and sending a ghost
    ///   only when ingredients are missing.
    ///
    /// `use_max_items` is the Shift click and `creative` the player's mode.
    /// `player` is swapped in, so `host.players().acting_player()` names them.
    ///
    /// The game has checked what vanilla checks before `handlePlacement`: the
    /// player is not a spectator, the window is the open menu of this type,
    /// the menu is still valid, the player's book holds `recipe`, and its
    /// `PlacementInfo` is not `NOT_PLACEABLE`. The general `player_recipe_book_click`
    /// hook has already run. Nothing is asked of a part for a menu with another
    /// layout; the game places those itself.
    ///
    /// After a mutating callback returns, the engine resolves the menu only to
    /// verify the same opening and uses its current state. Refused `Grid`
    /// inputs (a slot the menu lacks, hides or cannot store, a repeated slot,
    /// an empty grid, or a recipe larger than the slots given) are reported on
    /// each attempt without applying the grid. A panic rolls back
    /// the callback's nested transaction and discards its `placement`. Only a panic
    /// disables the callback for the session, naming the mod in logs and error
    /// toasts. When no part listens, the click is refused.
    ///
    /// On the player's own `InventoryMenu` (`minecraft:inventory`, vanilla's
    /// `AbstractCraftingMenu.handlePlacement`) the game places a recipe itself unless a Part
    /// attached to it listens here; the Parts then take the placement over, every mod's in mod
    /// order. `Nothing` places nothing, which refuses the 2×2 grid; `Ghost` sends the ghost; and
    /// `Grid` is accepted only as that grid (`width` and `height` 2, `slots` `[1, 2, 3, 4]`,
    /// `clear` among them), which runs the game's own placement as a Java menu's
    /// `super.handlePlacement` does. Any other `Grid` is refused with a warning and places
    /// nothing.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        version = 3,
        neoforge = "none (vanilla RecipeBookMenu.handlePlacement, which each menu overrides; no event)"
    )]
    fn recipe_book_place(
        menu: &str,
        recipe: &str,
        use_max_items: bool,
        creative: bool,
        placement: &mut RecipeBookPlacement,
    );
}
