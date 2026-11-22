# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others. `gui_paint`
takes a buffer of `GuiDraw` ops for the same calls, with `Push` / `Pop` for the pose.

`ClientHost::debug_overlay_visible()` reports whether the client draws its debug overlay this frame. In
`render_gui_post`, use it to leave space for the overlay. Always-on debug entries can make it visible with
F3 off. F1 hides the overlay unless a screen is open. The read is available only on the client host.
An unfinished `render_gui_post` or `gui_render_post` callback leaves no drawing in the batch;
its corner reservations do not affect the next listener.

## Registering a HUD layer

In `Hooks::register_gui_layers`, call `event.layers.register(id, placement, Box::new(draw))` on the
`GuiLayerRegistrar`. Registration runs once at client startup.
Use your own namespaced ID and a `GuiLayerPlacement`: below or above all layers, or immediately
below or above a vanilla anchor or a layer already registered. The client logs and refuses
invalid IDs, duplicate IDs and unknown anchors; `register` returns whether it accepted the layer
into the callback's pending order. Later calls in the same callback can anchor to that layer.
A [failed registration callback](../../concepts/load-phases.md#registration-and-setup)
publishes none of its pending insertions, replacements or wrappers. Vanilla and earlier mods'
layers remain in place for the next callback. Only a panic disables the registering callback.

The client keeps the callback for the session and calls it as the mod that installed it. Capture
any state the layer needs in the closure, and use the current `ClientHost` and `DrawContext`
passed on each draw. Inserted layers, replacements, wrappers and their layer hooks still dispatch under F1. `gui_layer_pre` can cancel any slot that dispatches.
A vanilla anchor still determines the layer's position if its body doesn't draw that frame.

### Replacing or wrapping a layer

In the same registration hook, call `event.layers.replace_layer(id, Box::new(draw))` to discard
an existing layer's body, including any wrappers. Use `event.layers.wrap_layer(id, Box::new(wrap))`
to keep the previous body and draw around it. Both methods accept a vanilla ID or an already
registered mod ID and return `false` if the ID isn't registered. They keep the layer's ID,
order and dispatch. A replacement does not inherit the original body's eligibility checks.
Those checks apply only to the original body, including when a wrapper calls `draw_previous()`
and reaches it.

A replacement drops superseded pending callbacks before returning. A panic there rejects the
current listener's batch and disables its registration callback. Bodies from earlier listeners
retire after publication, in the order this listener first replaced their layers. A retirement
panic is reported against the earlier owner without undoing the replacement or stopping later
listeners. Capture owned startup data from `RegistrationContext` if cleanup needs it; the Host
and drawing context are borrowed only for drawing.

A wrapper gets a `GuiLayerWrapContext`: read the client with `host()`, paint with the scoped
`DrawContext` from `with_draw`, and call `draw_previous()` where the previous body should paint.
The latest wrapper runs first. A delegated mod body draws as its own mod; vanilla paint runs
as the engine, not as the wrapper's mod. A panic in delegated vanilla paint is an engine fault;
catching the unwind inside the wrapper does not let the layer finish or disable the wrapper.
Don't call `draw_previous()` if you want to skip the previous body for this draw without discarding
it at registration.

```rust,ignore
event.layers.wrap_layer("minecraft:hotbar", Box::new(|ctx| {
    ctx.draw_previous();
    ctx.with_draw(&mut |draw| {
        let [w, h] = draw.size();
        draw.fill([w / 2.0 - 91.0, h - 23.0, 182.0, 1.0], 0xFF80_80FF);
    });
}));
```

`gui_layer_pre` and `gui_layer_post` fire once per slot, not per wrapper. Post still fires if the
wrapper omitted the previous body. If a retained painter or wrapper panics or is refused, its
paint is discarded. A wrapper's completed `draw_previous()` calls keep their paint in order,
without being called again. A panic disables just the offending callback for this session and
reports the mod in the error log and the shared error toast. Callback failure toasts remain visible under F1, except for audio stream notices. A refusal is reported without disabling the callback. A disabled wrapper leaves the
previous body to draw directly. If Pre cancels the slot, neither the
installed body nor Post runs. For work that must happen even when the HUD is hidden, use the independent foreground
`gui_render_post` hook.

`render_gui_post` runs after the complete HUD walk and before the open screen. Focused chat
paints separately in its screen. The original subtitle layer can defer paint to a screen
background without running its hooks again. Cancel that layer to suppress scheduled paint.

## Status rows and fixed text positions

`minecraft:player_health` draws the hearts. The neighbouring `minecraft:armor_level`,
`minecraft:food_level`, `minecraft:vehicle_health` and `minecraft:air_level` slots draw the other
status rows. `GuiLayerPlacement::Above("minecraft:player_health".into())` places a layer between
hearts and armor; use `Above("minecraft:air_level".into())` for a layer above all the status rows.

Cancel a slot with `gui_layer_pre`, or install a custom body in `register_gui_layers`:

```rust,ignore
event.layers.replace_layer("minecraft:player_health", Box::new(|host, draw| {
    let Some(player) = host.local_player() else { return };
    let Some(living) = host.living(player.entity()) else { return };
    let Some(health) = living.health(host) else { return };
    let [w, h] = draw.size();
    draw.fill([w / 2.0 - 91.0, h - 39.0, health, 4.0], 0xFFE0_4040);
}));
```

Use `wrap_layer` and call `ctx.draw_previous()` if the vanilla row should still draw. Heart
flashing and `player_heart_type` run only in the vanilla heart body, after the layer's Pre hook.
Bubble bookkeeping and pop audio run only in the vanilla air body, even if the heart layer was
cancelled or replaced. The status slots still run their layer hooks and installed bodies under F1,
in creative or spectator mode, and off a mount. The original bodies check HUD visibility and the
conditions for the row: survival or adventure for the player's rows, a mount with hearts for vehicle
health. To hide your own paint under F1, check `host.hud_mode() == mod_api::HudMode::AllVisible`.
A `Flow::Cancel` from `render_gui_pre` skips the whole walk, including Post, even under F1. To replace all
rows formerly drawn by `health_hud`, replace or wrap `player_health`, `armor_level`, `food_level`
and `air_level`; `vehicle_health` is separate.
You can keep a `GuiDraw` list: pass it to `draw.gui_paint(&draws)` and handle its returned error.
Sunrise's radial overlay keeps clear of the actual paint from installed status-layer bodies,
including replacements and wrappers.

Adding a taller status bar does not move the selected item name or the action bar. The name starts
59 GUI pixels from the bottom (45 outside survival and adventure); the action bar is centred 68
pixels from the bottom. A mod that needs to move either can cancel and redraw its layer:
`minecraft:selected_item_name` or `minecraft:overlay_message`.

## Registering F3 text

Push a `DebugEntry::text(id, group, lines)` in `Hooks::register_debug_entries` at client
startup. Use any valid `<namespace>:<path>` ID (including custom `minecraft:<path>` IDs that do
not collide with built-in vanilla entries); it identifies the entry in the debug options and
saved profile. The group says which vanilla lines yours join; the closure receives the current
client host and appends to the entry's lines.

```rust,ignore
use mod_api::{DebugEntry, DebugGroup, RegisterDebugEntriesEvent, RegistrationContext};

fn register_debug_entries(&self, _: &RegistrationContext, event: &mut RegisterDebugEntriesEvent<'_>) {
    event.entries.push(DebugEntry::text(
        "example:queue".into(),
        DebugGroup::Memory,
        |_host, lines| lines.push("Queue: ready".into()),
    ));
}
```

Each mod starts with an empty batch. The client keeps the closure and calls it as your mod only
while the entry is visible and its mod is enabled. A panic discards its lines and disables that
callback for the session. By default, reduced debug info hides the entry even if it is set to
always on (matching vanilla's `DebugScreenEntry.isAllowed`); call `.with_reduced_debug_info(true)`
on the `DebugEntry` to keep it allowed under reduced debug info.

## Registering world-space debug renderers

In `Hooks::register_debug_renderers`, push `DebugRenderer::new(draw)` for each renderer that
should run until the next refresh. To gate a renderer on a debug options switch, register
`DebugEntry::renderer(id)` at startup in `Hooks::register_debug_entries` (or reference a
built-in vanilla entry such as `"minecraft:chunk_borders"`) and check
`event.is_debug_entry_enabled(id)` before pushing. Unconditional renderers require no switch
entry. Use the callback's `ClientHost`, `WorldRenderView` and `WorldDrawContext` to draw, and
capture any per-refresh state specific to this renderer.

```rust,ignore
use mod_api::{
    DebugEntry, DebugRenderer, RegisterDebugEntriesEvent, RegisterDebugRenderersEvent,
    RegistrationContext, WorldDepth, WorldLine,
};

const GUIDE_LINE: &str = "example:guide_line";

fn register_debug_entries(&self, _: &RegistrationContext, event: &mut RegisterDebugEntriesEvent<'_>) {
    event.entries.push(DebugEntry::renderer(GUIDE_LINE.into()));
}

fn register_debug_renderers(&self, _: &RegistrationContext, event: &mut RegisterDebugRenderersEvent<'_>) {
    if !event.is_debug_entry_enabled(GUIDE_LINE) {
        return;
    }
    let line = WorldLine::new([0.0, 65.0, 0.0], [2.0, 65.0, 0.0], 0xFF00_FF00, 2.0, WorldDepth::Tested);
    event.renderers.push(DebugRenderer::new(move |_host, _view, draw| {
        let _ = draw.lines(std::slice::from_ref(&line));
    }));
}
```

The client collects a fresh list at startup, login, the first world view and the next world view
after a selection rebuild or respawn. Every submitted renderer from an enabled mod is kept in
registration order, with no ID-based filtering or deduplication. Keep per-refresh state in the
new callbacks. Only installed callbacks run in a world view, as their registering mods, after
`submit_custom_geometry` and within its shared draw limits. A panic disables only that
callback for the rest of the session, including after a selection refresh.

## Block overlays

Block, water and fire overlays draw in camera space, after the level and hand and before the GUI. In
`extract_block_screen_effect`, call `event.set_custom_renderer(&MY_RENDERER)` to choose a static
`BlockScreenEffectRenderer` for just that overlay. The last eligible listener that sets a renderer wins.
After cancellation, only `#[listener(receive_cancelled)]` listeners can change the choice; the selected
renderer still submits even if the overlay remains canceled. A failed listener leaves the previous
renderer and cancellation in place for later eligible listeners. To replace an earlier listener's renderer
without adding quads or suppressing vanilla drawing, select a renderer that submits no quads and returns
`false`.

Water and fire are extracted only in first person. Water starts with the player's tick-sampled eye fluid,
which [`camera_submersion`](camera-and-collision-reads.md#the-fluid-the-camera-is-in) can override. The
sample updates through death tick 20, when the local player is removed from the level. Extraction keeps
using that last sample until respawn. The water event provides the default water state at the player's
eye block, even if that block is waterlogged.
`event.partial_tick` is world interpolation; `event.player_partial_tick` is local-player interpolation,
which keeps advancing while the level is frozen.

`submit` receives a read-only `ClientHost`, the effect, state, position, world partial tick, HUD visibility
and a `CameraDrawContext`. Return `false` to prepend your quads to the vanilla overlay, or `true` to skip
just that overlay's vanilla quads. Returning `false` does not restore canceled block or water quads. Fire
is different: canceling with no renderer suppresses vanilla fire, but with a selected renderer that returns
`false`, vanilla fire still follows. A failed submission drops its quads without changing the
vanilla drawing decision. A panic names the mod in the log and client error toast. It disables the
panicking extraction listener or that mod's submission for that overlay kind, respectively, for the
rest of the session; unrelated callbacks still run. Submission runs in block, water, fire order; HUD camera overlays such as the
vignette still draw above them.

```rust,ignore
use mod_api::{
    BlockScreenEffect, BlockScreenEffectRender, BlockScreenEffectRenderer, ClientHost,
    ExtractBlockScreenEffectEvent, Flow,
};

struct WaterTint;
impl BlockScreenEffectRenderer for WaterTint {
    fn submit(&self, _: &dyn ClientHost, render: BlockScreenEffectRender<'_>) -> bool {
        render.draw.quad(
            [[-1.0, -1.0, -0.5], [1.0, -1.0, -0.5], [1.0, 1.0, -0.5], [-1.0, 1.0, -0.5]],
            0x3000_4040,
        );
        false
    }
}

// Inside your impl Hooks:
fn extract_block_screen_effect(
    &self,
    _: &dyn ClientHost,
    event: &mut ExtractBlockScreenEffectEvent<'_>,
) -> Flow {
    if event.effect == BlockScreenEffect::Water {
        event.set_custom_renderer(&WaterTint);
    }
    Flow::Continue
}
```

## Gradient fills

`DrawContext::fill_gradient(rect, top, bottom)` is vanilla's `GuiGraphics.fillGradient`, the call screen
backgrounds and tooltips are drawn with. `rect` is `[x, y, w, h]` in GUI pixels, as `fill` takes it. `top` and
`bottom` are straight-alpha ARGB colours (`0xAARRGGBB`): `top` at `y`, blending to `bottom` at `y + h`.
With positive extents these are the visual top and bottom edges. Alpha blends with the colours, so a gradient can
fade out. The current HUD shader discards interpolated alpha below 0.004, not just exact zero; near-transparent
fragments can therefore differ from vanilla's exact-zero discard.

```rust,ignore
fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // A panel shaded from dark purple at the top to black at the bottom.
    event.batch.fill_gradient([20.0, 20.0, 120.0, 60.0], 0xF010_0010, 0xF000_0000);
    // A shade that fades out downwards.
    event.batch.fill_gradient([20.0, 80.0, 120.0, 10.0], 0x8000_0000, 0x0000_0000);
}
```

In a `gui_paint` buffer the same call is the op `GuiDraw::FillGradient { x0, y0, x1, y1, top, bottom }`, with
vanilla's corner arguments: `top` stays on the `y0` edge and `bottom` on the `y1` edge.

- **Pose and clip.** A gradient follows the current pose (a `gui_paint` buffer's `Push`) and the hook call's
  clip (`push_clip`), as `fill` does. A clip that cuts the rectangle keeps the colour ramp: the cut edge has the
  colour that was there.
- **Empty or inverted.** Zero width or height draws nothing, as does inversion on exactly one axis.
  Inverting both axes (`w < 0` and `h < 0`, or `x1 < x0` and `y1 < y0`) draws sorted bounds with the
  colours flipped: the first colour stays on its input y edge, now the lower edge. This preserves vanilla's
  visible winding and colour placement. A `fill_gradient` rectangle with a NaN or infinite coordinate draws nothing.
  Zero-area calls are culled here; vanilla can retain their layering bounds without drawing pixels.
- **Cost.** One quad on the GUI batch, as `fill`; nothing is drawn or kept when a mod doesn't call it.

## Scaled text

`DrawContext::text_scaled(text, at, scale, argb, shadow)` draws `text` as `text` does, scaled by `scale` about
`at`, its top-left corner: what vanilla's HUD draws with `PoseStack.scale` around a line, without a `gui_paint`
buffer of `Push`, `Text` and `Pop`. A half-size line under a label is `scale` 0.5, starting a quarter of its
unscaled width left of where it is centred.

```rust,ignore
use mod_api::{GuiLayerPlacement, RegisterGuiLayersEvent, RegistrationContext};

fn register_gui_layers(&self, _: &RegistrationContext, event: &mut RegisterGuiLayersEvent<'_>) {
    event.layers.register(
        "example:cps".into(),
        GuiLayerPlacement::Above("minecraft:hotbar".into()),
        Box::new(|_host, draw| {
            let width = draw.text_width("12 CPS");
            // Centred at (40, 30), half size.
            draw.text_scaled("12 CPS", [40.0 - width / 4.0, 30.0], 0.5, 0xFFFF_FFFF, false);
        }),
    );
}
```
