# Drawing on the HUD and in screens

A HUD layer, a screen or a tooltip hook draws through the `DrawContext` it is handed, in GUI pixels. Each call
is one of vanilla's `GuiGraphics` calls: `fill`, `text`, `sprite`, `item_stack` and the others.
[Draw through a canvas](gui-paint.md#drawing-directly-with-a-canvas) with `draw_gui` to group
fills, styled text, blits and seeded items with their decorations in lexical pose and scissor scopes.
See [status rows and fixed text positions](#status-rows-and-fixed-text-positions) to draw at the
original layer or reserve space for overlays.

`ClientHost::debug_overlay_visible()` reports whether the client draws its debug overlay this frame. In
`render_gui_post`, use it to leave space for the overlay, and use `ClientHost::hud_layer_bounds(layer)`
to read the `[x, y, width, height]` rectangle in GUI pixels of a layer drawn earlier in the current
HUD pass, clipped to the viewport and its scissors. For example, use `"minecraft:effects"` to place
an overlay below the visible effect icons, with each overlay's offset managed by the mod itself.
`Ok(None)` means the layer did not paint visible geometry or the HUD pass is not running; screen hooks
never see the earlier HUD's bounds. Always-on debug entries can make the debug overlay visible with
F3 off. F1 hides the debug overlay unless a screen is open. A panicking `render_gui_post` listener
leaves no drawing in the batch.

## Registering a HUD layer

In `Hooks::register_gui_layers`, call `event.layers.register(id, placement, Box::new(draw))` on the
`GuiLayerRegistrar`. Registration runs once at client startup.
Use any unused namespaced ID and a `GuiLayerPlacement`: below or above all layers, or immediately
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
wrapper omitted the previous body. If a retained painter or wrapper panics, any open clips from
that draw scope close and completed primitives stay in the batch without rolling back or replaying
earlier bodies. The shared error log and toast name the mod, only that callback is disabled for the
session, and later layers still draw. A disabled wrapper leaves the previous body to draw directly.
Callback failure toasts remain visible under F1, except for audio stream notices. Draw errors such
as malformed `gui_paint` buffers are returned to the caller without disabling the callback or
changing previously submitted paint. If Pre cancels the slot, neither the installed body nor Post
runs.

`render_gui_post` runs after the complete HUD walk, including under F1, and before screens, toasts,
debug overlays and IME. It does not run on the title, during replay, or after a canceled
`render_gui_pre`. Use `host.window_focused()` to check focus, and `Hooks::screen_render_post` to
paint above an open screen. Focused chat paints separately in its screen. The original subtitle layer
can defer paint to a screen background without running its hooks again. Cancel that layer to suppress
scheduled paint.

### Editing the hotbar attack indicator

Use `hotbar_attack_indicator_progress` to edit `event.progress` without replacing
`minecraft:hotbar`. The hook receives a read-only `ClientHost`. It runs inside the original
hotbar body after the items, and starts from the local player's attack charge at partial tick
zero, even when the camera follows a different player. It runs even at full charge, so setting
progress below `1.0` can make the indicator visible. Setting it to `1.0` hides the indicator.

Listeners see the last accepted progress. A non-finite value or one outside `0.0..=1.0` is
refused without erasing earlier accepted edits or disabling the listener. The change applies
only to this draw: it does not change the crosshair indicator, attack charge, or combat.

The hook runs only in Hotbar indicator mode with the HUD visible and a player camera outside
spectator mode. It does not run if Pre cancels the hotbar, a replacement takes over, or a
wrapper skips the original body. A wrapper that delegates to the original body reaches it.

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
A `Flow::Cancel` from `render_gui_pre` skips the whole walk, including Post, even under F1.
It does not suppress toasts, which draw after the active screen or overlay. F1 hides ordinary
toasts and audio stream failure notices, but not other callback error toasts.
To replace all rows formerly drawn by `health_hud`, replace or wrap `player_health`,
`armor_level`, `food_level` and `air_level`; `vehicle_health` is separate.
Use [`draw.draw_gui`](gui-paint.md#drawing-directly-with-a-canvas) in the installed body to group
fallible draws with a scoped pose or scissor.
Sunrise's radial overlay keeps clear of the actual paint from installed status-layer bodies,
including replacements and wrappers.

To sit on a row, read how far the status rows reach: `host.gui_status_heights()` gives the left
and right columns' heights above the bottom edge (NeoForge's `Hud.leftHeight` and `rightHeight`),
as they stand when your layer starts. Both are 39 before the hearts; each vanilla row adds its
height as it draws, so a layer `Below("minecraft:food_level".into())` sees the height the food row
draws at and one `Above` it the height after it. A row that is hidden, cancelled or replaced adds
nothing, and a replacement does not reserve space for the rows after it. Outside the HUD layers
the read is `None`.

To make room for a row of your own, raise a column with
`host.set_gui_status_heights([left, right])` (assigning NeoForge's `Hud.leftHeight` and
`rightHeight`). The vanilla rows that draw after your layer start from the new heights and add
their own on top: a layer `Above("minecraft:armor_level".into())` that draws a row and raises the
left column by its height moves the rows drawn after it. A wrapper that sets the heights before
`ctx.draw_previous()` moves the row it wraps as well. Rows already drawn stay where they are, both
columns go back to 39 at the start of each HUD pass, and the call returns false, setting nothing,
outside the HUD layers.

Adding a taller status bar does not move the selected item name or the action bar. The name starts
59 GUI pixels from the bottom (45 outside survival and adventure); the action bar is centred 68
pixels from the bottom. A mod that needs to move either can cancel and redraw its layer:
`minecraft:selected_item_name` or `minecraft:overlay_message`.

## Showing an action-bar line

A client mod puts a line over the hotbar with `host.set_overlay_message(line, animate_color)`, as
`Hud.setOverlayMessage` does in Java. No player handle or connection is needed: it is this window's
HUD. Each call replaces the current line (empty text too) and restarts the stock timer and fade;
`animate_color` cycles the colour like a jukebox's "Now Playing" line. The game's own lines use the
same slot, so a server's action-bar packet or a jukebox message replaces a mod's line, and the other
way round. Nothing is added to chat, narrated or sent. Text the boundary refuses returns
`Err(TextFormError)` and the previous line stays. A server mod sends a player's action bar with
`player_action_bar` instead.

```rust
{{#include ../../../examples/src/client_rendering.rs:tool_mode_changed}}
```

## Registering F3 text

Push a `DebugEntry::text(id, group, lines)` in `Hooks::register_debug_entries` at client
startup. Use any valid `<namespace>:<path>` ID (including custom `minecraft:<path>` IDs that do
not collide with built-in vanilla entries); it identifies the entry in the debug options and
saved profile. The group says which vanilla lines yours join, or `DebugGroup::Named(id)` puts
them in a group of your own that every entry naming the same ID shares (`minecraft:memory` is
vanilla's memory group). The closure receives the current client host and appends to the
entry's lines.

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

In `Hooks::register_debug_renderers` (hook version 4), push `DebugRenderer::new(draw)` for each
renderer that should run until the next refresh. Callbacks run after `submit_custom_geometry` and
share its deferred geometry and submission rules. To gate a renderer on a debug options switch, register
`DebugEntry::renderer(id)` at startup in `Hooks::register_debug_entries` (or reference a
built-in vanilla entry such as `"minecraft:chunk_borders"`) and check
`event.is_debug_entry_enabled(id)` before pushing. Unconditional renderers require no switch
entry. Use the callback's `ClientHost`, `WorldRenderView<'_>` (lending the frame's `Camera`,
world `partial_tick`, `game_time`, `improved_transparency` state, and visible
`RenderableSection` slice), and `WorldDrawContext` to draw, and capture any per-refresh state
specific to this renderer.

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
`submit_custom_geometry`, sharing its deferred `WorldDrawContext` (which grows dynamically and
draws at the end of the world pass). A panic disables only that callback for the rest of the
session, including after a selection refresh.

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
fn render_gui_post(&self, _: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
    // A panel shaded from dark purple at the top to black at the bottom.
    event.draw.fill_gradient([20.0, 20.0, 120.0, 60.0], 0xF010_0010, 0xF000_0000);
    // A shade that fades out downwards.
    event.draw.fill_gradient([20.0, 80.0, 120.0, 10.0], 0x8000_0000, 0x0000_0000);
}
```

`GuiCanvas::fill_gradient` takes the same arguments and follows the canvas's current pose and scissor.

- **Clip.** A clip from `DrawContext::with_scissor` or `GuiCanvas::scope` keeps the colour ramp:
  the cut edge has the colour that was there.
- **Empty or inverted.** Zero width or height draws nothing, as does inversion on exactly one axis.
  Inverting both axes (`w < 0` and `h < 0`) draws sorted bounds with the colours flipped: the first
  colour stays on its input y edge, now the lower edge. This preserves vanilla's visible winding
  and colour placement. A non-finite rectangle draws nothing; the canvas reports `NotFinite`.
  Zero-area calls are culled here; vanilla can retain their layering bounds without drawing pixels.
- **Cost.** One quad on the GUI batch, as `fill`; nothing is drawn or kept when a mod doesn't call it.

## Player faces

`GuiCanvas::player_face(uuid, at, size, hat, flip, argb)` inside `DrawContext::draw_gui` draws a player's skin
head as vanilla's `PlayerFaceExtractor` does, the head the player list, a team screen or a waystone's owner
line shows: the face, then the hat layer when `hat`, each `size` GUI pixels square at `at`, tinted by the ARGB
`argb` (`0xFFFF_FFFF` for none).

```rust,ignore
use mod_api::{DrawContext, GuiDrawError};

fn draw_member(draw: &mut dyn DrawContext, member: u128, x: f32, y: f32) -> Result<(), GuiDrawError> {
    draw.draw_gui(&mut |canvas| {
        canvas.player_face(member, [x, y], 8.0, true, false, 0xFFFF_FFFF)
    })
}
```

- **Which skin.** A player with an entry on the connection's player list (`PlayerInfo`) draws the skin the
  player list draws. Any other UUID, an offline team member say, draws its profile's skin, looked up by id the
  way a player head's profile is: the lookup starts in the background the first time and is kept as heads keep
  theirs. Until the skin is downloaded and in the atlas, and when the profile has none, the face is the UUID's
  default skin (`DefaultPlayerSkin.get`). The call never waits, so draw it every frame and the real skin appears
  when it is ready; new skin requests wait for the outer `draw_gui` group to commit.
- **Upside down.** `flip` draws the face upside down. The call never guesses it from a name: vanilla's player
  list flips Dinnerbone and Grumm only while their player is in the level, so set it yourself if you want that.
- **Pose and clip.** The face follows the canvas's pose and scissor (`GuiCanvas::scope` and
  `DrawContext::with_scissor`), as `blit` does; the hat layer is drawn under the same pose, scissor and tint as
  the face.

## Standalone textures

`GuiCanvas::texture(id, dst, region, argb)` inside `DrawContext::draw_gui` is vanilla's
`GuiGraphicsExtractor.blit` with a texture's `Identifier`: a picture that isn't in the GUI atlas, such as a
guide book's chapter art. Unlike `blit`, which takes a GUI-atlas name and a `Blend`, it takes a resource
`Identifier` and no blend: the quad at `dst = [x, y, w, h]` in GUI pixels, showing `region.src` texels from
`region.uv` of an image `region.tex` texels big, tinted by the ARGB `argb`.

```rust,ignore
use mod_api::{DrawContext, GuiBlitRegion, GuiDrawError, Identifier};

fn draw_chapter(draw: &mut dyn DrawContext, art: &Identifier) -> Result<(), GuiDrawError> {
    // `art` is `mymod:textures/book/chapter.png`, parsed once at load.
    draw.draw_gui(&mut |canvas| {
        let region = GuiBlitRegion::new([0.0, 0.0], [256, 128], [256, 128]);
        canvas.texture(art, [20.0, 20.0, 128.0, 64.0], region, 0xFFFF_FFFF)
    })
}
```

- **Which file.** `id` is the exact resource, `assets/<namespace>/<path>`, from the active resource stack:
  any namespace, a mod's own included, and a resource pack can replace it. Nothing is added to it, so name the
  `textures/` folder and the `.png` yourself. Only packed resources are read: not files, URLs or dynamic
  textures.
- **When it loads.** The image is decoded and uploaded the first time it is drawn and kept until the next resource
  reload, which loads the new one, even while no screen draws it. Drawing it every frame costs one quad.
- **Coordinates.** The quad samples from `uv / tex` to `(uv + src) / tex`, as `innerBlit` does, without
  clamping: past 1 the image repeats (or clamps, below), and a negative `src` or `tex` flips that axis.
- **Sampler.** As the image's `.png.mcmeta` says, from the image's pack or a pack above it: `"blur": true`
  filters linearly, `"clamp": true` clamps to the edge; otherwise nearest and repeat. No mipmaps.
- **Missing images.** An id that names no image in the stack and no live dynamic texture when you call is refused
  (below). One that is there but doesn't decode, or one whose `.png.mcmeta` isn't valid JSON or
  has a `texture` section vanilla's codec refuses, draws vanilla's magenta and black missing texture in its place, in
  order with the other draws. The call isn't refused for it.
- **Refused.** A NaN or infinite `uv` returns `GuiDrawError::NotFinite`, a `tex` with a zero side
  `GuiDrawError::BadBlitRegion`, and an id that names nothing `GuiDrawError::UnknownTexture`; the call draws nothing.
- **Pose and clip.** The quad follows the canvas's pose and scissor (`GuiCanvas::scope` and
  `DrawContext::with_scissor`), as `blit` does.

## Scaled text

`DrawContext::text_scaled(text, at, scale, argb, shadow)` draws `text` as `text` does, scaled by `scale` about
`at`, its top-left corner. A half-size line under a label uses `scale` 0.5, starting a quarter of its
unscaled width left of where it is centred. This is plain text; for a styled `Text`, draw with
`GuiCanvas::text` inside a scope that translates to `at`, scales, and translates back.

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
