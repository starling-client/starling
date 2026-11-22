# Textures made at run time

A client mod that bakes its own images (a panel, a gauge, glyphs from its own font, a map, a schematic view) creates
each one under a name and draws it as a GUI sprite. This is vanilla's `DynamicTexture`: Java registers one with
`TextureManager.register(id, texture)`, sends its pixels with `DynamicTexture.upload()`, draws it with
`GuiGraphics.blit(id, ...)` and frees it with `TextureManager.release(id)`. In Sunrise the image lives in a slot of
the GUI atlas, so it draws as any other GUI sprite.

```rust
use mod_api::prelude::*;
use mod_api::{DynamicTextureError, Identifier};

const PANEL: &str = "mymod:dynamic/panel";

/// Bake the panel once: 64 by 32 pixels, row by row from the top left, four bytes (RGBA) each.
fn bake_panel(host: &mut dyn ClientHost) -> Result<(), DynamicTextureError> {
    let (width, height) = (64, 32);
    let rgba = [24, 24, 48, 230].repeat(width * height);
    let id = Identifier::parse(PANEL).unwrap();
    host.create_dynamic_texture(&id, width as u32, height as u32, &rgba)
}

fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
    // The whole image (`src` in its own pixels), stretched over 128 by 64 GUI pixels.
    event
        .batch
        .sprite(PANEL, [0.0, 0.0, 64.0, 32.0], [8.0, 8.0, 128.0, 64.0], None);
}
```

The four calls are on the client host. They change
the game's atlas, so they take `&mut dyn ClientHost`: call them from a hook that has it, such as `client_tick_post`,
not from inside a draw.

- **`create_dynamic_texture(id, width, height, rgba)`** makes the image. From the next frame, its id is a GUI sprite of
  `width` x `height` pixels: `DrawContext::sprite`, `gui_nine_slice`, `tiled_sprite`, `gui_paint`'s `Blit` and
  `gui_sprite` draw and find it as any other sprite.
- **`update_dynamic_texture(id, width, height, rgba)`** gives it new pixels, written into the slot it already holds on
  the next frame. The size may change within the image's class (below); the sprite takes the new size. Update when
  the image changed, not every frame: each update copies the pixels and uploads the image.
- **`free_dynamic_texture(id)`** frees it: the name is gone at once, the slot goes back to the pool and the image's
  share of the budget back to the mod. An image never freed lasts until the game ends.
- **`dynamic_texture_budget()`** answers what the mod may still create, `DynamicTextureBudget { images, bytes }`.

## Names

The id is in a namespace owned by the calling mod (`mymod:...`, its extra manifest namespaces, or a content package it serves via `served_by`); the path is the mod's to choose. No mod can update or free
another's image: an id outside the calling mod's owned namespaces (or created by another mod) is `NotOwned`, and textures created under served namespaces share the calling host's per-mod budget. An id that already names one of the mod's images, or a GUI
sprite of the packs (`mymod:gui/panel`), is `NameTaken`.

## Slots and size classes

The images live in a pool of fixed slots of the GUI atlas, in three size classes (`DYNAMIC_TEXTURE_CLASSES`). An image
takes a slot of the smallest class whose side holds both its width and its height:

| Class | Slot | Slots in the pool |
|---|---|---|
| Small | 64 x 64 | 32 |
| Medium | 256 x 256 | 8 |
| Large | 1024 x 1024 | 2 |

The slots are shared by every mod, first come. When every slot of the image's class is held, `create` is `NoSlot`:
free an image, or make it fit a smaller class. A side longer than 1024 (`DYNAMIC_TEXTURE_MAX_SIDE`) is `TooLarge`:
textures of their own, for larger images, are not part of this API.

An update re-patches the slot the image holds, so a size of another class is `ClassChange`: free the image and create
it again at the new size.

The pool exists only when a client mod is loaded; a game without mods has no slot and its atlas does
not grow. Where there is no pool (a server, or no client mod loaded) every call is `Unavailable`.

## Budgets

| Limit | Value | Refusal |
|---|---|---|
| Images a mod holds | `DYNAMIC_TEXTURE_MOD_IMAGES`, 16 | `OverBudget` |
| Pixels a mod holds | `DYNAMIC_TEXTURE_MOD_BYTES`, 4 MiB (`width * height * 4` an image) | `OverBudget` |
| Longest side | `DYNAMIC_TEXTURE_MAX_SIDE`, 1024 pixels | `TooLarge` |

These are limits of the atlas route; vanilla's `TextureManager` has none, so read what remains with
`dynamic_texture_budget` rather than assume. A 1024 x 1024 image takes a mod's whole byte budget. The bytes are what
the game keeps of each image's last pixels, to put them back after a resource reload; an update counts its new size
in place of the old, and a free gives its share back.

## Refusals

Every call answers `Result<_, DynamicTextureError>`, and a refused call changes nothing:

| Refusal | When |
|---|---|
| `NotOwned` | the id is outside the calling mod's namespace |
| `BadSize` | a side is 0, or the pixels are not `width * height * 4` bytes |
| `TooLarge` | a side is longer than 1024 |
| `NoSlot` | every slot of the image's class is held |
| `OverBudget` | the mod would pass 16 images or 4 MiB |
| `NameTaken` | `create` of an id that names an image or a GUI sprite |
| `Unknown` | `update` or `free` of an id that names no image |
| `ClassChange` | `update` to a size of another class |
| `Unavailable` | no pool here: a server, or no client mod loaded |

Match `DynamicTextureError` non-exhaustively.

## Inline images in chat and signs

Implement `Hooks::substitute_inline_images` to display atlas sprites in chat history and on
both sign faces. Each call sees one translated, styled `event.run` with legacy formatting already applied. The hook
runs before wrapping, width measurement and bidi reordering, not for each glyph as it is painted.

```rust
use mod_api::prelude::*;
use mod_api::{Identifier, InlineImageReplacement, SubstituteInlineImagesEvent};

fn substitute_inline_images(&self, _: &dyn ClientHost, event: &mut SubstituteInlineImagesEvent<'_>) {
    let sprite = Identifier::parse("mymod:dynamic/star").unwrap();
    for (start, matched) in event.run.match_indices(":star:") {
        let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(start + matched.len())) else {
            continue;
        };
        event.images.push(InlineImageReplacement { start, end, sprite: sprite.clone() });
    }
}
```

Each half-open UTF-8 byte range must stay within the run and must not include a newline. The first valid proposal in
hook dispatch order wins over overlapping proposals. A missing sprite or invalid range leaves the text alone unless
another valid proposal covers it. Each replacement occupies one 8×8 `minecraft:gui` glyph. Chat clicks and hovers keep
the span's style; signing, search and copy still use the original message. On a sign, an image uses its
span's colour, or white if the span has none; the sign's dye does not tint it. Sign data and the editor keep
the original text. Name tags and the chat input field do not use this hook.

Give the hook the id of a pack sprite or an existing dynamic texture. Keep the listener a cheap match: it may run
again when text, resources, sprite geometry or the enabled mods change. Create or free the texture in a mutable client
hook, not during substitution. For an animation, keep one image id and update its same-sized pixels as frames change;
there is no need to rebuild the text layout.

A trusted native mod downloading images can use a pinned Cargo HTTP dependency on a background worker. Keep the
network and decoding off the game thread, and hand the decoded RGBA to a mutable client hook for upload. The image
hook neither fetches data nor provides a network service.

## Resource reloads

A resource pack change rebuilds the GUI atlas. Every image keeps its name, class and slot, and the game puts its last
pixels into the new atlas by itself: there is nothing to upload again. Sampling is the atlas's: nearest, as GUI
sprites.

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
`create_dynamic_texture` refuses it as taken, and a sprite the list removed draws the missing sprite, never a model's
sprite of the same id. After each load, ask `gui_sprite` again (`texture_atlas_stitched` for the GUI atlas): a region
from before a reload may have moved.
