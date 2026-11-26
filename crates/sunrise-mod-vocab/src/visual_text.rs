//! Styled characters obtained from the client's visual text sequence.
use crate::text::{TextStyle, discard_style_tags};

/// One character accepted by vanilla's `FormattedCharSink` in visual order.
/// Deep opaque style data is released iteratively when the character is dropped.
// exhaustive: the three values supplied by FormattedCharSink form this record.
// no wire: explicit retention of an in-process visitor value.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualTextCharacter {
    /// Unicode scalar value, including shaping and mirroring applied by the client.
    pub scalar: u32,
    /// Full effective style, including click, hover and font information.
    pub style: TextStyle,
    /// UTF-16 offset within the sink's current directional/style run.
    /// This is not a global component offset; it restarts for each run.
    pub source_index: u32,
}

impl Drop for VisualTextCharacter {
    fn drop(&mut self) {
        discard_style_tags(&mut self.style);
    }
}

/// A character lent by `ClientHost::visit_text_visual_order`.
// no wire: a borrow from an in-process visitor.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct VisualTextCharacterRef<'a> {
    /// The Unicode scalar in visual order, with shaping and mirroring already applied.
    pub scalar: u32,
    /// The character's effective style, valid for the visitor call.
    pub style: &'a TextStyle,
    /// UTF-16 offset in the current directional and effective-style run.
    pub source_index: u32,
}

impl<'a> VisualTextCharacterRef<'a> {
    // hidden: the engine's visual-order walk builds the characters it lends; mods only receive them.
    #[doc(hidden)]
    pub fn new(scalar: u32, style: &'a TextStyle, source_index: u32) -> Self {
        Self {
            scalar,
            style,
            source_index,
        }
    }
    /// Retain this character and its style independently of the visitor.
    pub fn to_owned(self) -> VisualTextCharacter {
        VisualTextCharacter {
            scalar: self.scalar,
            style: self.style.clone(),
            source_index: self.source_index,
        }
    }
}
