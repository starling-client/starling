//! Starling's part of creative.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Creative {
    /// How far past an item tab's grid of items a stack put down still goes back to the tab.
    pub const GRID_MARGIN: f32 = 4.0;


    /// Whether `mouse`, relative to the screen origin, is on an item tab's grid of `rows` rows of
    /// items or within [`Creative::GRID_MARGIN`] of it. The display slots' hover boxes leave no
    /// gap between them (`isHovering` grows each by a pixel); the margin is Starling's.
    pub fn on_item_grid(mouse: (f32, f32), rows: usize) -> bool {
        let m = Self::GRID_MARGIN;
        let (left, top) = (8.0 - m, 17.0 - m);
        let (right, bottom) = (8.0 + 9.0 * 18.0 + m, 17.0 + rows as f32 * 18.0 + m);
        mouse.0 >= left && mouse.0 < right && mouse.1 >= top && mouse.1 < bottom
    }


    /// Whether `mouse`, relative to the screen origin, is within `reach` of the middle of the
    /// Survival Inventory tab's Destroy Item slot, as a finger means a slot a little past it.
    pub fn near_trash(mouse: (f32, f32), reach: f32) -> bool {
        let (x, y) = Self::TRASH_POSITION;
        (mouse.0 - x - 8.0).abs() <= reach && (mouse.1 - y - 8.0).abs() <= reach
    }

    /// Bedrock Inventory: the crafting grid and its result in the room at the player's right.
    pub fn bedrock_craft_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            100 => Some((129.0, 5.0)),
            101 => Some((147.0, 5.0)),
            102 => Some((129.0, 23.0)),
            103 => Some((147.0, 23.0)),
            200 => Some((173.0, 14.0)),
            _ => None,
        }
    }


    /// Starling: the tab the screen works with, without `select`'s search box and scroll: Bedrock
    /// Inventory shows two tabs' windows and goes from one to the other inside a frame.
    pub fn show(&mut self, tab: usize) {
        if let Some(t) = self.tabs.get(tab) {
            self.selected_id.clone_from(&t.id);
        }
    }
}

impl Creative {
    /// Bedrock Inventory's item window: six rows of items, the last right under the fifth.
    pub fn six_row_slots() -> impl Iterator<Item = usize> {
        Self::DISPLAY..Self::DISPLAY + 54
    }

}
