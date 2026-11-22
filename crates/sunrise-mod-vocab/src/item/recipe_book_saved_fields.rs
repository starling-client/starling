//! Saved `recipeBook` field names shared by registration and player-file persistence.

/// Vanilla open/filtering pairs, in crafting, furnace, blast-furnace, smoker order.
pub const VANILLA: [(&str, &str); 4] = [
    ("isGuiOpen", "isFilteringCraftable"),
    ("isFurnaceGuiOpen", "isFurnaceFilteringCraftable"),
    (
        "isBlastingFurnaceGuiOpen",
        "isBlastingFurnaceFilteringCraftable",
    ),
    ("isSmokerGuiOpen", "isSmokerFilteringCraftable"),
];
/// Known recipe IDs.
pub const RECIPES: &str = "recipes";
/// Recipe IDs still to highlight.
pub const TO_BE_DISPLAYED: &str = "toBeDisplayed";
/// Engine-owned flags for books without explicit flat fields.
pub const EXTENSION: &str = "sunrise:book_settings";
