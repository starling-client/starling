# Design: dynamic textures for client mods

> The native contract below still applies. WebAssembly mods were removed; the parts about guests are history.
> See [textures made at run time](../../modding/src/how-to/client/dynamic-textures.md) for the current native API.

Answer to [#1343](https://github.com/sunrise-org/sunrise/issues/1343), built to the contract the maintainers settled
in the issue on 2026-11-14: a client mod creates an RGBA image under a name, draws it as a GUI sprite, updates it and
frees it, and reads what remains of its budget. The how-to is
[Textures made at run time](../../modding/src/how-to/client/dynamic-textures.md).

## Vanilla 26.3

`TextureManager.register(id, new DynamicTexture(label, nativeImage))` makes a texture under an id,
`DynamicTexture.upload()` sends changed pixels, `GuiGraphics.blit(id, ...)` draws it, and `TextureManager.release(id)`
frees it. NeoForge and Fabric API add nothing to it; mods use it without a mixin. Vanilla has no cap on how many a mod
registers, so the budgets below are limits of Sunrise's atlas route, stated as such. Read from the issue and public
documentation; the decompiled 26.3 sources were not at hand when this was written.

## Calls

On the native `ClientRenderClientHost`:

```rust
#[access(WorldOnly)]
fn create_dynamic_texture(&mut self, id: &Identifier, width: u32, height: u32, rgba: &[u8])
    -> Result<(), DynamicTextureError>;
#[access(WorldOnly)]
fn update_dynamic_texture(&mut self, id: &Identifier, width: u32, height: u32, rgba: &[u8])
    -> Result<(), DynamicTextureError>;
#[access(WorldOnly)]
fn free_dynamic_texture(&mut self, id: &Identifier) -> Result<(), DynamicTextureError>;
#[access(WorldOnly)]
fn dynamic_texture_budget(&self) -> Result<DynamicTextureBudget, DynamicTextureError>;
```

The types and constants (`DYNAMIC_TEXTURE_CLASSES`, `DYNAMIC_TEXTURE_MAX_SIDE`, `DYNAMIC_TEXTURE_MOD_IMAGES`,
`DYNAMIC_TEXTURE_MOD_BYTES`) live in `crates/sunrise-mod-vocab/src/client/dynamic_texture.rs`; the pool and its
decisions in `crates/sunrise-client-render/src/hud/dynamic_textures.rs`; the game's calls in
`crates/sunrise-client/src/game/modding/native_dynamic_textures.rs`.

The calls take `&mut self` and `WorldOnly`, as `mod_image` did: binding a name changes the atlas's sprite table,
which every sprite path reads, so it is a write of the game's state, made from a hook that holds the host mutably (a
client tick, for example), not from inside a draw.

## Where the images live

In the HUD atlas, as a pool of fixed slots in three size classes, `DYNAMIC_TEXTURE_CLASSES`: 64 x 64, 256 x 256 and
1024 x 1024. An image takes a slot of the smallest class whose side holds both its width and its height.

`HudAtlas::build_over_with_dynamic_textures(root, defaults, game, pool)` packs the slots as empty images beside the
atlas's other run-time slots (maps, skins, pack icons), and only when `pool` is true. The game passes
`resource_tables::dynamic_texture_pool()`, true when any native client mod is loaded, with no mixin and no
opt-in (the rule of #4437), and asks it again at every resource reload. A game with no mod packs no slot and its
atlas does not grow. After packing, the slots' names (`sunrise/dynamic-texture/<side>/<n>`) leave the atlas's
sprites for `HudAtlas::dynamic_texture_pool`, so no mod can draw a slot by name and see another mod's image.

**Slot counts (decision).** `DYNAMIC_TEXTURE_SLOTS` is 32 small, 8 medium and 2 large slots: 32 x 64 x 64 +
8 x 256 x 256 + 2 x 1024 x 1024 texels, 10.5 MiB of atlas, paid only with a client mod loaded. The default resources
pack a 4096 x 5120 atlas (80 MiB), so the pool adds about an eighth of it; the packer rounds the height up to whole
1024-row bands, so how many rows it adds depends on how the pool's shelves fall among the packs' images. The reasons
for each count:

- 2 large slots: a mod's whole byte budget is one 1024 x 1024 image (a world map, a schematic view, the use the old
  shared slot served). Two let two mods each hold one, where the old route let one mod hold it for everyone. A third
  would cost the atlas another 4 MiB for a case no known mod has.
- 8 medium slots, 2 MiB: panels and gauges, half of one mod's count budget.
- 32 small slots, 512 KiB: icons and glyph sheets, two mods' whole count of 16.

A full class is refused by name (`NoSlot`); the mod can free an image or fit a smaller class. The pool is shared by
every mod, first come; the per-mod budgets keep one mod from holding all of it (16 images take at most half the small
class, 4 MiB at most one large slot).

## Names

The id is an `Identifier` in the calling mod's namespace (`crash::current_native_mod`, as runtime levels are owned):
`mymod:dynamic/panel` for the mod `mymod`. Creating an image inserts the id into the atlas's sprites, as the image's
part of its slot (its `width` x `height` texels from the slot's corner, with that GUI size). Every sprite path then
draws it as any other sprite: `DrawContext::sprite` (`native_sprite`, `src` in the image's own pixels),
`gui_nine_slice`, `tiled_sprite`, `GuiDraw::Blit` and `gui_sprite`. An id that already names an image or a sprite of
the packs is refused (`NameTaken`), so an image never shadows a pack's sprite. Free removes the name and returns the
slot to the pool.

## Refusals

`DynamicTextureError` is a `#[non_exhaustive]` `Wire` enum. Every refusal changes nothing and is decided before the
pixels are copied:

| Refusal | When |
|---|---|
| `NotOwned` | the id is outside the calling mod's namespace, or no mod is calling |
| `BadSize` | a side is 0, or the pixels are not `width * height * 4` bytes |
| `TooLarge` | a side is longer than 1024 (`DYNAMIC_TEXTURE_MAX_SIDE`) |
| `NoSlot` | every slot of the image's class is held |
| `OverBudget` | the mod would pass 16 images or 4 MiB of pixels |
| `NameTaken` | `create` of an id that names an image or a GUI sprite |
| `Unknown` | `update` or `free` of an id that names no image |
| `ClassChange` | `update` to a size of another class than the image's |
| `Unavailable` | the atlas has no pool: no window (a server), or no client mod loaded |

An image larger than 1024 x 1024 is `TooLarge`: textures of their own, bound per draw, are the follow-up to this
issue, not part of it.

## Budgets

Per mod, `DYNAMIC_TEXTURE_MOD_IMAGES` (16) images and `DYNAMIC_TEXTURE_MOD_BYTES` (4 MiB) of pixels, counted as
`width * height * 4` an image, which is also what the game keeps of it on the CPU. `dynamic_texture_budget` answers
what remains (`DynamicTextureBudget { images, bytes }`); a free slot is not part of it, since the slots are shared.
These are limits of the atlas route, stated in the docs, because vanilla's `TextureManager` has none.

## Update

An update writes the new pixels into the slot the image already holds, from the next frame, and the sprite takes the
new size. **A size of another class is refused with `ClassChange`, not moved (decision).** The contract says an
update re-patches the same slot; moving to another class would make an update also take a slot, so it could fail with
`NoSlot` halfway through, or hold two slots for a moment, in a way the mod did not ask for. A mod that wants another
class frees the image and creates it again, which says exactly what it gives back and what it takes.

## Reload and a new renderer

A resource reload rebuilds the atlas from the packs, so its texture loses the images. The game keeps the last pixels
of each image on the CPU (counted in the byte budget). After the reload, `DynamicTextures::rebind` puts every name
back into the new atlas's sprites, at the same class and slot of the new pool, and the next frame patches the new
atlas with the kept pixels. Names persist; a freed image is gone with its pixels. A new renderer (another graphics
API, a lost device) makes its atlas texture from the packed pixels alone, so `forget_uploads` patches every image
again.

## Memory

Nothing is allocated per draw: a dynamic texture is a sprite rect like any other. The pixels are copied once per
create or update, into the kept copy, and that copy is what the budget counts. The frame's patch shares the copy
(`ImagePatch` holds an `Arc<[u8]>` of it, in `HudBatch::image_patches`) instead of copying it again, and an update of
the same size writes into the kept copy in place when no frame still holds it. Each image is patched at most once a
frame, however often it was updated. Every refusal is decided before the copy.

## The old call

`ClientRenderClientHost::mod_image`, one 1024 x 1024 slot (`sunrise/modimage`) shared by every mod, is removed in this
change with its slot, its test and its row (`tools/ci/ignored-tests/mod-image.txt`), so there is one way to put a
mod's image on screen. Change note: `crates/sunrise-mod-vocab/changes/dynamic-textures.toml`. Diverges from
upstream: horizon-rust/horizon keeps `native_mod_image`.

## Tests

- `hud::dynamic_textures::tests`: the slot and budget decisions with no GPU (classes, a full class, a freed slot and
  budget coming back, an update in the same slot and in place), every refusal and that it changes nothing, and a
  rebuilt atlas getting every kept image again.
- `hud::tests::atlas::the_dynamic_texture_pool_exists_only_when_a_mod_is_loaded`: no pool and no growth without a mod; with
  one, every slot whole, apart from the others and out of the sprite table.
- `game::native_dynamic_textures_tests::the_pool_follows_the_loaded_client_mods`.
- Historical: the removed `wire_loopback::dynamic_textures` fixture covered calls and refusals across the guest boundary.
- Daily (`#[ignore = "daily"]`, its row in `tools/ci/ignored-tests/daily-sunrise-client-dynamic-textures.txt`):
  `a_mods_dynamic_texture_draws_under_its_id_until_it_is_freed` draws an image created by a mod with no mixin through
  `native_sprite` on the real HUD pipeline.
