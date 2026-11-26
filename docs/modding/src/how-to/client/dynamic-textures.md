# Textures made at run time

A client mod that bakes its own images (a panel, a gauge, glyphs from its own font, a map, a schematic view) creates
each one under a name and draws it by that name. This is vanilla's `DynamicTexture`: Java registers one with
`TextureManager.register(id, texture)`, sends its pixels with `DynamicTexture.upload()`, draws it with
`GuiGraphics.blit(id, ...)` and frees it with `TextureManager.release(id)`. In Sunrise each one is a GPU texture of its
own, `RGBA8_UNORM` (never sRGB, as vanilla's dynamic textures and block atlas are UNORM), sampled nearest with repeat
by the GUI as `DynamicTexture` does; there is no choice of format or GUI sampler. It has one mip level unless you ask for
more with `mip_levels`. (A mod render pipeline that wants another filter, or the other mip levels, picks them in its own
[sampler binding](render-pipelines.md#samplers-and-mipmapped-textures).)

```rust
{{#include ../../../examples/src/client_rendering.rs:dynamic_panel}}
```

The calls are on the client host. They change the game's textures, so they take `&mut dyn ClientHost`: call them from
a hook that has it, such as `client_tick_post`, not from inside a draw.

- **`create_dynamic_texture(id, width, height, mip_levels, rgba)`** makes the texture, `rgba` being `width * height`
  pixels of four bytes, row by row from the top left: level 0. `mip_levels` is 1 for vanilla's `DynamicTexture`, up
  to the full chain (`32 - max(width, height).leading_zeros()`) otherwise; the other levels start empty. From the next
  frame, `GuiCanvas::texture(&id, dst, region, argb)` draws it inside `DrawContext::draw_gui`: `region.tex` is its
  size, `region.uv` and `region.src` pick the part to draw. The GUI always samples level 0, so a mipmapped texture
  draws there as a one-level one does; a mod pipeline binds it by id (`TextureSource::DynamicTexture`) to read the
  levels.
- **`update_dynamic_texture(id, width, height, rgba)`** gives it new pixels. A different size makes a new texture
  under the same id, as registering a new `DynamicTexture` under an id does, keeping its `mip_levels` (the new size's
  full chain must hold them). Update when the texture changed, not every frame.
- **`update_dynamic_texture_region(id, mip_level, [x, y], [w, h], rgba)`** replaces one region of one mip level and
  keeps the rest, after `CommandEncoder.writeToTexture(texture, source, mipLevel, depthOrLayer, destX, destY, width,
  height)`. The region is in pixels of that level, whose side is the texture's side shifted right by `mip_level` (at
  least 1), and `rgba` must be exactly `w * h * 4` bytes. A region not inside the level is `OutOfBounds`, a level the
  texture does not have is `BadMipLevels`. The levels are filled from the CPU, as vanilla's `MipmapGenerator` does for
  the block atlas; nothing is generated on the GPU.
- **`release_dynamic_texture(id)`** frees it (`TextureManager.release`). A texture never released lasts until its mod
  unloads or the game ends.

Calls are checked at once and applied in the order they were made, before the frame that follows; none is merged. The
game keeps no copy of your pixels: keep your own if you need them again.

## Names and drawing

The id is in a namespace owned by the calling mod (`mymod:...`, its extra manifest namespaces, or a content package it
serves via `served_by`); the path is the mod's to choose. Only the owner can create, update or release it: another
mod's call is `NotOwned`. Drawing has no owner check: any mod may draw any live id. `GuiCanvas::texture` has one lookup
rule for an id: a live dynamic texture of that id, otherwise the resource stack; an id in neither when you call it is
`GuiDrawError::UnknownTexture` and draws nothing. An id released after its draw was
submitted, before the frame draws, shows vanilla's missing texture, as vanilla does after `release`. A dynamic
texture is not a GUI atlas sprite: `DrawContext::sprite`, `gui_nine_slice`, `tiled_sprite` and `gui_sprite` do not
reach it.

## Limits and refusals

The only limits are the device's: a side past the largest 2D texture (`max_texture_dimension_2d`) is `TooLarge`,
and `mip_levels` of 0 or past the full chain is `BadMipLevels`. There is no count or byte cap. Every call answers `Result<_, DynamicTextureError>`, and a refused call changes nothing:

| Refusal | When |
|---|---|
| `NotOwned` | the id is outside the calling mod's namespace, or the texture is another mod's |
| `BadSize` | a side is 0, or the pixel buffer's length is not `width * height * 4` (never a panic) |
| `TooLarge` | a side is longer than the device's largest texture |
| `NameTaken` | `create` of an id that names a dynamic texture |
| `Unknown` | `update`, `update_dynamic_texture_region` or `release` of an id that names none |
| `OutOfBounds` | a region not inside the mip level |
| `BadMipLevels` | `mip_levels` of 0 or past the full chain, a `mip_level` the texture lacks, or a new size whose chain is shorter than the texture's levels |
| `Unavailable` | nothing draws here: a server, or no window |

Match `DynamicTextureError` non-exhaustively.

## Lifetime

Textures survive a resource reload (F3+T, a pack change) and a world closing: there is nothing to upload again. They
are freed by `release_dynamic_texture` or when their mod unloads. If the game loses its graphics device and rebuilds
it, every texture is gone: your next update reads `Unknown`, so create it again.

## Inline images in chat and signs

Implement `Hooks::substitute_inline_images` to display atlas sprites in chat history and on
both sign faces. Each call sees one translated, styled `event.run` with legacy formatting already applied. The hook
runs before wrapping, width measurement and bidi reordering, not for each glyph as it is painted.

```rust
{{#include ../../../examples/src/client_rendering.rs:inline_images}}
```

Each half-open UTF-8 byte range must stay within the run and must not include a newline. The first valid proposal in
hook dispatch order wins over overlapping proposals. A missing sprite or invalid range leaves the text alone unless
another valid proposal covers it. Each replacement occupies one 8×8 `minecraft:gui` glyph. Chat clicks and hovers keep
the span's style; signing, search and copy still use the original message. On a sign, an image uses its
span's colour, or white if the span has none; the sign's dye does not tint it. Sign data and the editor keep
the original text. Name tags and the chat input field do not use this hook.

Give the hook the id of a pack sprite or a live dynamic texture: one lookup rule, the dynamic texture first. A dynamic
texture draws in chat, the HUD's text, screens and both sign faces as its own texture in the glyph's 8×8 box (on a
sign, a quad of its own with the texture bound, lit as the sign's text is, white-tinted unless the span has a colour),
and a texture released after its text was laid out shows vanilla's missing texture until the text is laid out again.
Keep the listener a cheap match: it may run again when text, resources, sprite geometry or the
enabled mods change. For an animation, keep one texture id and update its pixels as frames change; there is no
need to rebuild the text layout.

A trusted native mod downloading images can use a pinned Cargo HTTP dependency on a background worker. Keep the
network and decoding off the game thread, and hand the decoded RGBA to a mutable client hook for upload. The image
hook neither fetches data nor provides a network service.

## Sprites made at load instead

An image that does not change while the game runs belongs in a sprite source, not in a live texture: bind a provider
directly in `Hooks::register_sprite_sources` via `event.register(id, provider)`, list an entry of its type in the GUI
atlas's definition (`assets/minecraft/atlases/gui.json`), and return `Ok(vec![SpriteSourceOutput::Pixels { ... }])`
from the provider for the id you want (`mymod:panel`, or `minecraft:container/slot` to replace a vanilla sprite). Any
valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted, later registrations replace earlier ones
(last-writer-wins), and registering a built-in vanilla source type overrides it when decoding atlas definitions. The
GUI atlas is one of the engine atlases that run bound providers; others (for example `particles`)
run their own listed sources. A mod-registered atlas runs its definition under the atlas's registered ID,
even when another atlas uses the same definition. An unused definition does not run any sources.
The GUI atlas publishes each sprite by its id: `gui_sprite`, `DrawContext::sprite` and the GUI's own draws find it there,
with the animation and nine-slice metadata of the file it names when an entry is an alias (a generated sprite has
none), and the bound provider runs again each time resources load. A GUI id its list has a sprite for is the list's:
a dynamic texture is not a sprite of this atlas, and a sprite the list removed draws the missing sprite, never a model's
sprite of the same id. After each load, ask `gui_sprite` again (`texture_atlas_stitched` for the GUI atlas): a region
from before a reload may have moved.
