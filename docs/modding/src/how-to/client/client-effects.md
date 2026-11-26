# Client effects and tooltips

Use client services to edit tooltips, read effect blends, draw extra camera views and change
local sound output. See the [API reference](../../reference/api.md) for the generated signatures.

## Client effect interpolation

Pass the render partial tick unchanged to `LivingRef::effect_blend_factor(host, effect, partial)`
for the local player's client effect blend. The read does not clamp it. An unknown effect,
stale handle or non-local entity returns `None`; a known but absent effect returns `Some(0.0)`.

An ordinary blended addition starts at 0 before its first effect tick, then interpolates each
tick to the post-countdown target (1 while active, 0 once expired). An effect that expires on
its first tick never rises; an infinite effect also interpolates on addition. An unblended
addition starts at its target, and replacing an effect keeps its blend. Darkness and nausea
have longer fades.

## Reading the tooltip preference

Read `ClientHost::gui_options().advanced_item_tooltips` for the current
`advancedItemTooltips` preference (F3+H). The snapshot reflects the current client
setting; request a new snapshot after a toggle. This is separate
from the advanced flag supplied to an individual tooltip request.

## Showing an item's tooltip in your screen

Call `ItemResource::tooltip(host, advanced, creative)` for the same item lines the
inventory builds, including item parts, component appenders and `item_tooltip`
edits. Use the GUI option above for `advanced`; set `creative` only for a creative
tab's own slot. The request returns text elements, not images or a render request.
Pass their text to `DrawContext::gui_tooltip` at your GUI cursor position. If a line
has a separate RGBA colour, use its RGB channels as the fallback text style colour;
`u32::MAX` means keep the text's own style.

During item tooltip line construction, the request returns base lines without
running line callbacks again. Outside line construction, including in gather or
pre-render callbacks, the request runs line callbacks. It never gathers images or
draws a tooltip.

## Adding tooltip lines

Use `ItemPart::tooltip` for an attached item's own lines at the vanilla
`Item.appendHoverText` position, after the name and before component lines.
Use `Hooks::item_tooltip` for NeoForge's global `ItemTooltipEvent` moment: it
receives every item's finished tooltip, including the parts' earlier lines,
component lines and advanced information. No attached part is needed for a
global listener, so this is also how a mod edits vanilla or other mods' tooltips.
The global event lends `stack: ItemStackView<'_>` (call `project()` to copy
components), `display: &TooltipDisplay<'_>`, and live `lines: TooltipLines<'_>`
validated per edit with `TextFormError` (one invalid edit leaves all other lines
intact), alongside player IDs and the request's `advanced` and `creative` flags.

## Custom tooltip components and render hooks

Custom tooltip components follow vanilla's `TooltipComponent` / `ClientTooltipComponent`
and NeoForge's `RegisterClientTooltipComponentFactoriesEvent`, `RenderTooltipEvent.GatherComponents`,
`RenderTooltipEvent.Pre`, and `RenderTooltipEvent.Texture`:

1. **Define a data payload and client renderer.** Implement `TooltipComponentData` on your
   payload type and `ClientTooltipComponent` on the client component that measures and draws it:

```rust,ignore
use mod_api::{
    ClientTooltipComponent, DrawContext, Text, TooltipComponentData, TooltipFont,
};

struct ChargeBarData {
    filled: i32,
    max: i32,
}
impl TooltipComponentData for ChargeBarData {}

struct ChargeBarClient {
    filled: i32,
    max: i32,
    label: Text,
}

impl ClientTooltipComponent for ChargeBarClient {
    fn width(&self, font: &dyn TooltipFont) -> i32 {
        font.width(&self.label).unwrap_or(0).max(60)
    }

    fn height(&self, font: &dyn TooltipFont) -> i32 {
        font.line_height() + 6
    }

    fn draw_text(&self, font: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
        let _ = draw.text_with_font(font.id(), &self.label, [x as f32, y as f32], 0xFFAA_AAAA, true);
    }

    fn draw_image(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        width: i32,
        _height: i32,
        draw: &mut dyn DrawContext,
    ) {
        let bar_y = (y + font.line_height() + 1) as f32;
        let fill_w = (width as f32) * (self.filled as f32 / self.max.max(1) as f32);
        draw.fill([x as f32, bar_y, width as f32, 4.0], 0xFF33_3333);
        draw.fill([x as f32, bar_y, fill_w, 4.0], 0xFF44_CCFF);
    }
}
```

2. **Register the client factory once at startup.** Implement
   `Hooks::register_client_tooltip_component_factories` and bind the payload type to its
   `ClientTooltipComponent`. A later registration for the same type replaces the earlier one.
   If the registration listener panics, none of its factories replace or extend the table:

```rust,ignore
fn register_client_tooltip_component_factories(
    &self,
    _cx: &mod_api::RegistrationContext,
    event: &mut mod_api::RegisterClientTooltipComponentFactoriesEvent<'_>,
) {
    event.factories.register::<ChargeBarData, ChargeBarClient>(|data| ChargeBarClient {
        filled: data.filled,
        max: data.max,
        label: Text::literal(format!("{}/{}", data.filled, data.max)),
    });
}
```

3. **Supply the component for an item or globally.**
   - For your own item, implement `ItemPart::tooltip_image` (vanilla's `Item.getTooltipImage`)
     and return `Some(TooltipComponent::custom(ChargeBarData { filled, max }))` (or
     `Some(TooltipComponent::bundle(stack))`). It is inserted right after the item's name line
     before `render_tooltip_gather_components` runs. Return `None` to leave vanilla's component
     unchanged.
   - To add, reorder, replace or remove elements on any item's tooltip, implement
     `Hooks::render_tooltip_gather_components` and push
     `TooltipElement::Component(TooltipComponent::custom(data))` alongside `TooltipElement::Text`.
     Text lines wrap automatically when the tooltip overflows both sides of the screen. Set
     `event.max_width = width` with a positive `width` to tighten wrapping further. Return
     `Flow::Cancel` to skip the tooltip (neither `render_tooltip_pre` nor
     `render_tooltip_texture` runs).

4. **Inspect prepared components, replace the font, move or cancel in `render_tooltip_pre`.**
   After Gather wraps text and prepares every element into a `PreparedTooltipComponent`,
   `Hooks::render_tooltip_pre` runs before measurement and drawing (for both item tooltips and
   non-item tooltips where `event.stack` is `None`):

```rust,ignore
fn render_tooltip_pre(
    &self,
    _host: &dyn mod_api::ClientHost,
    event: &mut mod_api::RenderTooltipPreEvent<'_>,
) -> mod_api::Flow {
    let _first_component_size = event.draw.with_tooltip_font(event.font, |font| {
        let component = event.components.first()?;
        Some([component.width(font)?, component.height(font)?])
    });
    // Replace the font used for measurement and drawing without rewrapping text lines:
    *event.font = mod_api::Identifier::parse("minecraft:alt").unwrap();
    mod_api::Flow::Continue
}
```

   Supply dimensions in your `ClientTooltipComponent`'s `width` and `height`
   implementations. To read a component's dimensions, use
   `component.width(font)` and `component.height(font)` in Pre, as above (`None` means
   measurement failed). Gather sees unprepared data, not dimensions. The `width` and `height`
   passed to `draw_image` are for the entire tooltip's content box, not that component.

   Custom components can be inspected with `component.downcast_ref::<ChargeBarClient>()`,
   and built-in lines and bundles with `as_text()` and `as_bundle()`. Use `event.positioner`
   to compute placement. Item and cursor tooltips use the default cursor positioner, while
   widget tooltips can position relative to the widget. Set `event.place.x` / `event.place.y`
   to move the anchor; a widget positioner may ignore it. The initial cursor anchor is
   truncated to whole GUI pixels. Text lines wrap only when Gather runs: with no Gather
   listener, Pre sees each line whole, as vanilla passes it. A stack that can't cross into
   the API skips Gather and Pre, and the engine's tooltip is drawn as it is. The item font is queried before Gather's wrapping and again before Pre;
   the second answer seeds `event.font` but never changes the lines. Return
   `Flow::Cancel` to suppress the box, component passes and `render_tooltip_texture` (anything
   drawn through `event.draw` stays visible).
   Use `Hooks::render_tooltip_texture` to override the tooltip style sprites (`event.texture`)
   or solid background/border colours.

Every prepared component — including zero-width or zero-height components — participates in
the two-pass draw order (`draw_text` for every component, then `draw_image` for every
component, matching `GuiGraphicsExtractor.tooltip`). An unregistered custom payload aborts
that tooltip before `render_tooltip_pre`, `render_tooltip_texture`, or painting with an error
log (without affecting later tooltips). A failed measurement or component draw aborts that
whole tooltip, including the background and any earlier component paint. Pre listener paint
stays visible. A panic in a factory, component callback, or destructor for retained tooltip data,
renderer, or factory captures disables the owning callback for the session, with a log entry
and error toast naming the mod. The first publisher owns a retained payload: a Gather listener
or an item-image provider. Reordering or cloning someone else's component does not transfer
its ownership or disable a healthy provider when that payload panics on drop.

## Drawing a second world camera

Native internals mods can request extra world cameras through `world_views` and
consume their textures in `world_view_drawn`. These are raw renderer callbacks, not
stable API callbacks. Construct a `WorldView` with a
stable ID, camera, yaw, pitch, field of view and pixel size, and push it to the
`WorldViews` the hook hands you. The hook is asked once a frame. Up to eight
views with distinct IDs are taken, with sides limited to 4096 pixels and the
device limit, and 4 x 4096 x 4096 pixels in all. `push` refuses any other view
with a `WorldViewRefusal` naming it and the reason, and logs each refusal once.
Each view culls the world with its own frustum, and the player's view keeps its
own culling. These callbacks have no Wire form.

For a view seen in another view, such as a portal seen through a portal, push the
outer view first and add `with_parent(outer_id)` to the inner one. Views nest up to
depth 5; depth 6 is refused. The inner view is drawn first, so in the outer view's
`world_view_drawn` its picture is this frame's: draw it into the outer view's
target there. `WorldViewDrawn` carries each view's `parent` and `nesting_depth`.

For a view through a portal, add `with_clip_plane([x, y, z, d])`: nothing behind
that world-space plane is drawn, and the camera must be behind it. In such a view
an entity wins the depth test against terrain up to 1/1024 of its distance in
front of it (about 1 block at 1024 blocks). For a mirror,
give the camera reflected through it and add `with_mirror()`: the picture is
flipped left to right, with face culling flipped to match, so the same faces are
culled as in an unmirrored view. A clip plane with a zero normal, a value that is
not finite, or the camera in front of it refuses the view.

A view draws the player's level unless it names another with
`with_level(Some(level))`, a `LevelRef` from `host.level(dimension)`; `None` is the
player's level, and so is a handle naming it. Viewed levels are not drawn yet: a view
of any other level (one the server keeps open for viewing, a handle from an earlier
epoch, a level closed or never open) is refused as `WorldViewRefusalReason::Level`
and logged once. The handle is resolved once a frame, so a stale one is refused from
the next frame on.

Use `world_view_drawn` to copy a still to your own buffer or draw into the supplied
target with its encoder. Copy a picture you need to preserve: targets are reused
while requested and retired when absent. Views draw before `world_drawn`, where
you can composite their pictures into the player's frame. Extra cameras increase
rendering work and do not include particles, weather, hands or shader-pack passes.
Extra views also omit the sun, moon, stars, clouds and block outline. Translucent
terrain uses the player camera's sorting. See the generated `world_views` hook notes
for the rendering contract.

Views draw only the player's own level. A request targeting another level is refused with
`WorldViewRefusalReason::Level`.

## Muffling sounds

`ClientHost::muffle_sounds(amount, lowpass, ticks, exempt)` temporarily muffles the sound output, for example to accompany ringing ears after a nearby blast:

- `amount`, from 0 to 1, is the fraction of every sound's volume taken away.
- `lowpass`, from 0 to 1, dulls the output, from untouched to a 300 Hz low-pass.
- Both effects start at full strength and fade linearly over `ticks` client ticks. The fade waits while the game is paused.
- The sound events in `exempt` keep their direct sound clear, including ones already playing, but their shared reverb tail is still muffled. Use this for the mod's ringing sound. Other sounds already playing, music and interface clicks are muffled too.

A new call replaces the current muffle; `ticks` of 0 ends it immediately. The effect belongs to the client’s sound
output. The call refuses an amount or strength outside 0 to 1, or an exempt event that is not a namespaced sound ID,
without changing the output. There is no limit on the number of events in `exempt`.

Play the accompanying sound with `ClientHost::play_local_sound(&sound, after)`. The `after` delay is counted from the
next audio update in unpaused sound-engine ticks. The sound engine calculates final volume and pitch after applying the
sound file's own settings. A finite negative base volume can start a silent sound, and a finite negative base pitch
reaches the engine's minimum pitch. `play_sound` uses the same base volume and pitch domain on start and on live refresh;
assign `*event.sound = None` to cancel or stop, not a negative number. A positive repeat gap waits between plays; zero or a
negative gap loops automatically.

The `SoundRef` it returns follows and controls the same instance without restarting it:

- `state(host)` tells you whether it is waiting, playing or stopped; `stop(host)` stops it for good.
- `fade(host, volume, ticks)` changes the volume to a finite target. A target of zero or less stops it when the fade ends.
- `move_to(host, position)` changes its position, in world blocks or as an offset from the listener for relative sounds.
  The position must be finite in the mixer's float coordinates; it is not checked against the world border.
- `set_pitch(host, pitch)` changes its finite base pitch. The engine applies the final pitch clamp.

The writes take effect from the next audio update and return `false` for an unreadable parameter or a sound that is over.

### A sound's own effects

`SoundPlay::effects` (`SoundEffects`) shapes one sound in the mixer, where `muffle_sounds` shapes them all:

- `gain` turns the whole sound down, direct and reverb, without changing how far it carries.
- `direct` turns down only the sound heard directly; `lowpass` and `highpass` filter it (two-pole Butterworth
  filters, 12 dB per octave). A sound behind a wall: a low `lowpass` and some `direct` taken away.
- `room`, `hall` and `cavern` send it to three shared reverbs, and `reverb_lowpass` dulls what is sent (one pole,
  6 dB per octave).

Every field's valid range, inclusive at both ends:

| Field | Range | Off / open |
| --- | --- | --- |
| `gain` | 0 to 1 | 1 |
| `direct` | 0 to 1 | 1 |
| `lowpass` | 20 to 20000 Hz | 20000 (open) |
| `highpass` | 0 to 20000 Hz | 0 (off) |
| `room`, `hall`, `cavern` | 0 to 1 | 0 (no send) |
| `reverb_lowpass` | 20 to 20000 Hz | 20000 (open) |

The cutoffs are in hertz whatever the output device's sample rate, so a value valid on one device is valid on every
device. `SoundEffects::valid` refuses NaN, infinities and any value outside its range; nothing is clamped. The same
check runs on all three paths: a `play_sound` edit when a sound starts, the live refresh of a ticking sound, and
`play_local_sound`. A refused value plays nothing (or stops the live sound). Inside the mixer, a
`lowpass` or `highpass` above the output's Nyquist frequency (half its sample rate; the mixer stops at 0.49 of the
rate) works as that limit: on a 32 kHz output, a `lowpass` of 18000 filters as 15680 would. That limit depends on the
device, not on your value, and is not a refusal.

Start from `SoundEffects::NONE`, which changes nothing, and set the fields you need. Pass them with
`play_local_sound`, or set them on any sound the client plays from a `play_sound` listener: when it starts, and on each
live refresh of a ticking sound (`event.starting` false, once a tick), so a mod can follow a moving source. Every
change glides over one output buffer and never clicks.

A sound that doesn't tick can't change its effects after it starts: it keeps the ones it started with until it ends.
Only a ticking sound gets live refreshes. A NeoForge mod that changes an OpenAL channel at any time has no
counterpart here yet. Per-channel calls may follow as their own issue.

`SoundEffects` is a plain value with no sound id in it. Mod audio streams ([Audio streams](audio-streams.md)) fire no
sound hooks and have their own voices, so effects don't reach them yet.

#### The three reverbs

Each reverb is an eight-line feedback delay network shared by every sound that sends to it. Pick one by its numbers:

| Bus | 60 dB decay | Damping | Line lengths (about) | Sounds like |
| --- | --- | --- | --- | --- |
| `room` | 0.6 s | 7000 Hz | 13.9 to 33.8 ms | a house, a corridor |
| `hall` | 1.8 s | 4500 Hz | 30.0 to 73.2 ms | a large build, a ravine |
| `cavern` | 4.0 s | 3000 Hz | 46.2 to 112.6 ms | a deep cave, a stronghold |

The decay is the time for the tail to fall 60 dB at low frequencies. The damping is a one-pole low-pass inside
the loop, flat at 0 Hz and about 3 dB down at the damping frequency on each pass through a line, so higher frequencies
die sooner, as in a real room. The line lengths set how dense the echoes are. The reverbs' delay lines are allocated
once, when the sound system starts, and about 1.1 MiB is used whatever the device's rate. A reverb nothing is sent to
costs nothing once its tail has died away. Mod-set reverb parameters are not available yet.

## Replacing the weather effects

Each client tick `tick_weather_effects` is asked before vanilla's weather effects run
(`ClientLevel.tickWeatherEffects`): the rain splash particles around the camera, the smoke where rain
falls on lava, magma and lit campfires, and the ambient rain sound. It gets the level, its clock, the
camera's position and the rain level. Return `Flow::Cancel` to skip all of it for that tick, sound timer
included, and draw and play your own weather instead (NeoForge's `CustomWeatherEffectRenderer.tickRain`).
The rain columns drawn each frame are separate.

## Narrating a line

A client mod has the player's narrator say a line with `ClientNarratorClientHost::narrate`,
as Jade's "Narrate Target" reads the block or entity under the crosshair (`GameNarrator.saySystemNow`
in Java). It goes through the game's own narrator, so the Narrator option, the voice volume and
the narrator's log of recent lines apply as to the game's own narration. Nothing reaches the server.

```rust
{{#include ../../../examples/src/client_misc.rs:target_changed}}
```

`Narration::Now` (`saySystemNow`) cuts off what is being said; `Narration::Queued`
(`saySystemQueued`) waits for it. The line is the component's plain string (`Component.getString`)
in the current language. There is no length cap beyond the component's own form limits.

Each refusal says and logs nothing, and has its own name: `Text` (the component was refused at the
boundary), `Empty` (its text is empty), `Off` (the option is Off or Chat only) and `Unavailable`
(no speech on this platform, or no client narrator). Match `NarrateError` non-exhaustively.
