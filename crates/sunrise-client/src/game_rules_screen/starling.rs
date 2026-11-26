//! Starling's part of game_rules_screen.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl RulesScreen {
    /// The box that takes the keys, for the on-screen keyboard's editor on Android: 0 the
    /// search, then the rules' number boxes in their order. `edit` sets it.
    pub fn typed_box(&self) -> Option<(usize, &EditBox)> {
        if self.confirm {
            return None;
        }
        if self.search.focused {
            return Some((0, &self.search));
        }
        let i = self.numbers.iter().position(|(_, b)| b.focused)?;
        Some((i + 1, &self.numbers[i].1))
    }
}
