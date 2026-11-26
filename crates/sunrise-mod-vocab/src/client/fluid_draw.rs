//! Still-fluid boxes drawn by block entity, mob and player submit hooks
//! ([`BlockEntityDrawContext::fluid_box`](crate::BlockEntityDrawContext::fluid_box)).

/// How [`BlockEntityDrawContext::fluid_box`](crate::BlockEntityDrawContext::fluid_box) draws a
/// still fluid: a machine's tank, a basin's contents.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FluidBoxOptions {
    /// `0xAARRGGBB` drawn in place of the stack's own tint: plain water's `0xFF3F76E4`
    /// (never the biome's water colour) and lava's white. `None` keeps the stack's.
    pub argb: Option<u32>,
    /// Whether the box's bottom face is drawn; the other five always are.
    pub render_bottom: bool,
    /// `TextureAtlasSprite.uvShrinkRatio`: how far each face's UVs pull in towards
    /// the sprite's centre. A port passes its sprite's own, 4 over the largest dimension of the
    /// atlas it was stitched into; Sunrise keeps block sprites in arrays, so it asks for it.
    pub uv_shrink_ratio: f32,
}

impl FluidBoxOptions {
    /// The stack's own tint, `render_bottom` and `uv_shrink_ratio`.
    #[inline]
    pub fn new(render_bottom: bool, uv_shrink_ratio: f32) -> Self {
        Self {
            argb: None,
            render_bottom,
            uv_shrink_ratio,
        }
    }

    /// The same options drawn in `argb` (`0xAARRGGBB`) instead of the stack's tint.
    #[inline]
    pub fn with_argb(self, argb: u32) -> Self {
        Self {
            argb: Some(argb),
            ..self
        }
    }
}
