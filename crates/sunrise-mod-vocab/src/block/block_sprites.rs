//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// A sprite: its array (0 for every vanilla texture on most GPUs; past the first array's layers in
/// 1 to 7) and its layer in that array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SpriteRef {
    pub array: u32,
    pub layer: u32,
    /// It animates (water, lava, fire, sea lanterns...): the arrays always hold the frame being shown.
    pub animated: bool,
    /// Some of its pixels are clear (alpha 0): leaves, glass, flowers. Draw those as holes.
    pub cutout: bool,
}
/// One sprite of the blocks atlas, read on the CPU ([`RenderAssets::atlas_sprite_pixels`](crate::RenderAssets::atlas_sprite_pixels)):
/// the sprite's first frame (vanilla's `SpriteContents` for the sprite, as stitched after resource-pack
/// layering and its `.mcmeta`), untinted, at its own resolution.
// no wire: an owned read of the published sprite, never sent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasSpritePixels {
    /// The frame's width in pixels.
    pub width: u32,
    /// The frame's height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8, rows from the top, pixels from the left
    /// (`NativeImage.Format.RGBA`'s order), not premultiplied and not tinted.
    pub rgba: Vec<u8>,
}

/// Why [`RenderAssets::atlas_sprite_pixels`](crate::RenderAssets::atlas_sprite_pixels) read nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of a local read.
pub enum AtlasSpriteError {
    /// The blocks atlas of the published resources holds no sprite at this reference: its array
    /// or layer is past the last. A reference from an earlier resource load that is still in range
    /// is not refused and may name a different sprite; resolve references again when
    /// `block_sprites_version` changes.
    NoSuchSprite,
    /// No resource load has published a blocks atlas yet.
    NotReady,
}

// hidden: the engine builds the answer from the published atlas pixels; a mod only reads it.
#[doc(hidden)]
pub fn atlas_sprite_pixels(width: u32, height: u32, rgba: Vec<u8>) -> AtlasSpritePixels {
    AtlasSpritePixels {
        width,
        height,
        rgba,
    }
}

/// One face of a block's model on one of its sides.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FaceSprite {
    pub sprite: SpriteRef,
    /// Its texture by the name `BlockEntityDrawContext::cube` and `quad` take (`block/oak_log`: vanilla's
    /// without its namespace, a mod's with it), to draw the face elsewhere: a copycat block drawing its
    /// material's faces on its own shape, with `uv` for the face's turn.
    pub texture: String,
    /// Where the face lies on the side, in the side's two axes (`side_axes`), from 0 to 1 across the block:
    /// `[a0, b0, a1, b1]`. `[0, 0, 1, 1]` for a whole face.
    pub rect: [f32; 4],
    /// How deep into the block the face lies from the side's own plane (0 for a cube's face, 0.5 for a slab's top).
    pub inset: f32,
    /// The texture across the side: the point (a, b) on it shows `uv = [u0, v0] + a * [du_u, du_v] + b * [dv_u,
    /// dv_v]` of the sprite, `[u0, v0, du_u, du_v, dv_u, dv_v]`. Rotations and mirrors (a log on its side, a
    /// model's `rotation` and `uvlock`) are in it.
    pub uv: [f32; 6],
    /// Its colour source, none for an untinted face: multiply the texture by the source's colour.
    pub tint: Option<TintSource>,
    /// The model's tint index for the face, -1 for none: `ClientHost::block_tint_color` turns it into the
    /// colour the face is multiplied by at a position.
    pub tint_index: i32,
    /// It has partly clear pixels: drawn sorted and blended.
    pub translucent: bool,
}
impl FaceSprite {
    /// This face drawn on side `side` of the box `from`..`to` (in blocks, 0 to 1 across a block) of
    /// another block, as a copycat draws its material: the corners and the UV rectangle (`[u0, v0, u1,
    /// v1]` in texels of 16 per side) to give `BlockEntityDrawContext::quad`, with [`FaceSprite::texture`]
    /// as its sprite. The texture runs as it does on the material's own side, turned and mirrored as its
    /// model turns it (`uv`: a log on its side, glazed terracotta), and is cut where the box covers the
    /// side, so copycats side by side line up as the material's blocks would. The corners go
    /// counterclockwise seen from outside the box. None when the texture doesn't run along the
    /// side's axes (an element turned by 22.5 degrees about the side's normal): no UV rectangle
    /// draws it, so draw it otherwise than with `quad`.
    pub fn quad_on(
        &self,
        side: Direction,
        from: [f32; 3],
        to: [f32; 3],
    ) -> Option<([[f32; 3]; 4], [f32; 4])> {
        let (a, b) = match side {
            Direction::Down | Direction::Up => (0, 2),
            Direction::North | Direction::South => (0, 1),
            Direction::West | Direction::East => (2, 1),
        };
        let normal = side.normal().map(|n| n as f32);
        let axis = normal.iter().position(|n| *n != 0.0).unwrap_or(1);
        let plane = if normal[axis] > 0.0 {
            to[axis]
        } else {
            from[axis]
        };
        let mut ab = [
            [from[a], from[b]],
            [from[a], to[b]],
            [to[a], to[b]],
            [to[a], from[b]],
        ];
        let point = |[pa, pb]: [f32; 2]| {
            let mut p = [0.0; 3];
            p[axis] = plane;
            p[a] = pa;
            p[b] = pb;
            p
        };
        let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
        let [p0, p1, p2] = [point(ab[0]), point(ab[1]), point(ab[2])];
        let (e, f) = (sub(p1, p0), sub(p2, p0));
        let cross = [
            e[1] * f[2] - e[2] * f[1],
            e[2] * f[0] - e[0] * f[2],
            e[0] * f[1] - e[1] * f[0],
        ];
        if cross[0] * normal[0] + cross[1] * normal[1] + cross[2] * normal[2] < 0.0 {
            ab.reverse();
        }
        let [u0, v0, du_u, du_v, dv_u, dv_v] = self.uv;
        let uv = ab.map(|[pa, pb]| [u0 + pa * du_u + pb * dv_u, v0 + pa * du_v + pb * dv_v]);
        // `quad` gives its corners the rectangle's (u0, v0), (u0, v1), (u1, v1) and (u1, v0) in turn: start
        // at a corner whose next one keeps its u, and the rectangle's order carries any turn or mirror.
        // The first edge keeps u and the next keeps v; a texture that runs across the side's axes has
        // no such corner.
        let same = |p: f32, q: f32| (p - q).abs() < 1e-4;
        let k = (0..4).find(|&i| {
            same(uv[i][0], uv[(i + 1) % 4][0]) && same(uv[(i + 1) % 4][1], uv[(i + 2) % 4][1])
        })?;
        let order = [k, k + 1, k + 2, k + 3].map(|i| i % 4);
        let (first, opposite) = (uv[k], uv[(k + 2) % 4]);
        Some((
            order.map(|i| point(ab[i])),
            [first[0], first[1], opposite[0], opposite[1]].map(|t| t * 16.0),
        ))
    }
}

/// What a block draws on each side, in [`Direction::ALL`]'s order (down, up, north, south, west, east).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BlockSprites {
    /// The faces on each side, in the model's order: the one drawn first first (a grass block's side: its dirt
    /// and grass, then the grass overlay tinted over it). A side with none is empty. Faces that do not lie flat
    /// on a side (a flower's crossed planes, a rotated element) are left out.
    pub sides: [Vec<FaceSprite>; 6],
    /// The model's particle sprite (what breaking it scatters, a flower's whole picture), if it has one.
    pub particle: Option<SpriteRef>,
    /// The model is a plain cube: one whole face on each side, at its plane.
    pub cube: bool,
}

/// An atlas the client stitches its sprites into (vanilla's `TextureAtlas`), as
/// `texture_atlas_stitched` names it. Sunrise packs vanilla's many atlases into these two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextureAtlas {
    /// The block sprite arrays: every texture a block model uses, one layer each. Look a sprite up
    /// with `texture_sprites`, which answers a [`SpriteRef`].
    Blocks,
    /// The GUI atlas: GUI sprites, particles and the item and block textures drawn flat. Look a
    /// sprite up with `gui_sprite`, which answers an [`AtlasRegion`].
    Gui,
}

/// An atlas a client mod adds beside the game's (`RegisterTextureAtlasesEvent`), as
/// `register_texture_atlases` declares it: its id and the atlas definition listing its sprites.
/// Sunrise stitches its sprites into a texture of their own, each under [`Self::sprite_name`], and
/// the UVs of its sprites are local to that texture, not the shared GUI atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods submit declarations by value
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextureAtlasRegistration {
    /// The atlas's id (`mymod:icons`). Any namespace is allowed except collisions with vanilla
    /// 26.3's built-in engine atlases (`AtlasManager.KNOWN_ATLASES`, such as `minecraft:blocks` or
    /// `minecraft:gui`).
    pub atlas: Identifier,
    /// Its definition, `assets/<namespace>/atlases/<path>.json` (`mymod:icons`): a `sources` list
    /// like vanilla's atlas definitions (`directory`, `single`, `filter`, `unstitch`,
    /// `paletted_permutations`), which resource packs can change like theirs. Must not reuse a
    /// built-in engine atlas definition (`AtlasManager.KNOWN_ATLASES`) or a definition already
    /// registered by an earlier mod.
    pub definition: Identifier,
}

impl TextureAtlasRegistration {
    /// The GUI atlas name of this atlas's sprite `sprite` (`mymod:heart` in `mymod:icons` is
    /// `mymod:gui/atlases/icons/mymod/heart`): what `gui_sprite` looks it up by and
    /// `DrawContext::sprite` draws.
    pub fn sprite_name(&self, sprite: &Identifier) -> Identifier {
        let path = format!(
            "gui/atlases/{}/{}/{}",
            self.atlas.path(),
            sprite.namespace(),
            sprite.path()
        );
        Identifier::from_parts(self.atlas.namespace(), &path)
            .expect("the parts of two identifiers form a valid path")
    }
}

/// Where a sprite lies in the GUI atlas, as `gui_sprite` answers it: plain data, a snapshot of the
/// atlas built last. It stays right until the next `texture_atlas_stitched` for
/// [`TextureAtlas::Gui`], when a mod reads it again.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AtlasRegion {
    /// `[u0, v0, u1, v1]`: the sprite's corners, from 0 to 1 across the texture that holds it:
    /// the shared GUI atlas, or the sprite's own atlas texture for one of a registered atlas
    /// (`TextureAtlasRegistration`). Draw it by its name so the owning texture is selected.
    pub uv: [f32; 4],
    /// The sprite's size in its own pixels: the default resources' texture's size whatever the
    /// resolution of a pack that replaced it (the units vanilla's GUI code draws it in), the
    /// texture's own size for one the default resources lack.
    pub size: [u32; 2],
}

impl AtlasRegion {
    /// A region, from the game's atlas.
    pub fn new(uv: [f32; 4], size: [u32; 2]) -> Self {
        Self { uv, size }
    }
}

/// An emissive overlay sprite pair a client mod registers during resource load and reload
/// (`Hooks::emissive`). Transparent pixels in `overlay` leave `base` unchanged; opaque pixels
/// draw full-bright without illuminating surrounding blocks or changing gameplay light.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods submit declarations by value
#[derive(sunrise_mod_api_macros::Wire)]
pub struct EmissiveSpriteRegistration {
    /// The base block sprite id (`minecraft:block/redstone_ore` or `mymod:block/reactor`).
    pub base: Identifier,
    /// The overlay texture id (`mymod:block/reactor_e` or `mymod:emissive/reactor_glow`),
    /// resolved from `assets/<namespace>/textures/<path>.png`.
    pub overlay: Identifier,
}

impl EmissiveSpriteRegistration {
    /// Pair a base block sprite with an emissive overlay texture.
    pub fn new(base: Identifier, overlay: Identifier) -> Self {
        Self { base, overlay }
    }
}
