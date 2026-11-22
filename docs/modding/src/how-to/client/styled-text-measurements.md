# Measuring styled text outside drawing

Measure styled text without a draw context. Use `ClientHost::text_width(&Text)` and `text_split(&Text, width)`:
these are reads of the client's active font in unscaled GUI pixels, also usable
in client setup.

| Method on `ClientHost` | Result |
| --- | --- |
| `text_width(&Text)` | `i32`: styled width in GUI pixels. |
| `text_split(&Text, i32)` | `Vec<Text>`: wrapped, styled lines. |
| `text_visual_order(&Text)` | `Vec<VisualTextCharacter>`: styled characters in display order. |

Inside a drawing hook, `DrawContext::text_lines(&Text, max_width, TextAlign, scale)` wraps and
places the same way and adds what a label needs: each line's `Text`, its `at` position inside the
width and its width, plus the block's `width` and `height` — all answered in scaled GUI pixels, the
text wrapping at `max_width / scale`. It refuses invalid widths and scales and oversized layouts
with a `TextLinesRefusal`. See [Screens](screens.md#labels-wrapped-text-without-a-widget).

Build an owned `Text` directly with `Text::literal`, `Text::translate` or
`Text::new(TextContent::…)`. Use `with_style` to set the style and `append` to
add an ordered child, then pass `&text` to the host read. In-process text reads
(`text_string`, `text_width`, `text_split`, `text_visual_order`, and
`BlockEntityDrawContext::text_width`) are infallible and do not enforce the
512-node serialization depth cap or `ExtraCodecs` profile/property bounds;
those checks apply at serialization boundaries (Wire, JSON/SNBT codecs such as
`text_from_json`, and NBT/network writes). There is no intermediate form to
build or validate. `DrawContext::gui_tooltip` and `BlockEntityDrawContext::text` also
accept deep local text, but still return `TextFormError` without drawing if a
component cannot be represented (for example, non-finite metadata).

To inspect the immediate nested components, use `text.visit_children(|child| …)`.
It lends appended children first, then any translation arguments, selector or NBT
separator, or inline-object fallback, and finally the hover text or hover entity
name. It does not recurse into those components. Read `children`, `content` or
`style.hover` directly if you need to distinguish their roles.

Width is the same styled and translated width used by the engine's tooltip
layout. Wrapping follows `Font.split(FormattedText, width)`: it keeps styles and inline objects,
breaks at newlines and fitting spaces, and uses the vanilla one-pixel minimum
for widths below one. Returned lines are API-owned `Text` values. Translation
uses the current client language; a later language or resource-pack font change
is reflected by the next call, so do not retain dimensions across reloads.

NeoForge's `ClientTooltipComponent` receives a `Font` in its width and drawing
methods. These reads use that same role—the active client font—without passing
an engine font object or requiring a `DrawContext`. They do not draw anything
and do not expose a full rendering context.

Chat and signs can [substitute inline images](dynamic-textures.md#inline-images-in-chat-and-signs) before their own
layout. The reads above measure the component you pass, without running that presentation hook.

## Resolved strings

For vanilla `Component.getString()` behavior, use
`host.text_string(&Text) -> String` on either side,
including setup and pack discovery. It uses the same in-process component resolution as
measurement: translation arguments, keybinds and siblings resolve on each call.
Styles do not add characters; literal legacy `§` formatting codes remain in the
string.

- **Client and integrated server:** both use the local player's selected language,
  active resource packs (including client mod translations) and key bindings.
  An integrated server does not resolve against a LAN guest's language.
- **Dedicated server:** starts with built-in vanilla and Sunrise English; world
  and data-load listeners still see those built-ins. Once the world has loaded,
  just before `server_starting`, it loads `assets/<namespace>/lang/en_us.json` from
  enabled mods and the data packs selected for this world. Sunrise visits
  namespaces in lexical order and, within each namespace, reads the original
  files in pack priority order (mods first). Later values win. An invalid
  entry leaves the earlier values intact and stops the remaining files in
  that namespace. Vanilla's deprecated keys are applied at the end. Neither
  client resource packs nor later data-pack reloads affect the snapshot.
  Keybind components give their literal IDs.

During pack discovery and initial reload registration, before language
resources have loaded, `text_string` still resolves literals, fallbacks and
translation keys directly without language overrides (`text_from_json` and
`text_from_legacy` still return `TextFormError::Unavailable` there).

A missing key uses its component fallback, or the key itself without a
fallback. On a dedicated server, a pack language file may also supply an
object or array representing a component: it replaces the whole translation,
including its arguments and fallback. A recursive reference to the same
language entry resolves to no text.

Send an unresolved component to a player when the result should use that player's
language. Do not cache resolved strings across language changes.

## Visual order

For vanilla `Component.getVisualOrderText()` behavior, use
`ClientHost::text_visual_order(&Text) -> Vec<VisualTextCharacter>`.
Each API-owned Wire record contains a Unicode scalar (`scalar`), its full
resolved `TextStyle`, and the sink's `source_index`. Characters are already in
visual order, including Arabic shaping and mirrored punctuation; do not apply
bidi a second time. This read uses the engine's existing language and bidi
pipeline, is available outside drawing, including client setup.

Sink indices are UTF-16 offsets local to a constant-style substring within a
directional run, not global component positions. For example, `A😀B` yields
indices `0, 1, 3`; Hebrew `שלום` yields `3, 2, 1, 0`. A style or bidi run boundary
restarts the index, but adjacent components with equal effective styles stay
in the same run. For example, custom click IDs `example` and `minecraft:example`
match for run grouping, but each character retains its original click event.
A resolved color is returned as uppercase `#RRGGBB`, even
if the input used a named color. Shaping can shorten the string, so these
indices are not guaranteed to identify original input scalars. Use the records to re-layout existing tooltip
lines without losing click, hover, font or other style information.

Action keybind IDs such as `key.jump` resolve through the client's current key
binding to their assigned physical key display name, matching vanilla's `KeybindContents`.
Physical key names resolve normally.
