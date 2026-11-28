//! Starling's part of item_data.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl ItemStack {
    /// `ItemCooldowns.getCooldownGroup`: `use_cooldown`'s group, else the item's id.
    pub fn cooldown_group(&self) -> String {
        let group = self
            .data()
            .and_then(|d| d.other("minecraft:use_cooldown"))
            .and_then(|c| c.get("cooldown_group"));
        match group {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => self.item.to_string(),
        }
    }

}
