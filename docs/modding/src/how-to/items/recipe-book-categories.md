# Recipe book categories and displays

Use `Hooks::recipe_book_categories` to supply categories and displays for your custom recipe
types. Append `RecipeBookCategory` values to `event.categories`. Register the book identity with
`recipe_book_type` during content registration and retain its `RecipeBookTypeKey`; declaring a
category does not register a book, change its settings or unlock recipes.

For Java-compatible saved flag names, use `recipe_book_type_with_fields`; see
[Recipe book types and saved flags](../../concepts/recipe-book-types.md).

This example assumes `book` is your registered key and the named menu, items and recipe exist:

```rust,ignore
use mod_api::{RecipeBookCategory, RecipeBookDisplay, RecipeBookDisplayEntry, RecipeSlotDisplay};

event.categories.push(RecipeBookCategory {
    id: "mymod:crushing".into(),
    recipe_type: "mymod:crusher".into(),
    icon: "mymod:crusher".into(),
    secondary_icon: None,
    target_menu: "mymod:crusher".into(),
    book: book.clone(),
    search_group: "mymod:crusher_search".into(),
    displays: vec![RecipeBookDisplayEntry {
        recipe: "mymod:crushed_stone".into(),
        displays: vec![RecipeBookDisplay::Shapeless {
            ingredients: vec![RecipeSlotDisplay::Item("minecraft:stone".into())],
            result: RecipeSlotDisplay::Item("minecraft:gravel".into()),
            station: RecipeSlotDisplay::Item("mymod:crusher".into()),
        }],
        placement: None,
    }],
});
```

## Declaration rules

| Field | Requirement |
| --- | --- |
| `id` | Unique namespaced category identifier outside `minecraft:`. |
| `recipe_type` | Namespaced identifier outside `minecraft:` present in the recipe table. Multiple categories can use one type. |
| `icon` | Registered item identifier other than `minecraft:air`. |
| `secondary_icon` | `None` for a one-icon tab, or a registered item identifier other than `minecraft:air` drawn beside `icon`, as vanilla's two-icon tabs do. |
| `target_menu` | Registered menu identifier. |
| `book` | Registered `RecipeBookTypeKey`. All categories in one book name the same menu and search group. |
| `search_group` | Namespaced identifier outside `minecraft:` owned by exactly one book; it cannot also name a category. Search includes exactly that book's categories. |
| `displays` | Recipe entries belonging to this type. Each recipe identifier occurs only once across the declaration batch. |

Build displays from your type's recipe JSON through `Host::recipes_of_type`. The hook runs lazily
once per recipe table, not per frame. A client reads its own recipe table unless
`datapack_sync` supplies the server's custom type.

`host.recipe_book_category(recipe_type)` returns a validated declaration or `None` for an
undeclared or invalid type, before resources load, or outside a world. When a type has several
categories, it returns the first in category-id order. Implement
`Hooks::recipe_book_categories` to supply your declarations.

## Display and placement rules

`RecipeBookDisplay` provides `Shaped`, `Shapeless`, `Furnace`, `Stonecutter` and `Smithing` layouts,
and `Custom { slots, result, station }` for a recipe type whose pictures none of them fit, such as a
cooking pot's inputs beside a container. A custom display's slot meaning belongs to the recipe
type: the book lists its non-empty `slots` in order as the recipe's ingredients (all of them count
in the craftable filter) and never draws `station`. The four built-in menu layouts map none of it, so a
menu bound to one refuses a category that holds a custom display.
Each recipe entry can supply several displays. `RecipeSlotDisplay` supports items, owned
`ItemStack` values, tags, fuel, empty slots, alternatives through `Composite`, potion and
component filters, dyeing, smithing trims and remainders.

- Resource references must resolve, and owned stacks must pass stack validation.
- A shaped display has positive width and height, each at most `i32::MAX`, and exactly
  `width * height` ingredient cells.
- Furnace duration is at most `i32::MAX`; experience is finite and nonnegative.
- Nested slot displays have a maximum depth of 32, with the outer slot at depth 0.
- When your menu has a registered `RecipeBookLayout`, its book and recipe type must match
  the category. Displays must fit the layout.
- For a recipe type bound to a menu placement layout, supply authoritative ingredient cells in
  `RecipeBookDisplayEntry::placement` using `PlacementInfo`. Display alternatives do not imply
  placement ingredients. The example's `None` does not declare a placement plan.

## Validation failures

The game validates the complete declaration generation before publishing any category. Invalid
identifiers, missing resources, duplicate categories or recipes, conflicting book ownership,
invalid layouts and invalid placement refuse the entire generation rather than skipping an entry.
The diagnostic begins `recipe book declarations refused:` and includes the typed error, such as
`InvalidId`, `UnknownResource`, `DuplicateCategory`, `DuplicateRecipe`, `ConflictingBook`,
`InvalidLayout` or `InvalidPlacement`.

Both successful and refused generations are cached until the recipe table changes. A recipe-table
replacement, new session or server `/reload` rebuilds declarations. Categories are ordered by
category identifier; a book's search group includes only its own categories, not categories from
other books sharing a menu.

## Binding a book to a menu

In `Hooks::register`, call `r.menu_recipe_book(menu, &book, recipe_type, layout)`.
Both `menu` and `recipe_type` must be full `namespace:path` identifiers accepted by
`Identifier::parse_namespaced`; neither part may be empty.
The layout maps recipe roles to container backing indices, not player inventory slots.
Every declared role must be in the content menu and agree with its crafting grid if it has one.
Register a client factory for the same menu with `r.register_menu_screen(menu, factory)`.
The factory returns `MenuScreen::new(layout)` with the geometry and sets
`MenuScreenLayout::recipe_book` to `Some(book.clone())`. Content loading checks the layout's
recipe roles, not whether the client factory is registered. If the factory is missing when
an open arrives, the client logs a warning and leaves its current menu unchanged. The screen
does not select the recipe type or determine which slots take server clicks. A book added only
to the screen, without a menu binding, is display-only. Either kind waits for authoritative book settings.

For a block-backed crusher with one persistent ingredient slot and one result slot, declare
`data/mymod/sunrise/menu/crusher.json` as `{ "slots": 2, "results": [1] }`. The block's
container opens that menu when used. Call this during `Hooks::register`, retain the returned
key for `recipe_book_categories`, and use the same menu and recipe type in the category.
Supply `PlacementInfo` for the recipes, unlike the display-only example above.

```rust,ignore
use mod_api::{
    ClientHost, MenuScreen, MenuScreenLayout, MenuScreenOpen, ModContainer, RecipeBookLayout,
    RecipeBookToggle, RecipeBookTypeKey, Registrar, ScreenSlot, ToggleOrigin,
};
use std::sync::Arc;

fn register_crusher(r: &mut dyn Registrar) -> Result<RecipeBookTypeKey, String> {
    const MENU: &str = "mymod:crusher";
    let block = r.block_key("mymod:crusher");
    r.block_container(&block, ModContainer {
        slots: 2,
        faces: std::array::from_fn(|_| vec![0, 1]),
        inputs: vec![0],
        outputs: vec![1],
        insert_rules: vec![],
        menu: Some(MENU.into()),
    });
    let book = r.recipe_book_type("mymod:crusher_book")?;
    r.menu_recipe_book(MENU, &book, "mymod:crusher", RecipeBookLayout::Crafting {
        width: 1,
        height: 1,
        slots: vec![0],
        result: 1,
    })?;

    let screen_book = book.clone();
    r.register_menu_screen(MENU, Arc::new(
        move |_: &dyn ClientHost, _: MenuScreenOpen<'_>| {
            Some(MenuScreen::new(MenuScreenLayout {
                width: 176.0,
                height: 166.0,
                slots: vec![
                    Some(ScreenSlot { position: [44.0, 35.0], no_item_icon: None }),
                    Some(ScreenSlot { position: [116.0, 35.0], no_item_icon: None }),
                ],
                inventory_x: 8.0,
                inventory_y: 84.0,
                recipe_book: Some(screen_book.clone()),
                recipe_book_toggle: Some(RecipeBookToggle {
                    x: 8.0, y: 33.0, from: ToggleOrigin::Menu,
                }),
                ..MenuScreenLayout::default()
            }))
        },
    ))?;
    Ok(book)
}
```

Give the block its usual storage and processing behavior; the recipe book's layout places
ingredients in slot 0 but does not run the crusher or fill its result slot. Screen-local moves
and hides affect where ghosts draw, not which backing indices the server uses.

## Placing a recipe yourself

`Hooks::player_recipe_book_click` observes a known recipe clicked in the current vanilla or mod
book, after display, knowledge and menu checks and before placement. For a `Mod` layout it runs
before the menu part's `recipe_book_place` decision, not instead of it.

A menu whose inputs are not a crafting grid, a furnace, a stonecutter or a smithing table (a cooking pot
with six inputs, a preview slot and a container slot) registers `RecipeBookLayout::Mod` as its layout.
The game then maps no display to a slot and answers a recipe click through your
`MenuPart`'s `recipe_book_place` hook. Before the hook runs the game checks what vanilla checks: the
player is not a spectator, the window is the open menu of this type, the menu is still valid, the
player's book holds the recipe, and its `PlacementInfo` is not `NOT_PLACEABLE`. Every recipe of a
`Mod` menu supplies a `PlacementInfo`; its ingredients are what `RecipeBookPlacement::Grid` moves.

```rust,ignore
fn recipe_book_place(
    &self,
    _host: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::RecipeBookPlaceEvent<'_>,
) {
    // The first five physical container slots take ingredients as a 3 by 2 grid,
    // clearing slots 0..6 back to the player's inventory first.
    *event.placement = mod_api::RecipeBookPlacement::Grid {
        width: 3,
        height: 2,
        slots: (0..5).collect(),
        clear: (0..6).collect(),
    };
}
```

`event.placement` arrives as `RecipeBookPlacement::Nothing` and chooses one of three answers:

- `RecipeBookPlacement::Nothing`: handle placement yourself through normal `Host` operations or do nothing.
- `RecipeBookPlacement::Ghost`: a manual menu reports missing ingredients, so while the same opening
  remains current the server sends the clicked display to the client to draw the recipe's ghosts
  (`recipe_book_ghost`).
- `RecipeBookPlacement::Grid { width, height, slots, clear }`: run vanilla's canonical placer
  (`ServerPlaceRecipe.placeRecipe`) once on the active menu and its live backing storage using the
  actual physical container slot identities in `slots` and `clear` and the click's current Shift
  (`event.use_max_items`) and creative (`event.creative`) flags, clearing `clear` (and the grid's own
  slots) first, syncing normal slot changes, and sending a server-origin ghost only when ingredients
  are missing. A recipe never reaches a cell past the last slot given.

After a mutating callback returns, the engine resolves the menu only to verify the same opening and uses
its current state. Refused `Grid` inputs (a slot the menu lacks, hides or cannot store, a repeated slot,
an empty grid, a recipe larger than the slots given, or a slot whose pickup a part refuses) are logged
on each attempt without applying the grid. As in vanilla, the survival inventory room check applies
to the input grid, not to extra slots on the clear list. A panic rolls back the
callback's nested transaction and discards its `placement`. Only a panic disables the callback for
the session, naming the mod in logs and error toasts. A menu with no part that listens refuses every click.

## Drawing the ghosts yourself

When a click finds items missing, the server tells the client and the client draws the recipe's ghosts in
the menu's slots. The `recipe_book_ghost` hook of your `MenuPart` is asked just before, where vanilla
calls the book component's `fillGhostRecipe`:

```rust,ignore
fn recipe_book_ghost(
    &self,
    host: &dyn mod_api::ClientHost,
    event: &mut mod_api::RecipeBookGhostEvent<'_>,
) {
    // A pot: the meal preview in slot 6 and the inputs in slots 0 and up.
    if let mod_api::RecipeBookDisplay::Custom { slots, result, .. } = event.display {
        event.ghosts.clear();
        event.ghosts.push(mod_api::GhostSlot { slot: 6, display: result.clone(), result: true });
        for (index, slot) in slots.iter().enumerate() {
            event.ghosts.push(mod_api::GhostSlot { slot: index as u16, display: slot.clone(), result: false });
        }
    }
}
```

`display` is the alternative the player clicked, sent directly by the server for the current opening.
`ghosts` arrives holding the game's own ghosts for the menu's layout (the crafting, furnace, stonecutter
and smithing layouts map the display to their slots; a `Mod` layout arrives empty); add, change or clear
them. Following vanilla's `GhostSlots.setSlot`, ghosts are resolved against the screen's actual shown
slots with no separate count cap: a nonempty resolved display replaces any prior ghost for the same
physical slot in place (preserving slot order), while an unresolved empty display is dropped without
erasing a prior ghost, and a ghost in a slot the menu's screen hides or lacks is omitted. The hook fires
on the client only, once per server ghost response, for the open menu's book, and only for a menu whose
type has a part that listens.

`MenuScreenLayout::recipe_book_big_result` gives the result ghost vanilla's 24 by 24 frame
(`isBiggerResultSlot`). It is off by default.
