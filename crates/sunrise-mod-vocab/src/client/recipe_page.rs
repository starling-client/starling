//! A page of recipes a client mod shows in its item panel's place
//! ([`RecipesClientHost::panel_recipes`](crate::RecipesClientHost::panel_recipes)), described as
//! data and drawn by the game with its own slots, arrows, fluid wells and buttons.
//!
//! The renderer has limits, and anything past them is cut, never refused: at most
//! [`RecipePage::MAX_RECIPES`] recipes and [`RecipePage::MAX_TABS`] tabs a page,
//! [`RecipeView::MAX_SLOTS`] slots, [`RecipeView::MAX_RESULTS`] results and
//! [`RecipeView::MAX_FLUIDS`] fluids each way a recipe, and [`RecipeView::MAX_STACKS`] stacks a
//! slot or result cycles through.
use crate::{FluidStack, Stack};

/// What the panel shows instead of a mod's items
/// ([`panel_recipes`](crate::RecipesClientHost::panel_recipes)): a title and a back
/// button, a row of tabs between them, and the recipes, as many as fit a page.
///
/// At most [`MAX_RECIPES`](Self::MAX_RECIPES) recipes and [`MAX_TABS`](Self::MAX_TABS) tabs;
/// those past them are left out. A page with no recipes shows the items again, as `None` does.
/// The same title and tab again keep the page the player turned to. The player's clicks come
/// back as `panel_event`: `PanelEvent::SlotClicked`, `Action`, `Page`, `Back` and `Tab` answer
/// this page.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client recipe-panel display data.
pub struct RecipePage {
    /// The line beside the back button (an item's name, a recipe type's).
    pub title: String,
    /// Each tab's icon, left to right; empty for no tab row.
    pub tabs: Vec<Stack>,
    /// The selected tab's index in [`tabs`](Self::tabs).
    pub tab: u32,
    /// The recipes, in order; `PanelEvent::SlotClicked` and `PanelEvent::Action` name one by its
    /// index here.
    pub recipes: Vec<RecipeView>,
}

impl RecipePage {
    /// Recipes a page shows; those past it are left out.
    pub const MAX_RECIPES: usize = 256;
    /// Tabs a page shows; those past it are left out.
    pub const MAX_TABS: usize = 16;

    /// A page of `recipes` under `title`, with no tabs.
    pub fn new(title: impl Into<String>, recipes: Vec<RecipeView>) -> Self {
        Self {
            title: title.into(),
            tabs: Vec::new(),
            tab: 0,
            recipes,
        }
    }

    /// The page with a row of `tabs` (each an icon), `tab` the selected one.
    pub fn with_tabs(mut self, tabs: Vec<Stack>, tab: u32) -> Self {
        self.tabs = tabs;
        self.tab = tab;
        self
    }
}

/// How the panel lays out a [`RecipeView`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client recipe-panel layout mode.
pub enum RecipeLayout {
    /// A crafting grid of `width` x `height` slots, an arrow and the results.
    #[default]
    Grid,
    /// One input over a flame, an arrow and the results (furnaces, smokers, campfires).
    Cooking,
    /// One input, an arrow and the results (the stonecutter).
    Single,
    /// Template, base and addition, an arrow and the results.
    Smithing,
    /// A potion and an ingredient, an arrow and the results.
    Brewing,
}

/// One result of a [`RecipeView`]: a slot cycling through `stacks`, one a second, and the
/// chance in percent the recipe gives it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client recipe-panel result slot display.
pub struct RecipeResult {
    /// The stacks the slot cycles through; at most [`RecipeView::MAX_STACKS`], those past it are
    /// left out.
    pub stacks: Vec<Stack>,
    /// The chance in percent, drawn under the slot (`Some(25.0)` draws "25%"), kept to 0 to 100;
    /// `None` (or a value that isn't finite) gives the result every time.
    pub chance: Option<f32>,
}

impl RecipeResult {
    /// A result the recipe always gives, cycling through `stacks`.
    pub fn new(stacks: Vec<Stack>) -> Self {
        Self {
            stacks,
            chance: None,
        }
    }

    /// The result given with this chance, in percent.
    pub fn with_chance(mut self, chance: f32) -> Self {
        self.chance = Some(chance);
        self
    }
}

/// A recipe the panel shows, in a [`RecipePage`]. Each slot, and each result, cycles through its
/// stacks, one a second; an empty slot shows nothing. A stack of an item the game doesn't know is
/// left out of its cycle.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local client recipe-panel entry display data.
pub struct RecipeView {
    /// How the slots are laid out.
    pub layout: RecipeLayout,
    /// The grid's width ([`RecipeLayout::Grid`] only), kept to 1 to 3.
    pub width: u32,
    /// The grid's height ([`RecipeLayout::Grid`] only), kept to 1 to 3; slots past the grid
    /// continue in rows under it.
    pub height: u32,
    /// Each input slot's stacks, in order; at most [`MAX_SLOTS`](Self::MAX_SLOTS) slots of
    /// [`MAX_STACKS`](Self::MAX_STACKS) stacks each, those past them left out.
    pub slots: Vec<Vec<Stack>>,
    /// The results, left to right after the arrow; at most [`MAX_RESULTS`](Self::MAX_RESULTS).
    /// `PanelEvent::SlotClicked` numbers them after the slots.
    pub results: Vec<RecipeResult>,
    /// The fluids the recipe takes, `amount` in millibuckets, each in a well with its amount
    /// under it; at most [`MAX_FLUIDS`](Self::MAX_FLUIDS). A well shows the fluid's client look
    /// (sprite and tint), vanilla's colour for water and lava, and grey for a fluid with none.
    pub fluid_inputs: Vec<FluidStack>,
    /// The fluids the recipe gives, drawn as [`fluid_inputs`](Self::fluid_inputs) are; at most
    /// [`MAX_FLUIDS`](Self::MAX_FLUIDS).
    pub fluid_outputs: Vec<FluidStack>,
    /// A short line under the recipe (a cooking time), or "".
    pub note: String,
    /// The action button beside the recipe: `None` for none, `Some(true)` enabled, `Some(false)`
    /// shown greyed. A press comes back as `PanelEvent::Action`.
    pub action: Option<bool>,
}

impl RecipeView {
    /// Input slots a recipe shows (three 3 x 3 grids' worth).
    pub const MAX_SLOTS: usize = 27;
    /// Results a recipe shows.
    pub const MAX_RESULTS: usize = 4;
    /// Fluid wells a recipe shows each way, in and out.
    pub const MAX_FLUIDS: usize = 8;
    /// Stacks one slot or result cycles through.
    pub const MAX_STACKS: usize = 64;

    /// An empty recipe laid out as `layout`: a 1 x 1 grid, no slots, results or fluids, no note
    /// and no action button.
    pub fn new(layout: RecipeLayout) -> Self {
        Self {
            layout,
            width: 1,
            height: 1,
            slots: Vec::new(),
            results: Vec::new(),
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: None,
        }
    }

    /// A [`RecipeLayout::Grid`] recipe of `width` x `height` slots.
    pub fn grid(width: u32, height: u32) -> Self {
        Self::new(RecipeLayout::Grid).with_size(width, height)
    }

    /// The recipe with a grid of `width` x `height` slots.
    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// The recipe with these input slots, each a list of stacks it cycles through.
    pub fn with_slots(mut self, slots: Vec<Vec<Stack>>) -> Self {
        self.slots = slots;
        self
    }

    /// The recipe with these results.
    pub fn with_results(mut self, results: Vec<RecipeResult>) -> Self {
        self.results = results;
        self
    }

    /// The recipe with these fluids taken in and given out.
    pub fn with_fluids(mut self, inputs: Vec<FluidStack>, outputs: Vec<FluidStack>) -> Self {
        self.fluid_inputs = inputs;
        self.fluid_outputs = outputs;
        self
    }

    /// The recipe with a short line under it.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    /// The recipe with an action button: enabled when `enabled`, else shown greyed.
    pub fn with_action(mut self, enabled: bool) -> Self {
        self.action = Some(enabled);
        self
    }
}
