# Set the item panel's search text

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
// In the viewer's own panel_event hook (its mod id is "jei"): the player
// pressed its "clear" key, so the viewer empties its own search box.
host.panel_search("jei", "");
```

This is JEI's `IIngredientFilter.setFilterText`; the recipe book's own box does the same with
`searchBox.setValue`.
