//! Recipe-provided book displays. Recipe JSON remains owned by its recipe type.
use crate::*;

/// What one slot in a recipe display shows. Resource references use names, never
/// process-local registry ordinals. The host resolves these at recipe load.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: a closed set of slot forms the host resolves without a wildcard
pub enum RecipeSlotDisplay {
    Empty,
    AnyFuel,
    Item(String),
    ItemStack(ItemStack),
    Tag(String),
    WithAnyPotion(Box<RecipeSlotDisplay>),
    OnlyWithComponent {
        source: Box<RecipeSlotDisplay>,
        component: String,
    },
    Dyed {
        dye: Box<RecipeSlotDisplay>,
        target: Box<RecipeSlotDisplay>,
    },
    SmithingTrim {
        base: Box<RecipeSlotDisplay>,
        material: Box<RecipeSlotDisplay>,
        pattern: String,
    },
    WithRemainder {
        input: Box<RecipeSlotDisplay>,
        remainder: Box<RecipeSlotDisplay>,
    },
    Composite(Vec<RecipeSlotDisplay>),
}

/// Vanilla's five recipe display layouts, independent of the recipe's JSON, and `Custom` for a
/// recipe type whose pictures none of them fit.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: the five vanilla layouts and the custom one, which the host validates without a wildcard
pub enum RecipeBookDisplay {
    Shaped {
        width: u32,
        height: u32,
        ingredients: Vec<RecipeSlotDisplay>,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
    },
    Shapeless {
        ingredients: Vec<RecipeSlotDisplay>,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
    },
    Furnace {
        ingredient: RecipeSlotDisplay,
        fuel: RecipeSlotDisplay,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
        duration: u32,
        experience: f32,
    },
    Stonecutter {
        input: RecipeSlotDisplay,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
    },
    Smithing {
        template: RecipeSlotDisplay,
        base: RecipeSlotDisplay,
        addition: RecipeSlotDisplay,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
    },
    /// A display whose slots mean what the recipe type says: a cooking pot's inputs with a
    /// container beside them, for one. The book shows `slots` in order as the recipe's
    /// ingredients (empty slots hidden), counts every non-empty one in the craftable filter and
    /// shows `result` as the recipe's button; `station` is never drawn. The built-in menu layouts
    /// (crafting, furnace, stonecutter, smithing) map none of it, so a menu bound to one refuses
    /// a custom display; the mod places and draws ghosts for it itself.
    Custom {
        slots: Vec<RecipeSlotDisplay>,
        result: RecipeSlotDisplay,
        station: RecipeSlotDisplay,
    },
}

// no wire: an entry's placement can retain native ingredient predicates; only display data is portable.
/// Displays supplied by a recipe type for one recipe. A recipe may have several
/// displays. Declarations are evaluated once per recipe-table load, not per frame.
/// `displays` describes pictures; `placement` is a separate local requirement that
/// can retain native predicates. Send only the display data needed by a remote viewer.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: mods construct complete recipe display declarations with a literal
pub struct RecipeBookDisplayEntry {
    /// Recipe id in the owning category's recipe type.
    pub recipe: String,
    pub displays: Vec<RecipeBookDisplay>,
    /// Authoritative ingredient cells for this recipe, independent of its
    /// pictures. Required when the owning recipe type is bound to a menu's
    /// placement layout. `None` is not an inference from display alternatives.
    pub placement: Option<crate::PlacementInfo>,
}
