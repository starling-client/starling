# Recipe book types and saved flags

Register a custom recipe book during shared mod registration. Its namespaced ID is
its stable identity for menus, per-player state, and negotiated synchronization.
Use an explicit `namespace:path`; vanilla’s built-in book IDs are reserved. These IDs use
`Identifier::parse_namespaced`: `..:pot` is invalid,
`example:kitchen/pot` is valid, and whitespace is not stripped.

`recipe_book_type("example:cooking")` saves the book's open and craftable-only
filtering flags in the engine's `sunrise:book_settings` compound inside the
player's `recipeBook` compound. Existing books using this declaration keep that
layout. Malformed entries are skipped independently, and well-formed entries are
kept when their mod is absent.

For a book that needs Java-compatible flat settings, declare the two saved field
names instead:

```rust,ignore
let cooking = registrar.recipe_book_type_with_fields(
    "example:cooking",
    "isModCookingGuiOpen",
    "isModCookingFilteringCraftable",
)?;
```

This follows vanilla's `RecipeBookSettings.TypeSettings` codec: both names are
fields directly inside `recipeBook`, beside `isFurnaceGuiOpen` and the other
vanilla flags. A true flag is written as `1b`; a false flag is omitted. Missing
fields read as false. A player file from a Java world with those field names
therefore restores the same flags. The book is not also written to the engine
extension, and extension values are not a fallback for declared flat fields.
Changing a book's storage declaration is not a migration mechanism.
Like other custom flags, explicit fields load independently of vanilla recipe-list
validation; malformed recipe lists do not erase them, unlike Java's whole packed decode.

Names preserve their exact spelling and may contain any characters. They must be
nonempty and distinct, and neither may clash with another book's declared field,
vanilla's flag fields, `recipes`, `toBeDisplayed`, or `sunrise:book_settings`.
Invalid declarations refuse installation even if the mod ignores the returned
error. Registration freezes before player files are loaded.

Each saved compound key must fit Java's `DataOutput.writeUTF`: at most 65,535
modified UTF-8 bytes, counting NUL as two bytes and supplementary characters as
six. Registration refuses a longer extension book ID or explicit flat field name
and names the declaring mod. A flat-field book's ID is not a saved key. The wire
limit is separate: an ID sent to a peer must fit vanilla's `Identifier` stream
codec, and each packet must fit the active connection's framing.

The saved field names do not change the book ID used by menus or synchronization.
Both flags live in the player's whole recipe-book state, rather than in a menu
instance. Custom panels wait for the server's negotiated settings snapshot before
using that state; local intent does not overwrite the authoritative flags. The
channel's version and negotiation, not the client's brand, decide whether the
peer receives custom settings and can place recipes. `PlayerRef::set_recipe_book_settings`
changes server state; when called in a transaction it publishes its settings-change
hook and client update only on commit.

## Toggle button position

A menu that hosts a book reserves a 20 by 18 pixel spot for its toggle button with
`MenuScreenLayout::recipe_book_toggle`, a `RecipeBookToggle { x, y, from }`: `x` pixels right of the menu's left edge,
`y` pixels below the menu's top edge (`ToggleOrigin::Menu`) or below the window's centre
(`ToggleOrigin::ScreenCentre`). The centre is vanilla's `height / 2` with integer division, so a screen that
places its button at `height / 2 - 49` writes `RecipeBookToggle { x: 5.0, y: -49.0, from: ToggleOrigin::ScreenCentre }`
and lands on vanilla's pixel for every window height. Without an anchor the game tries a gutter beside the
menu, and refuses the book, naming the menu, when no spot fits in the window.

## Filter button look

A custom book's craftable-only button uses the crafting book's sprites and tooltip unless the
book declares its own look, once, right after registering the book:

```rust,ignore
registrar.recipe_book_filter_look(
    &cooking,
    RecipeBookFilterLook {
        enabled: "example:recipe_book/cooking_enabled".into(),
        disabled: "example:recipe_book/cooking_disabled".into(),
        enabled_highlighted: "example:recipe_book/cooking_enabled_highlighted".into(),
        disabled_highlighted: "example:recipe_book/cooking_disabled_highlighted".into(),
        tooltip: "example.recipe_book.cookable".into(),
    },
)?;
```

Each sprite is a 26 by 16 GUI sprite, with an explicit `namespace:path` following
the same identifier grammar as book IDs. The ID names the file
`assets/<namespace>/textures/gui/sprites/<path>.png`; the highlighted pair shows while the cursor is
over the button. The tooltip is a translation key, shown while filtering is on; while it is off the
button keeps vanilla's `gui.recipebook.toggleRecipes.all`. Only the book's own mod may declare its
look. A second declaration, a malformed sprite ID, or an empty tooltip key refuses
installation even if the mod ignores the returned error. Missing sprites draw nothing.

A category's tab can show two icons: set `RecipeBookCategory::secondary_icon` (see
[Recipe book categories and displays](../how-to/items/recipe-book-categories.md)).
