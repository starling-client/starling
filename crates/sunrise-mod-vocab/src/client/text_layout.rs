//! A label's lines, as `DrawContext::text_lines` measures and places them: no widget, no theme, no
//! state, so a mod draws each line itself.
use crate::{Text, text::TextFormError};

/// Where a label's lines sit inside its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextAlign {
    /// Lined up on the left edge of the width.
    Left,
    /// Centred in the width: `width / 2 - floor(line width / 2)` from its left edge, as vanilla's
    /// `TextAlignment.CENTER.calculateLeft` floors the half line width.
    Centre,
    /// Lined up on the right edge of the width.
    Right,
}

/// One line of a label: the piece of the component it carries, and where to draw it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextLine {
    /// The component's characters this line holds, styles and colours kept, its legacy `§` codes
    /// already applied to those styles and dropped (a `§r` restores the style its run carried).
    pub text: Text,
    /// Where to draw it, in scaled GUI pixels: `x` from the label's left edge, `y` from its top
    /// (`0` for the first line).
    pub at: [f32; 2],
    /// Its width in scaled GUI pixels, as `Font.width` measures it, styles included.
    pub width: f32,
}

impl TextLine {
    /// A line `width` GUI pixels wide, drawn with its top left at `at`.
    pub fn new(text: Text, at: [f32; 2], width: f32) -> Self {
        Self { text, at, width }
    }
}

/// A label laid out: its lines, already placed inside its width, and the block it occupies.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextLines {
    /// The lines, top to bottom.
    pub lines: Vec<TextLine>,
    /// The widest line's width in scaled GUI pixels, `0` with no line.
    pub width: f32,
    /// The block's height in scaled GUI pixels: the last line's `at[1]` plus one line height, `0`
    /// with no line. What a scroll container sizes its content from.
    pub height: f32,
}

impl TextLines {
    /// The layout a draw answers: `lines` already placed, the widest line's width and the block's
    /// height.
    pub fn new(lines: Vec<TextLine>, width: f32, height: f32) -> Self {
        Self {
            lines,
            width,
            height,
        }
    }
}

/// Why [`crate::DrawContext::text_lines`] answered no layout: nothing was measured, and no partial
/// list of lines is answered with it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextLinesRefusal {
    /// `max_width` is NaN or infinite, or so much larger than `scale` that dividing it by `scale`
    /// overflows: there is no box to wrap in and none to place lines in.
    WidthNotFinite,
    /// `scale` is NaN, infinite, zero or less: no line has a position or a size to answer.
    ScaleInvalid,
    /// The text wraps into more than 512 lines.
    TooManyLines,
    /// The lines hold more than 8192 characters together.
    TooManyGlyphs,
    /// The component could not cross to the game (deeper than 512 component nodes, malformed).
    Text(TextFormError),
}

impl From<TextFormError> for TextLinesRefusal {
    fn from(error: TextFormError) -> Self {
        Self::Text(error)
    }
}

impl std::fmt::Display for TextLinesRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WidthNotFinite => {
                f.write_str("text_lines: max_width or max_width / scale is NaN or infinite")
            }
            Self::ScaleInvalid => f.write_str("text_lines: scale is NaN, infinite, zero or less"),
            Self::TooManyLines => {
                f.write_str("text_lines: the text wraps into more than 512 lines")
            }
            Self::TooManyGlyphs => {
                f.write_str("text_lines: the lines hold more than 8192 characters")
            }
            Self::Text(error) => write!(f, "text_lines: text: {error}"),
        }
    }
}

impl std::error::Error for TextLinesRefusal {}
