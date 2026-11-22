//! What [`crate::DrawContext::gui_paint`] answers when it draws nothing of a buffer.
use crate::text::TextFormError;

/// Why [`crate::DrawContext::gui_paint`] drew nothing of its buffer. `index` is the refused op's
/// place in the buffer; the ops before it were not drawn either.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GuiPaintError {
    /// A `Text` op's text could not cross to the game (unrepresentable metadata).
    Text(TextFormError),
    /// A `GuiDraw::Item` names a slot that is not a hotbar slot (0-8) or the off hand (40).
    ItemSlot {
        /// The op's place in the buffer.
        index: u32,
        /// The slot it named.
        slot: u32,
    },
    /// A `GuiDraw::Item` whose `scale` or `pivot` is NaN or infinite.
    ItemNotFinite {
        /// The op's place in the buffer.
        index: u32,
    },
    /// A `GuiDraw::Blit` of a texture the GUI atlas does not hold.
    BlitTexture {
        /// The op's place in the buffer.
        index: u32,
        /// The texture it named.
        texture: String,
    },
    /// A `GuiDraw::Blit` whose texture size (`tex`) is not positive, or whose `uv` is NaN or
    /// infinite.
    BlitSize {
        /// The op's place in the buffer.
        index: u32,
    },
    /// A `GuiDraw::Sprite` the GUI atlas does not hold by GUI id (one its definition removed
    /// included), a mob effect sprite (`mob_effect/...`, namespaced or not) included, or a
    /// `GuiDraw::AtlasSprite` the packed block atlas does not hold by name.
    Sprite {
        /// The op's place in the buffer.
        index: u32,
        /// The sprite it named.
        sprite: String,
    },
    /// A `GuiDraw::Sprite` whose `region` has a sprite width or height of zero (or less).
    SpriteRegion {
        /// The op's place in the buffer.
        index: u32,
    },
    /// A `GuiDraw::Sprite` with a blend sprites cannot be drawn with (`Nausea`, `Vignette`).
    SpriteBlend {
        /// The op's place in the buffer.
        index: u32,
        /// The blend it named.
        blend: crate::client::hud_layout::Blend,
    },
}
impl From<TextFormError> for GuiPaintError {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}
impl std::fmt::Display for GuiPaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(error) => write!(f, "gui_paint: text: {error}"),
            Self::ItemSlot { index, slot } => {
                write!(
                    f,
                    "gui_paint: op {index}: Item of slot {slot}, not 0-8 or 40"
                )
            }
            Self::ItemNotFinite { index } => {
                write!(f, "gui_paint: op {index}: Item scale or pivot not finite")
            }
            Self::BlitTexture { index, texture } => {
                write!(
                    f,
                    "gui_paint: op {index}: Blit of unknown texture {texture}"
                )
            }
            Self::BlitSize { index } => {
                write!(
                    f,
                    "gui_paint: op {index}: Blit texture size not positive or uv not finite"
                )
            }
            Self::Sprite { index, sprite } => {
                write!(f, "gui_paint: op {index}: unknown sprite {sprite}")
            }
            Self::SpriteRegion { index } => {
                write!(
                    f,
                    "gui_paint: op {index}: Sprite region width or height not positive"
                )
            }
            Self::SpriteBlend { index, blend } => {
                write!(
                    f,
                    "gui_paint: op {index}: Sprite cannot draw with blend {blend:?}"
                )
            }
        }
    }
}
impl std::error::Error for GuiPaintError {}
