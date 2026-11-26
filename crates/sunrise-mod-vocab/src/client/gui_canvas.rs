//! Lexical, transactional GUI drawing.

use crate::{
    Identifier, ItemStack, StackWriteError, Text, TextLayoutError, TextLayoutSize, TextLayoutSpec,
    text::TextFormError,
};

use super::hud_layout::{Blend, PoseOp};

/// A partial `blitSprite`, in the sprite's logical pixel units.
// no wire: arguments to an in-process GUI draw
// exhaustive: plain sprite sub-rectangle coordinates built by struct literal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiSpriteRegion {
    /// Positive width and height of the whole sprite.
    pub size: [i32; 2],
    /// Source `u, v` offset; the destination width and height also give the source extents.
    pub offset: [i32; 2],
}

impl GuiSpriteRegion {
    /// The region at `offset` of a sprite whose whole size is `size`.
    pub fn new(size: [i32; 2], offset: [i32; 2]) -> Self {
        Self { size, offset }
    }
}

/// One corner of a [`GuiCanvas::triangles`] triangle, as vanilla's `POSITION_COLOR` GUI vertex.
// no wire: arguments to an in-process GUI draw
// exhaustive: plain vertex built by struct literal
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GuiVertex {
    /// `x, y` in the canvas's local GUI pixels, before its pose.
    pub pos: [f32; 2],
    /// Straight-alpha ARGB (`0xAARRGGBB`), blended across the triangle towards its other corners.
    pub argb: u32,
}

/// A source rectangle for `blit`, in the texture's logical pixel units.
// no wire: arguments to an in-process GUI draw
// exhaustive: plain texture source rectangle coordinates built by struct literal
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuiBlitRegion {
    /// Source `u, v` offset, which must be finite.
    pub uv: [f32; 2],
    /// Signed source extents. A negative extent reverses its UV axis.
    pub src: [i32; 2],
    /// Nonzero divisors for the two source axes, normally the whole texture's pixel size.
    pub tex: [i32; 2],
}

impl GuiBlitRegion {
    /// The source rectangle at `uv` with extents `src`, in a texture divided by `tex`.
    pub fn new(uv: [f32; 2], src: [i32; 2], tex: [i32; 2]) -> Self {
        Self { uv, src, tex }
    }
}

/// A refusal from a GUI call or its callback. A refused primitive adds no output; returning the
/// error from the callback discards its entire drawing group.
// no wire: errors stay with the in-process drawing callback
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GuiDrawError {
    /// This context cannot lend a GUI canvas.
    Unavailable,
    /// The callback chose to discard its group.
    Abort,
    /// A coordinate, pose, or result of draw arithmetic was not finite.
    NotFinite,
    /// Text could not be represented by the game (for example, it contains non-finite metadata).
    Text(TextFormError),
    /// The wrapping width, scale or scaled geometry cannot place the label.
    TextLayout(TextLayoutError),
    /// The stack could not be read for rendering. Storage count and damage limits do not apply.
    Stack(StackWriteError),
    /// No sprite is present at the requested GUI or packed-atlas id.
    UnknownSprite,
    /// `Vignette` and `Nausea` are only available for raw texture blits.
    UnsupportedSpriteBlend,
    /// The whole sprite size in a partial sprite blit was not positive on both axes.
    BadSpriteRegion,
    /// No raw texture is present at the requested id.
    UnknownTexture,
    /// A texture-axis divisor was zero.
    BadBlitRegion,
}

impl std::fmt::Display for GuiDrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "GUI drawing is unavailable in this context",
            Self::Abort => "GUI drawing was aborted by the caller",
            Self::NotFinite => "GUI draw geometry is not finite",
            Self::Text(error) => return write!(f, "GUI draw text: {error}"),
            Self::TextLayout(error) => return write!(f, "GUI draw: {error}"),
            Self::Stack(error) => return write!(f, "GUI draw stack: {error:?}"),
            Self::UnknownSprite => "unknown GUI draw sprite",
            Self::UnsupportedSpriteBlend => "this blend is not supported for GUI sprites",
            Self::BadSpriteRegion => "GUI sprite size must be positive",
            Self::UnknownTexture => "unknown GUI draw texture",
            Self::BadBlitRegion => "GUI blit texture-axis divisors must be nonzero",
        })
    }
}

impl std::error::Error for GuiDrawError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            Self::TextLayout(error) => Some(error),
            _ => None,
        }
    }
}

/// Checked GUI primitives for [`crate::DrawContext::draw_gui`], a child [`Self::scope`],
/// or a draw callback such as an item decorator. Coordinates are local GUI pixels and colours
/// are straight-alpha ARGB (`0xAARRGGBB`). A refused draw leaves the group as it was before that
/// call. Propagate the error with `?` to discard the group, or handle it to keep the other draws.
/// Widgets, scenes, focus and narration belong to a full [`crate::DrawContext`]; independent
/// host calls are not part of a canvas group.
pub trait GuiCanvas {
    /// The full GUI's width and height in GUI pixels, unaffected by the pose or scissor.
    fn size(&self) -> [f32; 2];

    /// Draw in a child scope. Apply `ops` in order to a copy of the current pose, then transform
    /// the local scissor (`[x, y, w, h]`) into screen space and intersect it with the enclosing
    /// scissor (or the viewport if there is none). The two ordered corners are transformed, flooring the resulting origin and
    /// extents separately. `None` inherits the enclosing scissor unchanged. Empty, inverted or
    /// reflected scissor axes draw nothing. The clip does not move when a descendant changes its
    /// pose, and only restricts visible pixels, not interpolation inside the clip. Non-finite
    /// geometry refuses the scope before calling `draw`; zero scale is allowed.
    /// On `Err` or unwind, discard the child's draws. On success they stay in order, but are
    /// still discarded if an enclosing group fails. The parent's pose and scissor never change.
    fn scope(
        &mut self,
        ops: &[PoseOp],
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError>;

    /// Fill `[x, y, w, h]` in the current pose and scissor, sorting the corners on both axes.
    fn fill(&mut self, rect: [f32; 4], argb: u32) -> Result<(), GuiDrawError>;

    /// Fill `[x, y, w, h]` with `top` at `y` and `bottom` at `y + h`. Zero area or inversion
    /// on exactly one axis draws nothing; inverting both axes keeps the colours on their input
    /// edges.
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) -> Result<(), GuiDrawError>;

    /// Draw untextured triangles from the mod's own corners, in order, for shapes a rectangle
    /// cannot make: rounded panels, discs and rings, line icons, arrows. This is what a mod's
    /// own `GuiElementRenderState` with `POSITION_COLOR` triangles draws when submitted through
    /// NeoForge's `GuiGraphicsExtractor.submitGuiElementRenderState`. Each corner takes the
    /// current pose and scissor like the other draws, its colour is blended across the
    /// triangle, and either winding draws. A corner that is not finite, before or after the
    /// pose, refuses the whole call with [`GuiDrawError::NotFinite`].
    fn triangles(&mut self, triangles: &[[GuiVertex; 3]]) -> Result<(), GuiDrawError>;

    /// Draw styled text, including legacy formatting, with `argb` as the default colour.
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        argb: u32,
        shadow: bool,
    ) -> Result<(), GuiDrawError>;

    /// Wrap and draw a styled label directly in this canvas's pose and scissor, as
    /// vanilla's `GuiGraphicsExtractor.textWithWordWrap`. `at` is the label's top left
    /// in local GUI pixels. Returns its dimensions before the enclosing pose. Both
    /// text measurement and paint use the same layout; no replay list is produced.
    /// Non-finite scaled layout geometry is refused even for blank or transparent text.
    /// For an off-draw layout, use `ClientHost::visit_text_lines`.
    fn text_with_word_wrap(
        &mut self,
        text: &Text,
        at: [f32; 2],
        spec: TextLayoutSpec,
        argb: u32,
        shadow: bool,
    ) -> Result<TextLayoutSize, GuiDrawError>;

    /// Draw the stack's 16-pixel GUI item model with its top-left at `at`, passing `seed` to
    /// its model resolver. This does not draw decorations. An empty stack draws nothing;
    /// inventory storage limits do not apply. New head-skin requests wait for the outer group
    /// to commit.
    fn item(&mut self, stack: &ItemStack, at: [f32; 2], seed: i32) -> Result<(), GuiDrawError>;

    /// [`Self::item`] with every vertex it draws, the enchantment glint's included, multiplied
    /// by straight-alpha ARGB `argb`: a fading pickup icon, a translucent ghost item in a recipe
    /// preview, a dimmed unavailable slot. `0xFFFFFFFF` draws exactly what [`Self::item`] draws,
    /// and a zero alpha (`argb >> 24 == 0`) draws nothing, as [`Self::text`] does: the item is
    /// not drawn at all, so its model hooks do not run for that call. An OptiFine CIT glint
    /// layer of a resource pack takes the tint's alpha only, as its own colour comes from the
    /// pack. The current
    /// pose and scissor apply as for [`Self::item`]. Vanilla 26.3's `GuiGraphicsExtractor.item`
    /// always draws opaque (its item blit's colour is `-1`); this is the multiplier
    /// `RenderSystem.setShaderColor` gave around `GuiGraphics.renderItem` before 1.21.6.
    fn item_tinted(
        &mut self,
        stack: &ItemStack,
        at: [f32; 2],
        seed: i32,
        argb: u32,
    ) -> Result<(), GuiDrawError>;

    /// Draw the stack's bar, cooldown, count and custom item decorations at `at`, in that order.
    /// In an item decorator's canvas or its children, this paints only the standard bar, cooldown
    /// and count, without invoking item decorators again.
    /// `count` replaces the normal count label; `Some("")` hides it. An empty stack draws nothing.
    /// Call this in the surrounding scope after drawing a squeezed model in a child
    /// [`Self::scope`], so the decorations follow the surrounding pose, not the squeeze.
    fn item_decorations(
        &mut self,
        stack: &ItemStack,
        at: [f32; 2],
        count: Option<&str>,
    ) -> Result<(), GuiDrawError>;

    /// Draw a present GUI sprite by id at `dst = [x, y, w, h]`, with an optional source region.
    /// A removed sprite is refused. Accepts `Blend::Textured` and `Blend::Crosshair`.
    fn blit_sprite(
        &mut self,
        name: &str,
        dst: [f32; 4],
        region: Option<GuiSpriteRegion>,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError>;

    /// Draw a sprite by its exact packed atlas name (not its GUI id) at `[x, y, w, h]`.
    fn atlas_sprite(&mut self, name: &str, dst: [f32; 4], argb: u32) -> Result<(), GuiDrawError>;

    /// Draw a region of a texture packed in the GUI atlas at `[x, y, w, h]`: `texture` is the
    /// packed name (a path under `textures/`, without `.png`) or a registered atlas's sprite
    /// (`mymod:gui/atlases/icons/mymod/heart`); `minecraft:name` and `name` are the same texture.
    /// Accepts every [`Blend`]. A live dynamic texture
    /// or a standalone resource-pack texture is not in the atlas and is refused with
    /// [`GuiDrawError::UnknownTexture`]: draw those with [`Self::texture`]. A non-finite `dst`
    /// or `uv` is refused with [`GuiDrawError::NotFinite`] and a zero `tex` side with
    /// [`GuiDrawError::BadBlitRegion`], before the name is looked up, as in [`Self::texture`].
    fn blit(
        &mut self,
        texture: &str,
        dst: [f32; 4],
        region: GuiBlitRegion,
        argb: u32,
        blend: Blend,
    ) -> Result<(), GuiDrawError>;

    /// `GuiGraphicsExtractor.blit` of a standalone texture, a picture outside the GUI atlas such
    /// as a guide book's chapter art, at `dst = [x, y, w, h]`, tinted by ARGB `argb`, under the
    /// current pose and scissor.
    ///
    /// One lookup rule for `id`: a live dynamic texture of that id
    /// (`ClientRenderClientHost::create_dynamic_texture`, of any mod: drawing has no owner check)
    /// is drawn, nearest with repeat and one mip level; otherwise `id` is the exact resource (`mymod:textures/gui/chapter.png`),
    /// read from the active resource stack, mod packs included, never rewritten: name the
    /// `textures/` folder and `.png` yourself. The quad samples `region.uv / region.tex` to
    /// `(region.uv + region.src) / region.tex` unclamped, as `innerBlit` does (a negative extent
    /// flips that axis), with the sampler the winning `.png.mcmeta` sets: `blur` filters
    /// linearly, `clamp` clamps to the edge, else nearest and repeat, no mipmaps. The image loads
    /// on first use and stays loaded until the next resource reload. An `id` that names neither a
    /// live dynamic texture nor a resource in the stack when this is called is refused with
    /// [`GuiDrawError::UnknownTexture`] and draws nothing. An image that is there but undecodable,
    /// or a `.png.mcmeta` vanilla cannot read, draws vanilla's missing texture in its
    /// place rather than refusing, and so does a dynamic texture released after this call
    /// and before the frame draws, as vanilla's `TextureManager.release` leaves its id. A
    /// non-finite `uv` is refused with [`GuiDrawError::NotFinite`] and a zero `tex` side with
    /// [`GuiDrawError::BadBlitRegion`].
    fn texture(
        &mut self,
        id: &Identifier,
        dst: [f32; 4],
        region: GuiBlitRegion,
        argb: u32,
    ) -> Result<(), GuiDrawError>;

    /// Draw the player `uuid`'s skin head (`PlayerFaceExtractor.extractRenderState`): the face,
    /// then the hat layer when `hat`, `size` GUI pixels square at `at`, tinted by ARGB `argb`,
    /// under the current pose and scissor. `flip` draws it upside down; nothing is read from a
    /// name. A player on the connection's player list draws its `PlayerInfo` skin; any other UUID
    /// its profile's, looked up once in the background as a player head's is. Until that skin is
    /// downloaded and uploaded, and when there is none, the UUID's default skin is drawn
    /// (`DefaultPlayerSkin.get`), so the call never waits. New skin requests wait for the outer
    /// group to commit.
    fn player_face(
        &mut self,
        uuid: u128,
        at: [f32; 2],
        size: f32,
        hat: bool,
        flip: bool,
        argb: u32,
    ) -> Result<(), GuiDrawError>;
}
