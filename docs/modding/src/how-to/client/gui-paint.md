# Drawing GUI content

## Drawing directly with a canvas

`DrawContext::draw_gui` lends a `GuiCanvas` with an identity pose and the current context's
clip. Call its methods directly inside the callback, in draw order. A nested `scope` applies
pose operations in order to a copy of the current pose and optionally scissors to
`[x, y, w, h]` in the resulting local coordinates. That clip stays fixed in screen space even
when a descendant changes pose. The scissor restricts the pixels drawn, not the original vertices
or their colour and texture interpolation. Returning from a child scope leaves the parent pose
and clip as they were.

```rust,ignore
use mod_api::{GuiDrawError, Text, client::hud_layout::PoseOp};

let result: Result<(), GuiDrawError> = draw.draw_gui(&mut |canvas| {
    canvas.scope(
        &[PoseOp::Translate(20.0, 20.0), PoseOp::Scale(2.0, 2.0)],
        Some([0.0, 0.0, 60.0, 16.0]),
        &mut |panel| {
            panel.fill([0.0, 0.0, 60.0, 16.0], 0xC020_2020)?;
            panel.text(&Text::literal("60 FPS"), [4.0, 4.0], 0xFFFF_FFFF, true)
        },
    )
});
```

A refused primitive draws nothing. Use `?` to discard the whole group on a refusal, or handle
it to retain the other draws. Successful children commit in place, but any error returned by a
parent still discards them. If you handle a child's error, only that child is discarded. Returning
`Err(GuiDrawError::Abort)` or unwinding also discards the current group. Widgets, scenes and
independent host calls are not part of the transaction; use the ordinary `DrawContext` for those
draws, with [`with_scissor`](screens.md#clipping-what-a-screen-draws) if you need to clip them.

For a wrapped component, call `canvas.text_with_word_wrap`. It wraps and paints in the current
scope and returns the [label dimensions](screens.md#labels-wrapped-text-without-a-widget). The
canvas walks the wrapped ranges and paints each line using reusable scratch instead of
building a replay list. If a scaled offset or dimension is not finite, the call
returns `GuiDrawError::TextLayout(TextLayoutError::GeometryNotFinite)` without drawing, even
for a blank or fully transparent label.

## Shapes a rectangle can't make

Call `canvas.triangles` with your own corners for rounded panels, discs and rings, line icons
or arrows. Each `GuiVertex` is a point in the scope's local pixels and an ARGB colour, blended
across its triangle. The triangles take the scope's pose and scissor like the other draws.
A disc, as a fan of triangles about its centre:

```rust,ignore
use mod_api::GuiVertex;
use std::f32::consts::TAU;

const SEGMENTS: usize = 32;
let at = |i: usize| {
    let angle = TAU * i as f32 / SEGMENTS as f32;
    GuiVertex { pos: [cx + r * angle.cos(), cy + r * angle.sin()], argb: 0xFF2E_7D32 }
};
let centre = GuiVertex { pos: [cx, cy], argb: 0xFF2E_7D32 };
let fan: Vec<[GuiVertex; 3]> = (0..SEGMENTS).map(|i| [centre, at(i), at(i + 1)]).collect();
draw.draw_gui(&mut |canvas| canvas.triangles(&fan))?;
```

Build the corners once and keep them while the shape doesn't change. A corner that isn't
finite, before or after the pose, refuses the whole call with `GuiDrawError::NotFinite`.

## Items

Call `canvas.item(&stack, at, seed)` to draw a stack's model in a 16-pixel slot, passing the seed to

Use `canvas.item_tinted` to multiply the item and its glint by an ARGB colour tint.
its model resolver. Call `canvas.item_decorations(&stack, at, None)` to add its bar, cooldown,
count and custom decoration, in that order. An optional `Some(label)` replaces the count text.
An `item_bar` override supplies a signed width in pixels. As in vanilla, the bar's foreground
endpoint is formed with wrapping integer addition.
The stack is read for display, not written to a slot: it keeps its components and can be over the
storage limit. Empty stacks draw nothing. Head-skin fetches and atlas requests caused by the draw
wait until the outer group commits.

To draw from the local inventory, read it at the calling boundary rather than sending a slot
number to the renderer. Inventory numbers `0..=8` are the hotbar and `40` is the off hand. An
absent stack means nothing to draw. Here the model and decorations both follow the panel's
translation, but only the model gets the pop squeeze, as in vanilla's hotbar:

```rust,ignore
use mod_api::client::hud_layout::PoseOp;

let stack = host.local_player()
    .and_then(|player| player.with_inventory(host, |inv| inv.get(0)?.project()))
    .flatten();
if let Some(stack) = stack {
    draw.draw_gui(&mut |canvas| {
        canvas.scope(&[PoseOp::Translate(20.0, 30.0)], None, &mut |panel| {
            let at = [0.0, 0.0];
            panel.scope(
                &[
                    PoseOp::Translate(8.0, 12.0),
                    PoseOp::Scale(0.5, 1.5),
                    PoseOp::Translate(-8.0, -12.0),
                ],
                None,
                &mut |model| model.item(&stack, at, 1),
            )?;
            panel.item_decorations(&stack, at, None)
        })
    })?;
}
```

`ItemPart::item_decorations` receives a `GuiCanvas` as `event.draw` after the standard
bar, cooldown and count. Use the icon's local `event.x` and `event.y` for drawing. The canvas
already has the caller's pose and scissor (identity for ordinary inventory, hotbar and tooltip
calls). On this canvas and its child scopes, `item_decorations` paints only the standard bar,
cooldown and count; it does not invoke any item decorators, even for a different stack. A
decorator returns `()`, so use `event.draw.scope` to roll back its own primitives together on
failure. Move any widgets, focus, narration or live scenes to ordinary
HUD or screen draw hooks.

For replacing or wrapping a HUD status layer, see [HUD drawing](hud-drawing.md).

## Blits

- `canvas.blit_sprite` looks up a GUI sprite by id, such as `container/slot` or `mymod:panel`.
  Pass `None` to draw the whole sprite or `Some(GuiSpriteRegion { size, offset })` to give the
  whole sprite's logical size and the source offset in its pixels. Only `Textured` and `Crosshair`
  work on this path. A removed GUI sprite is refused, not drawn as a missing texture.
- `canvas.atlas_sprite` looks up the exact packed atlas name, independently of the GUI lookup.
- `canvas.blit` draws part of a texture packed in the GUI atlas (a path under `textures/`,
  without `.png`, or a registered atlas's sprite such as `mymod:gui/atlases/icons/mymod/heart`).
  A live dynamic texture or a standalone resource-pack texture is refused with `UnknownTexture`:
  draw those with `canvas.texture`. `GuiBlitRegion { uv, src, tex }` gives the source offset, signed source
  extents and texture-axis divisors in pixels. A negative source extent mirrors that axis; a zero
  divisor is refused. `Textured` blends normally, `Vignette` darkens what is under
  it (`dst * (1 - src)`), `Nausea` is additive and `Crosshair` inverts it.
- `canvas.texture` draws part of a standalone texture by its exact resource `Identifier`
  (`mymod:textures/book/chapter.png`), any image of the resource stack outside the GUI atlas, with the
  same `GuiBlitRegion`. A missing image draws the missing texture instead of refusing
  ([Standalone textures](hud-drawing.md#standalone-textures)).

## Refusals

`GuiDrawError` belongs to the call that returned it. Geometry or pose arithmetic that isn't finite
returns `NotFinite`; text and stack conversion failures carry `Text` or `Stack` errors. The blit
methods distinguish `UnknownSprite`, `UnknownTexture`, `BadSpriteRegion`, `BadBlitRegion` and
`UnsupportedSpriteBlend`. Return a refusal from the callback to discard its whole group, or
handle it there and keep the successful draws.

## Turning what a GUI canvas draws

Pass `PoseOp::Rotate(angle)` to [the canvas's scope](#drawing-directly-with-a-canvas)
to turn by `angle` radians, as vanilla's `Matrix3x2fStack.rotate` does. Combine it with
`PoseOp::Translate(x, y)` and `PoseOp::Scale(x, y)` to draw a compass needle, a minimap arrow,
or a tilted label.

```rust,ignore
use mod_api::client::hud_layout::PoseOp;

// A needle 20 pixels long turned by `heading` about (cx, cy).
draw.draw_gui(&mut |canvas| {
    canvas.scope(
        &[PoseOp::Translate(cx, cy), PoseOp::Rotate(heading)],
        None,
        &mut |needle| needle.fill([-1.0, -20.0, 2.0, 20.0], 0xFFFF_0000),
    )
})?;
```

- **Order and direction.** Operations apply in order. `Translate` then `Rotate` turns about the
  translated point; `Rotate` then `Translate` moves along the turned axes. GUI y grows downwards,
  so a positive angle turns clockwise on screen.
- **Scope.** The pose applies to everything drawn through the child canvas, including its nested
  scopes. Returning to the parent restores the earlier pose and scissor. If this scope supplies
  a scissor, it transforms the rectangle's two ordered corners at entry, not the bounding box
  of a rotated rectangle. That screen-space clip does not turn with a later child.
- **Exactness.** The matrix is JOML's, float for float: a quarter turn of `FRAC_PI_2` leaves the
  same tiny non-zero cosine vanilla's does.
- **Refusal.** A NaN or infinite angle returns `GuiDrawError::NotFinite` before the callback
  runs. Handle that error to keep drawing in the parent, or propagate it to discard the group.
