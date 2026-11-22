# Show recipes in the item panel

A recipe viewer shows its pages in the item panel's place with `panel_recipes(mod_id, page)`, a
client `WorldOnly` call beside `panel_items` and `panel_style`. The page is data: the game draws it
with its own slots, arrows, fluid wells and buttons. It is
the same recipe view the game's own panel uses, so a mod never draws a second recipe window.

```rust
use mod_api::{RecipeLayout, RecipePage, RecipeResult, RecipeView};

fn show_planks(host: &mut dyn mod_api::ClientHost, log: mod_api::Stack, planks: mod_api::Stack) {
    let craft = RecipeView::grid(1, 1)
        .with_slots(vec![vec![log.clone()]])
        .with_results(vec![RecipeResult::new(vec![planks])]);
    let smelt = RecipeView::new(RecipeLayout::Cooking)
        .with_slots(vec![vec![log]])
        .with_note("10 s")
        .with_action(true);
    let page = RecipePage::new("Oak Planks", vec![craft, smelt]);
    host.panel_recipes("my_viewer", Some(page));
}
```

`None`, or a page with no recipes, shows the mod's items again. Showing the same title and tab
again keeps the page the player turned to.

## What a page holds

- `RecipePage`: a `title` beside the back button, a row of `tabs` (each an icon stack; `tab` is the
  selected one, set with `with_tabs`) and the `recipes`.
- `RecipeView`: a `layout`, the input `slots` (each a list of stacks the slot cycles through, one a
  second), up to four `results`, the fluid wells (`with_fluids`, amounts in millibuckets), a `note`
  line and an optional action button (`with_action(enabled)`; `false` draws it greyed).
- `RecipeLayout`: `Grid` (`width` x `height`, up to 3 x 3; slots past the grid continue in rows under
  it), `Cooking` (one input over a flame), `Single` (the stonecutter), `Smithing` and `Brewing`.
- `RecipeResult`: the stacks a result slot cycles through and an optional chance in percent, drawn
  under it ("25%").

## Limits

The renderer's limits are cut, never refused, and a call never panics:

| Item | At most |
|---|---|
| Recipes on a page | 256 (`RecipePage::MAX_RECIPES`) |
| Tabs | 16 (`RecipePage::MAX_TABS`) |
| Input slots in a recipe | 27 (`RecipeView::MAX_SLOTS`) |
| Results in a recipe | 4 (`RecipeView::MAX_RESULTS`) |
| Fluid wells, each way | 8 (`RecipeView::MAX_FLUIDS`) |
| Stacks a slot or result cycles | 64 (`RecipeView::MAX_STACKS`) |

A grid's `width` and `height` are kept to 1 to 3, and a chance to 0 to 100; a chance that isn't
finite draws none. A stack of an item the game doesn't know is left out of its slot's cycle.

## What the player does

The page's events come back through `panel_event`, as the items' do:

- `PanelEvent::SlotClicked { recipe, slot, stack, right }`: a click on a slot. `recipe` is the
  recipe's index in the page's `recipes`; results are numbered after the slots.
- `PanelEvent::Action(recipe)`: the recipe's action button.
- `PanelEvent::Page(first)`: the player turned the page; `first` is the first recipe now shown.
- `PanelEvent::Back`: the back button. The mod chooses what follows, often `panel_recipes(mod_id,
  None)`.
- `PanelEvent::Tab(index)`: the player picked a tab.

Looking recipes up stays the viewer's job: it reads them (`recipes_of_type`) and builds the pages.
A machine mod with its own recipe types supplies its pages through the viewer's published
integration; the engine has no hook between the two.
