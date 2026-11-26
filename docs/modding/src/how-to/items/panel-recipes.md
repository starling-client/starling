# Show recipes in the item panel

A recipe viewer shows its pages in the item panel's place with `panel_recipes(mod_id, page)`, a
client `WorldOnly` call beside `panel_items` and `panel_style`. The page is data: the game draws it
with its own slots, arrows, fluid wells and buttons. It is
the same recipe view the game's own panel uses, so a mod never draws a second recipe window.

```rust
{{#include ../../../examples/src/item_behaviors.rs:show_planks}}
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

## Dragging an item onto the screen

The player can drag an item off the panel's item list and drop it on the open screen, as JEI's
ghost ingredients do, to set a filter or configuration slot to an item they don't hold. A press on
an item becomes a drag once the pointer moves 4 GUI pixels with the left button held; the item
follows the pointer until the button comes up. A plain click stays a click: a left click's
`PanelEvent::Clicked` comes when the button comes up without having dragged (a right click, or a
left one while the player carries a stack, still comes as the button goes down).

- Let go outside the panel: the owner hears `PanelEvent::Dropped { stack, x, y }`, the item as the
  mod sent it and the point in GUI pixels from the window's top left (`ScreenView::mouse`'s
  frame). No slot of the screen hears that release: nothing is picked up, placed or thrown.
- Let go back on the panel, press Escape, switch away from the window, or close or replace the
  menu: the drag is cancelled and nothing is sent. Escape during a drag leaves the screen open. A
  press belongs to the menu that was open when it went down, and its release never reaches the
  screen underneath, even after a cancel. A screen hook that cancels the release drops the item
  with nothing sent.
- A click comes only while the list still has the pressed item at its index; a drag drops the item
  as it was when pressed.

The game moves nothing for a drop. The viewer finds what lies under `x, y` (a slot of a menu it
knows, or a target another mod published), forwards the drop to the mod that owns it, and that mod
checks the change on its own server; a client never decides a slot's contents.

```rust,ignore
fn panel_event(&self, host: &mut dyn mod_api::ClientHost, event: &mut mod_api::PanelEventEvent<'_>) {
    if event.mod_id != "my_viewer" {
        return;
    }
    if let mod_api::PanelEvent::Dropped { stack, x, y } = event.event {
        // Ask the target under (x, y) to take `stack` as a ghost: the target's mod sends it to
        // its server, which sets the filter slot if the player may.
        let _ = (host, stack, x, y);
    }
}
```

## Marking a recipe's cells

A viewer's "+" asks `transfer_recipe_check(recipe)` first. Its `RecipeTransferCheck` names the
recipe's cells, in the order of the recipe's slots (width by height for a shaped recipe, else its
ingredients in order): `missing_cells`, which nothing fills (drawn red, as JEI does), and
`craftable_cells`, which the open menu lacks but could craft (drawn blue: a storage network's
autocrafting). The game's own grid transfer names missing cells with `Missing` and never craftable
ones; a mod menu's `menu_transfer` part names both, with any answer (see
[Menu slots](../../concepts/capabilities/menu-slots.md#recipe-transfer-into-a-mod-menu)).

Looking recipes up stays the viewer's job: it reads them (`recipes_of_type`) and builds the pages.
A machine mod with its own recipe types supplies its pages through the viewer's published
integration; the engine has no hook between the two.

## Setting the item panel's search text

On the client, the mod that owns a `panel_items` panel can put text in its search box with
`panel_search(mod_id, text)`, passing its own id. It is a `WorldOnly` operation, like
`panel_style`.

- Only the owner can set its text: the call counts only when it comes from the owner's own hook
  or part and names the owner's id while its panel is showing. A call from any other mod changes
  nothing, even one that names the owner's id. A mod that wants another mod's viewer searched asks
  that viewer through its published integration, and the viewer makes the call.
- The text is cut to the box's 50 characters, counted in UTF-16 code units as vanilla's box counts
  them: a character outside the Basic Multilingual Plane, such as an emoji, takes two.
- Nothing changes during the call. The text goes into the box the next time the owner's panel
  draws its box: the value is replaced, the cursor goes to its end, and the panel goes back to its
  first page. A later call before then replaces an earlier one. Set while no container screen is
  open, or while the screen is too narrow for the panel, the text waits.
- The owner then hears `panel_event` `Search(text)`, the event the player's typing sends, so its
  filter changes in one place. No keystrokes are simulated: only the box's text and that one event.
  As the event comes after the call, calling `panel_search` from inside one of your own hooks is
  safe.
- A queued text is dropped if the player edits the box first (their edit is newer) or if another
  mod takes the panel. One mod's text never reaches another mod's box or hook.
- Setting the text the box already holds, once cut to 50, sends no event and leaves the cursor and
  selection where they are.

```rust
{{#include ../../../examples/src/item_behaviors.rs:clear_panel_search}}
```

This is JEI's `IIngredientFilter.setFilterText`; the recipe book's own box does the same with
`searchBox.setValue`.

## Choosing an item panel style

On the client, `panel_style(mod_id, style)` chooses how the mod's `panel_items` panel looks.
It is a `WorldOnly` operation: call it through the client host while a world is open, not from
server code or a title-screen callback. The choice is kept for the session even while the panel
is hidden; it is not a saved player preference.

In every style the mouse wheel pages the panel while the pointer is over it: one page per notch,
down to the next page and up to the previous one, stopping at the first and last page (it does not
wrap, even in Classic). Recipe pages shown in the panel's place turn the same way, and the mod hears
the turn as it does from the arrows. A notch over the panel does not reach the screen underneath;
scrolling anywhere else is unchanged. No hook is needed for this.

`PanelStyle` has three variants:

- `RecipeBook` (the default) uses recipe-book page arrows and a plain search field.
- `Viewer` uses square page buttons on a bar and the Recipes panel's search field. It changes
  drawing, not layout.
- `Classic { centred_search, window_width }` also changes layout: an 18 px ingredient grid sits
  2 px from the screen's right edge below an always-visible page row with wrapping navigation.
  The gap from the container window can exceed 2 px when the free width is not divisible by 18.
  The search field ends 4 px above the screen bottom by default. With `centred_search` and room
  under the window, it moves under the window and ends 2 px above the screen bottom instead.
  Nonempty searches with no matches are red; other Classic search text is white.

For Classic, `window_width: 0` uses the container's own bounds. A positive value reserves at least
that much width for a centred recipe window. This width option is Sunrise's own, not a JEI 1.12
setting. The Classic wrench opens the owning mod's page in **Options > Mods**, not a shared
viewer configuration screen. Only the page arrows are tinted; the wrench retains its own colours.

Switching back to RecipeBook or Viewer restores the normal search text colour, including after
a failed Classic search. Changing the style does not replace the panel's item list or its event
handling.
