# Draw a progress bar and a fuel gauge

Install a callback with `MenuScreen::with_draw` to draw a machine's gauges on the client with a
`DrawContext`, over the panel and slot frames but under the labels and items.
`Hooks::menu_background` adjusts the menu's blur; it isn't a drawing hook.

## Send the values in menu data slots

On the server, [set up the block entity's data slots](../world/machine-menu-data.md)
and update them as the machine runs. This example uses a slot for each value:
elapsed work, total work, fuel remaining, and fuel capacity, in that order.
For a custom menu, also set `"data_slots": 4` in the [menu type definition](../content-mods.md#menu-types).
Without it the server sends zero data slots and neither gauge draws, even if
the block entity has all four values. The open menu sends the values to its
viewers. The client reads them in `frame.data`; some or all can be missing
before the first updates arrive. Do not put changing values in `open.extra`; it
is read once, while constructing the screen.
For a block-backed menu, use values that fit in a signed 16-bit integer: larger
values wrap on the wire. A blockless mod menu's live data supports full `i32`.

## Crop the sprite and the destination

`draw.sprite(name, src, dst, tint)` takes two rectangles, each
`[x, y, width, height]`. `src` is in pixels from the sprite's top-left.
`dst` is in GUI pixels from the window's top-left, so add `frame.origin`
to panel-relative positions. Pass `None` for no tint. An unknown sprite ID
returns `false`.

This example uses the game's furnace sprites. In your registered factory, add the
callback to the screen you build for the machine. If the factory handles other
openers too, choose which screen to build using `open.block` there.

```rust,ignore
use mod_api::MenuScreen;

// Inside the factory, after choosing the machine's layout:
Some(MenuScreen::new(layout).with_draw(|_, frame, draw| {
    let &[work, work_total, fuel, fuel_total, ..] = frame.data else {
        return;
    };
    let fraction = |value: i32, total: i32| {
        if total > 0 {
            value.clamp(0, total) as f32 / total as f32
        } else {
            0.0
        }
    };
    let [x, y] = frame.origin;

    // Arrow: keep the left edge fixed and reveal more to the right.
    let w = (24.0 * fraction(work, work_total)).ceil();
    if w > 0.0 {
        draw.sprite(
            "container/furnace/burn_progress",
            [0.0, 0.0, w, 16.0],
            [x + 79.0, y + 34.0, w, 16.0],
            None,
        );
    }

    // Flame: keep the bottom edge fixed and reveal more upwards.
    let h = (14.0 * fraction(fuel, fuel_total)).ceil();
    if h > 0.0 {
        let top = 14.0 - h;
        draw.sprite(
            "container/furnace/lit_progress",
            [0.0, top, 14.0, h],
            [x + 56.0, y + 36.0 + top, 14.0, h],
            None,
        );
    }
}))
```

Shortening both rectangles reveals part of the image without squeezing it.
For the flame, shift **both** top edges by the height you cut off so the
bottom stays in place. The example skips empty gauges; draw a track or
unlit background first if you want it visible when the value is zero.
