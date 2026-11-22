# Field of view and name tags

Two client hooks change what the local player sees without drawing anything yourself, and a world draw hook can
add labels of its own.

## Field of view

`compute_fov_modifier` runs on the client once per tick while there is a local player, after vanilla's effects
(sprinting, a drawn bow) and before zoom and camera easing. It is NeoForge's `ComputeFovModifierEvent`.

| `ComputeFovModifierEvent` field | Meaning |
| --- | --- |
| `player` | The inputs of the calculation (`FovPlayer`) |
| `base` | The scale before any mod ran |
| `modifier` | The scale to use; write it to change the field of view |

A value that is not finite, or lies outside `0.01..=10`, falls back to `base`. The hook does not affect a spectator
camera attached to a mob.

## Name tags

`render_name_tag` runs when the game decides whether to show the name tag of a player, mob, armor stand or item
frame, before the tag is drawn. It is NeoForge's `RenderNameTagEvent`.

| `RenderNameTagEvent` field | Meaning |
| --- | --- |
| `entity_id`, `kind` | The entity and its type id |
| `vanilla_visible` | What vanilla decided |
| `visible` | Set it to show or hide the tag |
| `text` | Set it to `Some(text)` to replace the tag's text |

Distance and the entity's availability still apply. The local player's own tag is considered only when its camera
and settings would show it.

## Labels in the world

A world draw hook (`render_level_stage` or `submit_custom_geometry`) or a [registered debug renderer
callback](hud-drawing.md#registering-world-space-debug-renderers) can draw a label with `draw.name_tag`,
as vanilla draws a name tag: half a block above the position, facing the camera. Use the
callback's `draw` argument, or `event.draw` from a world draw hook.

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
Its quads share the view's 4,096 with `quads`. A non-finite position, unrepresentable component
metadata, or a label past the quads left is refused by name (`DrawError`) and draws nothing. The
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
