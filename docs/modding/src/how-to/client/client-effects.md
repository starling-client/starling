# Client effects and tooltips

Use client services to edit tooltips, draw extra camera views and change local sound output. See the [API reference](../../reference/api.md) for the generated signatures.

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

Views draw the player's own level for now. The other levels a server opens for the
client (viewed levels) are meshed only while a view names one, after the player's own
level's work and on one worker at a time, and their meshes stay 5 seconds after no view
names the level. Once views of a viewed level are drawn, such a view draws the sections
meshed so far and nothing where a section is not meshed yet.

## Muffling sounds

`ClientHost::muffle_sounds(amount, lowpass, ticks, exempt)` temporarily muffles the sound output, for example to accompany ringing ears after a nearby blast:

- `amount`, from 0 to 1, is the fraction of every sound's volume taken away.
- `lowpass`, from 0 to 1, dulls the output, from untouched to a 300 Hz low-pass.
- Both effects start at full strength and fade linearly over `ticks` client ticks. The fade waits while the game is paused.
- The sound events in `exempt` keep their direct sound clear, including ones already playing, but their shared reverb tail is still muffled. Use this for the mod's ringing sound. Other sounds already playing, music and interface clicks are muffled too.

A new call replaces the current muffle; `ticks` of 0 ends it immediately. The effect belongs to the client’s sound output. The call refuses an amount or strength outside 0 to 1, more than 64 exempt events, or an exempt event that is not a namespaced sound ID, without changing the output. Server code cannot call this client-only service.

Play the accompanying sound through `ClientHost::play_local_sound` with a `SoundPlay`.

The `SoundRef` it returns controls that play: `state`, `stop`, `fade(host, volume, ticks)`, and the live changes
`move_to(host, position)` and `set_pitch(host, pitch)`. Both changes land at the next audio update on the same
instance, so a looping sound is not restarted: use them to follow a moving machine or a rising whine. `move_to`
takes a position in the client's level (finite, within 30 million blocks) and has no effect on a sound played at the
listener (`SoundPlay::ui`); `set_pitch` takes a pitch at or above 0, as `SoundPlay::pitch`. Each returns `false`,
changing nothing, for a bad value or a sound that is over.

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
`play_local_sound`. A refused value plays nothing (or stops the live sound), as a bad volume does. Inside the mixer, a
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
