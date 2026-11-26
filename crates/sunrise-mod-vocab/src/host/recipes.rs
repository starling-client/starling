//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `recipes` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait RecipesHost {
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The recipes of registered recipe type `kind` (`minecraft:crafting`, `mymod:crusher`), vanilla's
    /// types and mods' own alike, from `data/<namespace>/recipe/*.json` of vanilla,
    /// the content mods (`.smod`) and the world's data packs, in file order: each
    /// one typed (`Recipe`: its ingredients, results, group, cooking time and
    /// experience, or its stonecutting, smithing or special crafting rule),
    /// together with its JSON. Locally loaded recipes give the original recipe JSON;
    /// synced vanilla serializers give the server's resolved item lists, and synced mod
    /// serializers give the data written by that serializer. Bound native predicates
    /// aren't written into the JSON or sent to clients. The `type` field names the
    /// serializer; this can differ from the reported recipe kind. Dynamic rules are not fixed
    /// outputs; use `LevelRef::recipe_for` for engine-matchable recipe types.
    /// Registered crafting kinds match through LevelRef::recipe_for. Cache other mod-owned
    /// types once per load/reload and match those in mod memory; those return
    /// RecipeLookupError::NotMatchable. These are the
    /// recipes in force (`RecipeManager.getAllRecipesFor`): data packs merged and
    /// load conditions held, so a pack's override or removal shows. A loaded recipe the API
    /// cannot represent is left out of this listing and logged at debug level with its id
    /// and reason; the rest remain in order. This does not change recipe loading. A data pack
    /// changes them on `/reload`, so look them up again when you need them. Empty
    /// for a type nobody uses and outside a world. These are the server's table on
    /// the server thread, a dedicated server or a LAN host; a client, the
    /// single-player window included, sees vanilla's and the content mods' recipes,
    /// not the world's data packs', unless a mod had the server send it the type
    /// (`native_datapack_sync`).
    fn recipes_of_type(&self, kind: &str) -> Arc<[Recipe]>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// An ingredient from recipe-file JSON text: an item, a `#tag`, or a list
    /// of them, a bare id as `minecraft:` (as vanilla reads it); `test` says
    /// whether a stack fits. Items the game doesn't know are left out; None
    /// for missing tags, invalid ingredients or no known named items, and
    /// outside a world. An existing-empty tag returns an empty ingredient. This helper
    /// parses standalone JSON; it does not bind a mod's custom kind. To test a custom
    /// ingredient already resolved at load, use [`Ingredient::test`]. Invalid JSON is
    /// refused with `RecipeJsonError::InvalidJson`.

    fn recipe_ingredient(&self, json: &str) -> Result<Option<Ingredient>, RecipeJsonError>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// A stack from recipe-file `result` JSON text: `id` (a bare one as
    /// `minecraft:`), `count` (1 by default, up to 99) and `components`. None
    /// without an `id`, for an item the game doesn't know or a count outside
    /// that, and outside a world.
    /// Invalid JSON is refused with `RecipeJsonError::InvalidJson`.
    fn recipe_result(&self, json: &str) -> Result<Option<ItemStack>, RecipeJsonError>;
    /// Side: both.
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    fn known_recipes(&mut self) -> Arc<[i32]>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The validated category and icon of a custom recipe type, or None for
    /// an undeclared/invalid type, before resources load, or outside a world.
    /// Returns the first category in category-id order when a type has several.
    /// This local declaration may include placement predicates; it is not the category
    /// data sent to a remote client. Any invalid declaration refuses the complete generation.
    /// Rebuilt when the recipe table changes. Does not unlock any recipes.
    fn recipe_book_category(&mut self, kind: &str) -> Option<RecipeBookCategory>;
}
/// The client host's methods of the `recipes` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait RecipesClientHost {
    /// Side: client.
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items, and items dragged off
    /// the panel onto the screen (`PanelEvent::Dropped`), come back as
    /// `panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    fn panel_items(&mut self, mod_id: &str, items: Vec<Stack>);
    /// Side: client.
    /// How mod `mod_id`'s item panel (`panel_items`) looks from now on:
    /// `PanelStyle::RecipeBook` (the default: the recipe book's page arrows
    /// and a plain search box), `PanelStyle::Viewer` (square page buttons on
    /// a bar over the items and the viewer's search box, as Sunrise's Recipes
    /// panel draws them) or `PanelStyle::Classic` (JEI's list on Minecraft
    /// 1.12: flush right under page buttons that always show and wrap around,
    /// the search box at the foot of the screen with a settings button that
    /// opens the mod's page in Options > Mods). The viewer's look changes
    /// only the drawing; the classic one also lays the panel out. Kept for
    /// the session, whether or not the panel is showing.
    fn panel_style(&mut self, mod_id: &str, style: PanelStyle);
    /// Side: client.
    /// Sets the text in mod `mod_id`'s item panel search box (`panel_items`),
    /// called by the panel's owner itself: `mod_id` must be the mod whose hook
    /// or part is making the call and whose panel is showing; any other call
    /// changes nothing, so one mod can't rewrite another's search (another mod
    /// asks the owner, through the owner's published integration). The text,
    /// cut to the box's 50 characters counted in UTF-16 code units as vanilla's
    /// box counts them, goes into the box the next time the owner's panel draws
    /// its box, never during this call: the box's value is replaced, the cursor
    /// goes to its end, the panel goes back to its first page, and the owner
    /// hears `panel_event` `Search(text)`, the event the player's typing sends.
    /// No keystrokes are simulated. A later call before that replaces an
    /// earlier one; the player editing the box first, or another mod taking
    /// the panel, drops it; text the box already holds changes nothing, its
    /// cursor and selection included. Set while no container screen is open,
    /// it waits for the panel to appear. A viewer `jei` clearing its own box
    /// calls `host.panel_search("jei", "")` from one of its hooks. JEI's
    /// `IIngredientFilter.setFilterText`.
    fn panel_search(&mut self, mod_id: &str, text: &str);
    /// Side: client.
    /// Shows `page` in mod `mod_id`'s item panel instead of its items, drawn by
    /// the game with its own slots (`RecipePage`, `RecipeView`): a title and a
    /// back button, tabs, and the recipes paged to the panel's height. `None`,
    /// or a page without recipes, shows the items again. The renderer's limits
    /// are cut, never refused: 256 recipes and 16 tabs a page, 27 slots, 4
    /// results and 8 fluids each way a recipe, 64 stacks a slot. The same title
    /// and tab again keep the page the player turned to. Clicks on its slots,
    /// actions, page turns, the back button and tabs come back as `panel_event`.
    fn panel_recipes(&mut self, mod_id: &str, page: Option<RecipePage>);
    /// Side: client.
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    fn place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String>;
    /// Side: client.
    /// Move available ingredients for a viewer recipe into the open crafting grid,
    /// without requiring the recipe to be learned. The screen shows any refusal.
    fn transfer_recipe(&mut self, book_id: i32);
    /// Side: client.
    /// A viewer's "+" for a recipe by its id, into the open menu: a machine's mod menu as
    /// well as a crafting grid. The server asks the `menu_transfer` part of the open mod menu's
    /// type first, then makes the game's own grid transfer; the screen shows the result as for
    /// `transfer_recipe`. `max` moves as many sets as the items allow (Shift). Shown as
    /// `NotIntoMenu` with no container screen open, in spectator mode, and on a server that
    /// doesn't take viewer transfers. In a solo world or on a LAN host the game is the server:
    /// its own player's transfer takes the same path, part first, then the grid.
    fn transfer_recipe_id(&mut self, recipe: &str, max: bool);
    /// Side: client.
    /// What `transfer_recipe_id(recipe, false)` would do now for the recipe
    /// with id `recipe` (`minecraft:stick`), worked out by the server without
    /// moving anything: the open mod menu's `menu_transfer` part asked first,
    /// then the grid, with the player's inventory, the grid and every
    /// container a mod lends the grid's recipes (a crafting station's chests,
    /// which the client can't see) counted. Any recipe the server knows,
    /// whether or not the player's recipe book holds it. For a viewer's "+",
    /// to mark the inputs it lacks (`missing_cells`) and, for a mod menu whose
    /// part names them, the ones it could craft (`craftable_cells`). The first
    /// call for a recipe asks and returns None; the answer comes a moment later
    /// and is kept for that recipe while the menu, the inventory and the grid
    /// stay as they were. On a remote server, while the answer is wanted, the
    /// client asks again once the cached answer is one second old. None outside
    /// a screen that takes `transfer_recipe_id` and on a server that doesn't
    /// answer it.
    fn transfer_recipe_check(&mut self, recipe: &str) -> Option<RecipeTransferCheck>;
    /// Side: client.
    /// The `book_id`s of the recipe `recipe_id` (`minecraft:stick`), the ids
    /// `transfer_recipe`, `place_recipe` and `known_recipes` use: one for each
    /// way the recipe book shows it, in the book's order. For a recipe viewer,
    /// which has the recipe from `recipes_of_type`. A server numbers its
    /// recipes itself and names none of them, so an id is given only when the
    /// server sent the recipe book entry with that number and this client's
    /// own numbering of the recipe agrees with it: the entry is the same
    /// recipe (the same result, category and ingredients), so two different
    /// recipes with identical displays cannot be told apart. Empty otherwise: a
    /// recipe the server sent no entry for (the player's book does not hold
    /// it, or it was removed), one the server numbers differently, one the
    /// book doesn't show (campfire, stonecutting, smithing, a mod's own
    /// type), an unknown id, and outside a world. The answer changes when
    /// the server adds, replaces or removes entries and when recipes reload
    /// or arrive, and no hook announces a recipe book change: query it when
    /// you use it, rather than keeping an answer from `recipes_received`.
    fn recipe_book_ids(&mut self, recipe_id: &str) -> Arc<[i32]>;
}
/// The handles' reads of the `recipes` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait RecipesBackend {
    /// Side: both. Exact load-time membership for component-independent ingredients.
    /// Custom/component predicates anywhere in the tree are NotEnumerable.
    fn recipe_ingredient_item_keys(
        &self,
        ingredient: &Ingredient,
    ) -> Result<Arc<[ItemKey]>, IngredientKeysError>;
    /// Side: both.
    /// Whether `stack` fits `ingredient` (`Ingredient.test`), matched with this
    /// game's loop context.
    fn recipe_ingredient_test(&self, ingredient: &Ingredient, stack: &ItemStack) -> bool;
}
