# Painting a GUI command buffer

`DrawContext::gui_paint(&[GuiDraw])` draws a buffer of vanilla's `GuiGraphics` calls in order, under their poses
(`GuiDraw::Push` / `Pop`). Every op in it is drawn, or none is: an op the paint cannot draw refuses the whole buffer
with a `GuiPaintError` that names it and its index, so a mod can draw it another way.

```rust,ignore
use mod_api::client::hud_layout::GuiDraw;

// The first hotbar slot's stack at twice its size, as the hotbar's pick-up "pop" squeezes it.
let drawn = draw.gui_paint(&[GuiDraw::Item {
    slot: 0,
    x: 20,
    y: 20,
    seed: 0,
    scale: [2.0, 2.0],
    pivot: [28.0, 28.0],
}]);
if drawn.is_err() {
    // Not drawn: fall back to a stack of our own.
    draw.item_stack(&stack, [20.0, 20.0], 16.0);
}
```

## Items

`GuiDraw::Item { slot, x, y, seed, scale, pivot }` draws the local player's stack in hotbar slot `slot` (0-8, or
40 for the off hand) as vanilla's hotbar does (`extractSlot`): the item (`GuiGraphicsExtractor.item`) with its top
left at `(x, y)`, scaled by `scale` about `pivot`, then its decorations (`itemDecorations`: count, damage bar,
cooldown) at `(x, y)` unscaled. An empty slot draws nothing and is not an error. The game's own hotbar draws its
items this way too. To draw a stack the player doesn't hold, use `DrawContext::item_stack`.

## Blits

`GuiDraw::Blit` draws part of a texture (`texture` is the path under `textures/`, without `.png`) with its
`blend`: `Textured` blends normally, `Vignette` darkens what is under it by the texture (`dst * (1 - src)`),
`Nausea` adds to it and `Crosshair` inverts it.

## Refusals

| `GuiPaintError` | When |
| --- | --- |
| `Text(TextFormError)` | A `Text` op's text cannot cross to the game (unrepresentable metadata). |
| `ItemSlot { index, slot }` | An `Item` of a slot other than 0-8 or 40. |
| `ItemNotFinite { index }` | An `Item` whose `scale` or `pivot` is NaN or infinite. |
| `BlitTexture { index, texture }` | A `Blit` of a texture the GUI atlas does not hold. |
| `BlitSize { index }` | A `Blit` whose `tex` size is not positive, or whose `uv` is NaN or infinite. |
| `Sprite { index, sprite }` | A `Sprite` or `AtlasSprite` the GUI atlas does not hold, a mob effect sprite (`mob_effect/...`, namespaced or not) included. |
| `SpriteRegion { index }` | A `Sprite` whose `region` has a sprite width or height of zero (or less). |
| `SpriteBlend { index, blend }` | A `Sprite` with the `Nausea` or `Vignette` blend, which sprites cannot be drawn with. |

The enum is `#[non_exhaustive]`. `index` is the op's place in the buffer; nothing of the buffer was drawn, the
ops before it included. A `TextFormError` converts into it with `?`.

- **Cost.** The check reads the buffer once before drawing it; a mod that doesn't paint pays nothing.
