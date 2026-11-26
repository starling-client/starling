//! Starling's part of font.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Font {
    /// The glyph for a character drawn in `style`, in one of the default font's takes when it has
    /// some (`variants`), chosen by `seed`: the same seed, the same take. Elsewhere `glyph_styled`.
    pub fn glyph_varied(&self, c: char, style: &Style, seed: u32) -> Glyph {
        let take = (seed % (self.variants.len() as u32 + 1)) as usize;
        if take == 0 || style.font.is_some() || !c.is_ascii() {
            return self.glyph_styled(c, style);
        }
        self.glyph_of(&self.variants[take - 1], c)
    }
}
