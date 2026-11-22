//! A ghost a mod puts in a menu slot when a recipe click found items missing.
use super::RecipeSlotDisplay;

/// One ghost of the recipe book's ghost recipe (vanilla's `GhostSlots.setInput` and `setResult`):
/// what `display` resolves to is shown, cycling through its stacks, in the menu's slot `slot`.
/// A ghost that resolves to no stack is dropped, as vanilla drops it, and so is one in a slot the
/// menu's screen does not show.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: the game builds it with a literal; a listener fills one in
pub struct GhostSlot {
    /// The menu slot (its index in the menu's own slots, not the player inventory's).
    pub slot: u16,
    /// What the slot shows.
    pub display: RecipeSlotDisplay,
    /// The recipe's result: it gets the frame `MenuScreenLayout::recipe_book_big_result` asks for.
    pub result: bool,
}

impl GhostSlot {
    /// A ghost in menu slot `slot` that shows `display`; `result` marks the recipe's result.
    pub fn new(slot: u16, display: RecipeSlotDisplay, result: bool) -> Self {
        Self {
            slot,
            display,
            result,
        }
    }
}
