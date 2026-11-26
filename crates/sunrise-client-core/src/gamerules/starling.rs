//! Starling's part of gamerules.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl GameRules {
    /// Starling: whether a boolean rule is on, as Sunrise's `GameRules::bool` answered and native
    /// mods made for it ask (`game.ui.rules.bool("block_drops")`).
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Value::Bool(true)))
    }
}
