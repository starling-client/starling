# Field of view and name tags

Two client hooks change what the local player sees without drawing anything yourself, and a world draw hook can
add labels of its own.

## Field of view

`compute_fov_modifier` runs on the client once per camera tick when the camera follows a player, local or
spectated. It runs after vanilla's effects (such as sprinting and a drawn bow) and before camera easing
(`Camera.tickFov`). It corresponds to NeoForge's `ComputeFovModifierEvent`.

| `ComputeFovModifierEvent` field | Meaning |
| --- | --- |
| `player` | The camera player's handle: the local player or the spectated one |
| `raw` | `getFieldOfViewModifier`'s product before its final lerp |
| `effect_scale` | The Field of View Effects setting |
| `modifier` | The scale to use, starting as `Mth.lerp(effect_scale, 1, raw)`; write it to change the field of view |

A value that is not finite, or lies outside `0.01..=10`, falls back to the starting `modifier`. The hook does not run when the
camera follows a non-player entity, or while the camera player is scoping with a spyglass in first person
(`AbstractClientPlayer.getFieldOfViewModifier` returns `0.1` before `ComputeFovModifierEvent`). Use
`client_tick_pre` or `client_tick_post` for state updates that must run even then.

`compute_fov` runs after `Camera.calculateFov` and fluid/death adjustment, before projection and nausea spin
(NeoForge's `ViewportEvent.ComputeFov`), letting a listener replace the final world FOV in degrees (finite values
in `1..=179`). There is no separate zoom pass before this hook. Hand/HUD FOV uses its own path.

To compose a zoom feature (like a telescope key), check `host.playing()` and your registered key mapping.
Keep the zoom state in your mod. Divide the modifier by your zoom factor in `compute_fov_modifier` (eased by
vanilla `Camera.tickFov`, where a first-person spyglass already bypasses the hook and returns `0.1`), or
divide the angle in `compute_fov` (immediate). Scale relative look speed across mouse, controller and
keyboard in `player_turn` (or adjust mouse sensitivity and cinematic smoothing in `calculate_player_turn`).
When applying zoom or turn scaling in `compute_fov` or `player_turn`, if you want a local first-person
spyglass to take precedence, check `host.camera_type().is_first_person()`, read `host.local_item_use()` for
the active hand, and inspect that hand's stack with `LivingRef::with_hand_item`.

## The Damage Tilt option

`host.gui_options().damage_tilt_strength` is the player's Damage Tilt accessibility option, 0 (off) to 1
(vanilla's `Options.damageTiltStrength`): the factor vanilla's own hurt tilt takes. Scale a hurt shake
(`render_view_transform`) or a field-of-view punch by it, so a player who turned the tilt off for comfort doesn't get yours
either.

## Name tags

`render_name_tag_can_render` runs when the game decides whether to show the name tag of a player, mob, armor stand, item
frame or display entity, before the tag is drawn. It is NeoForge's `RenderNameTagEvent.CanRender`.

| `RenderNameTagCanRenderEvent` field | Meaning |
| --- | --- |
| `entity_id`, `kind` | The entity and its type id |
| `vanilla_visible` | What vanilla decided |
| `visible` | Set it to show or hide the tag |
| `text` | Set it to `Some(text)` to replace the tag's text |

Distance and the entity's availability still apply. The local player's own tag is considered only when its camera
and settings would show it.

`render_name_tag_do_render` then runs before each line of a shown tag is drawn, once a frame: the score line first
(the below-name score, or a mannequin's description), then the name. It is NeoForge's `RenderNameTagEvent.DoRender`.

| `RenderNameTagDoRenderEvent` field | Meaning |
| --- | --- |
| `entity_id`, `kind` | The entity and its type id |
| `line` | `NameTagLine::Score` or `NameTagLine::Name` |
| `content` | The line's plain text, borrowed for the call |
| `attachment` | Where vanilla draws the text, relative to `draw`'s pose; the text sits 0.5 above it, facing the camera |
| `partial_tick` | The frame's partial tick |
| `draw` | A draw posed at the entity's render origin, not turned, raised one line for the name when there is a score line |
| `cancelled` | Set it to skip vanilla's drawing of this line only |

What `draw` draws is kept and depth tested with the entity. Canceling the score line leaves the name where it was,
one line up, as vanilla does. The camera is `host.camera()`. Nothing is asked when no mod listens.

## Labels in the world

`Hooks::submit_custom_geometry` or a [registered debug renderer
callback](hud-drawing.md#registering-world-space-debug-renderers) can draw a label with `draw.name_tag`,
as vanilla draws a name tag: half a block above the position, facing the camera. Use the
callback's `draw` argument, or `event.draw` from `submit_custom_geometry`.

```rust,ignore
let options = NameTagOptions::new(false, true, 0xF0);
let _ = draw.name_tag([x + 0.5, y + 1.0, z + 0.5], &Text::literal("Furnace"), options);
```

| `NameTagOptions` field | Meaning |
| --- | --- |
| `see_through` | Off: drawn once, faded and translucent, hidden behind walls (a sneaking name). On: opaque in the world and a faded copy over everything, walls included |
| `background` | The black backdrop behind the faded text, at the player's text background opacity |
| `light` | The light at the label: sky light in the high four bits, block light in the low four (`0xF0` full sky, `0x0F` full block) |
| `faded_alpha` | `Some(a)` (`with_faded_alpha(a)`) sets the alpha byte of the faded text, in either copy, in place of the value derived from the text background opacity. An alpha under 26 draws no text |
| `exact_light` | `with_exact_light()`: with `see_through`, the copy in the world is drawn in `light` exactly, without the floor of 2 |

Without `see_through` the label is drawn in exactly `light`. With it, the copy in the world raises the sky and block
levels to at least 2 each (unless `exact_light`), and the copy over everything is unlit. The backdrop's alpha is 255 times the player's text
background opacity (0.25 when the background is for chat only), and the faded text's is 255 times the larger of
(opacity + 0.75) / 2 and 0.5: 0x3F and 0x7F at the defaults. Labels are drawn with the world's name tags, far to near.

A label is 0.025 blocks a font pixel, so it shrinks with distance; scale by distance yourself for a constant size.
Backing storage grows dynamically with no per-view quad cap, and in-memory `Text` components impose no
component-tree depth limit. Labels ignore click events, hover events, and opaque metadata; NBT limits
apply when serializing text, not when drawing labels. A non-finite position (`DrawError::NonFinite`)
draws nothing. The
[hook notes](../../reference/hook-notes.md#custom-world-geometry) give the details.

## The client session

For a HUD that shows session information, read `ClientHost::client_session()`, and the local `PlayerRef`'s
`block_breaking` and `ambient_mood`. Values that may be missing (a session, an external display name) are
`Option`s, and every counter stays on the client. `client_server()` lends `name`, the
redacted `address` and `save_folder` as `Option<&str>`. Copy a field with
`.map(str::to_owned)` only to keep it beyond the host borrow. To compare the applied
language, use `host.client_session().client_language().as_deref() == Some("en_us")`.
Keep the `Arc<str>` if you need that snapshot across a language change. The
[hook notes](../../reference/hook-notes.md#client-session-and-local-player-status) give
the exact availability and timing.
