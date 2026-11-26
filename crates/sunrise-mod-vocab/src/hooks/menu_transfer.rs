//! The `menu_transfer` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A recipe viewer's "+" on a mod menu (JEI's `IRecipeTransferHandler`):
    /// move one set of `recipe`'s ingredients (as many as the items allow
    /// with `max`) from `player`'s inventory into the open menu of a mod menu
    /// type, or with `dry_run` only say what that would do. Asked on the
    /// server before the game's own grid transfer, only for a mod menu type
    /// whose part answers, with `menu` its type, `pos` its block (None for a
    /// menu with no block) and `recipe` the recipe's id as the viewer sent
    /// it. Return `Some(Moves)` once moved, or `Missing`, `NoRoom` or
    /// `NotIntoMenu`; a part that returns None lets the game try its own
    /// crafting-grid transfer. A dry run must move nothing. With `dry_run` a
    /// part may also push the recipe's cells nothing fills to
    /// `missing_cells`, and the cells it lacks but could craft to
    /// `craftable_cells` (both empty by default), in `RecipeTransferCheck`'s
    /// cell order; the game keeps the ones below the recipe's cell count
    /// (its known layout: one for stonecutting, 256 for a recipe it can't
    /// inspect), each once, in ascending order, and ignores both outside a
    /// dry run.
    #[notes = r##"### Recipe transfer into a mod menu

A viewer's "+" (`RecipesClientHost::transfer_recipe_id`, or `transfer_recipe` for a recipe-book recipe)
names a recipe and the open window. The game's own transfer fills crafting grids only: a machine menu (an
inscriber, an alloy smelter, a pattern encoder) has none. The server first asks the `MenuPart` of the open
menu's type: `menu_transfer` gets the player, the menu type and block, the recipe id as sent (look it up with
the recipe reads; an id the server doesn't know is the part's to refuse), `max` for a Shift-click, and
`dry_run` for the viewer's check, which must move nothing. The part moves the ingredients itself with the
level host it is lent (the player's inventory through `ENTITY_ITEMS`, the block entity's container through its
capability) and returns a `RecipeTransferResult`; the viewer shows it as it shows the game's. A part that
returns None hands the transfer on to the game's grid transfer, which answers `NotIntoMenu` for a
menu without a crafting grid. Vanilla menu types are never asked. The contract is the same in a solo world and
for a LAN host's own player: the game is the server there, and asks the part on the same server-side path
(the player's own block container menu, its UUID, `dry_run` for the viewer's check) before its grid transfer.

Version 2 adds the dry run's cells. `missing_cells` names the recipe's cells no item fills and
`craftable_cells` the ones the menu lacks but could craft (a storage network's autocrafting), both as indices in
`RecipeTransferCheck`'s cell order: width by height for a shaped recipe, else its ingredients in order. They reach
the viewer as `RecipeTransferCheck::missing_cells` and `craftable_cells`, with any answer, so a part can answer
`Moves` and still mark the craftable cells. The game keeps both lists below the recipe's cell count, from its
known layout (one for a stonecutting recipe, three for smithing, the placement's slots for crafting and cooking;
256 for a recipe it can't inspect, such as a mod's own type), each once, in ascending order; pushes outside a dry
run are dropped."##]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "replace",
        version = 2,
        out = "result",
        scope = "menu",
        key = "menu",
        neoforge = "none (JEI IRecipeTransferHandler, which each mod implements for its menus)"
    )]
    fn menu_transfer(
        player: u128,
        menu: &str,
        pos: Option<[i32; 3]>,
        recipe: &str,
        max: bool,
        dry_run: bool,
        result: &mut Option<RecipeTransferResult>,
        missing_cells: &mut Vec<u32>,
        craftable_cells: &mut Vec<u32>,
    );
}
