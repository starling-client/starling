//! Starling's part of chunk.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Chunk {
    /// The blocks of the section at `section` (`y >> 4`); None outside the
    /// world's height, where [`Self::get`] reads 0.
    pub fn section_blocks(&self, section: i32) -> Option<&PalettedContainer> {
        let s = section - (MIN_Y >> 4);
        usize::try_from(s)
            .ok()
            .and_then(|s| self.sections.get(s))
            .map(|section| &section.blocks)
    }
}
