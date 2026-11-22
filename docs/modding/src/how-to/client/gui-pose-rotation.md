# Turning what a GUI paint draws

A `gui_paint` buffer moves what it draws with a pose: `GuiDraw::Push(ops)` applies `ops` to a copy of the
current pose (`pose().pushMatrix()`, then the ops) and `GuiDraw::Pop` brings the one before back. Besides
`PoseOp::Translate(x, y)` and `PoseOp::Scale(x, y)`, `PoseOp::Rotate(angle)` turns it by `angle` radians, as
vanilla's `Matrix3x2fStack.rotate` does: for a compass needle, a minimap arrow, a tilted label.

```rust,ignore
use mod_api::client::hud_layout::{GuiDraw, PoseOp};

// A needle 20 pixels long turned by `heading` about (cx, cy).
draw.gui_paint(&[
    GuiDraw::Push(vec![PoseOp::Translate(cx, cy), PoseOp::Rotate(heading)]),
    GuiDraw::Fill { x0: -1, y0: -20, x1: 1, y1: 0, color: 0xFFFF_0000 },
    GuiDraw::Pop,
])?;
```

- **About the origin.** The turn is about the current pose's origin, so translate to the pivot first, as above.
  The ops of a `Push` apply in order, each after the ones before it, as vanilla composes them:
  `Translate` then `Rotate` turns about the translated point; `Rotate` then `Translate` moves along the turned axes.
- **Direction.** GUI y grows downwards, so a positive angle turns clockwise on screen.
- **Scope.** A `Pop` undoes the turn with the rest of its `Push`. Pushes nest.
- **What follows it.** Everything the buffer draws after the `Push`: fills, sprites, blits and text, with no clip
  of its own.
- **Exactness.** The matrix is JOML's, float for float: a quarter turn of `FRAC_PI_2` leaves the same tiny
  non-zero cosine vanilla's does.
- **Refusal.** A NaN or infinite angle is refused: the pose stays as it was, the rest of the buffer still draws,
  and the log names it once with `E_GUI_POSE_ROTATE_NOT_FINITE`.
