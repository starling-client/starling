//! Recipe book declarations supplied by a custom recipe type.
#[path = "placement_info.rs"]
mod placement;
#[path = "recipe_book_saved_fields.rs"]
pub mod saved_fields;
pub use placement::PlacementInfo;
#[path = "recipe_book_layout.rs"]
mod layout;
pub use layout::RecipeBookLayout;
#[path = "recipe_book_filter.rs"]
mod filter;
pub use filter::RecipeBookFilterLook;
#[path = "recipe_book_ghost.rs"]
mod ghost;
pub use ghost::GhostSlot;
#[path = "recipe_display.rs"]
mod display;
#[path = "recipe_book_settings.rs"]
pub mod settings;
#[allow(unused_imports)]
use crate::*;
pub use display::{RecipeBookDisplay, RecipeBookDisplayEntry, RecipeSlotDisplay};
pub use settings::{RecipeBookIdentity, RecipeBookSettings};
// no wire: the category contains native placement declarations, not the remote book projection.
/// A mod's book category and displays for one custom recipe type. The local
/// declarations can include placement predicates and are not a network payload.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct RecipeBookCategory {
    /// A namespaced category id outside `minecraft:`.
    pub id: String,
    /// A custom type present in the current recipe table.
    pub recipe_type: String,
    /// A registered item id used as the category's icon.
    pub icon: String,
    /// A second registered item id drawn beside `icon` on the tab, as vanilla's
    /// `RecipeBookComponent.TabInfo` takes one: two icons share the tab side by side.
    pub secondary_icon: Option<String>,
    /// The menu whose screen owns this category.
    pub target_menu: String,
    /// Registered book identity, separate from category identity and settings.
    /// Declaring a category does not itself register a book or change its flags.
    pub book: RecipeBookTypeKey,
    /// Search category aggregating exactly this book's categories.
    pub search_group: String,
    /// Recipe-type supplied displays, rebuilt with this declaration on reload.
    pub displays: Vec<RecipeBookDisplayEntry>,
}

/// What a recipe viewer's transfer into the open crafting grid does or would
/// do (`RecipesClientHost::transfer_recipe`, `transfer_recipe_id`,
/// `transfer_recipe_check`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the four answers of Sunrise's transfer channels, matched without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RecipeTransferResult {
    /// It moves a set of the recipe's ingredients into the grid.
    Moves,
    /// The items there are don't make a set.
    Missing,
    /// The grid's items have no room in the inventory.
    NoRoom,
    /// Not into the open menu: the recipe doesn't fit its grid, or the
    /// server doesn't know it.
    NotIntoMenu,
}

/// The server's answer to `transfer_recipe_check`: what the transfer would
/// do, and with `Missing`, the recipe's cells nothing fills.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct RecipeTransferCheck {
    /// What the transfer would do.
    pub result: RecipeTransferResult,
    /// With `Missing`, the recipe's cells no item fills: indices into its
    /// slots (width by height for a shaped recipe, else its ingredients in
    /// order). Empty otherwise.
    pub missing_cells: Vec<u32>,
}

/// A namespaced custom recipe book identity returned during registration.
/// This owns no settings; player settings and synchronization are separate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: its private identity field prevents external construction or destructuring
pub struct RecipeBookTypeKey {
    pub(crate) id: String,
}

impl RecipeBookTypeKey {
    /// Frozen registered book identities in stable namespaced order.
    /// Intended for connection negotiation and declaration validation, not ticks.
    pub fn registered() -> Vec<Self> {
        let mut keys: Vec<_> = crate::parts::installed()
            .map(|registry| {
                registry
                    .recipe_book_types
                    .keys()
                    .map(|id| Self { id: id.clone() })
                    .collect()
            })
            .unwrap_or_default();
        keys.sort_by(|a, b| a.id.cmp(&b.id));
        keys
    }

    /// Explicit saved field pairs by book id, in unspecified order. Books without a
    /// pair use the engine extension. Intended for player-file persistence, not ticks.
    pub fn registered_save_fields() -> Vec<(String, (String, String))> {
        crate::parts::installed()
            .map(|registry| {
                registry
                    .recipe_book_fields
                    .iter()
                    .map(|(id, pair)| (id.clone(), pair.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether this process has installed this identity in its frozen registry.
    pub fn is_registered(&self) -> bool {
        crate::parts::installed()
            .is_some_and(|registry| registry.recipe_book_types.contains_key(self.as_str()))
    }

    /// The stable namespaced identity used in negotiated payloads and, unless explicit
    /// save fields were declared, as the key in the saved engine extension.
    pub fn as_str(&self) -> &str {
        &self.id
    }
}
