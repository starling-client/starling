//! Painting the engine's HUD draw list with items and every blit blend included.
use super::{GuiDraw, paint_with};
use crate::hud::{HudAtlas, HudBatch};

/// What draws a `GuiDraw::Item`: the stack of hotbar slot `slot` (0-8, 40 = the off hand).
pub trait GuiItems {
    /// `GuiGraphicsExtractor.item(owner, stack, x, y, seed)`: the slot's stack with its top left at
    /// `(x, y)`, 16 GUI pixels square; nothing for an empty slot.
    fn item(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32, seed: i32);
    /// `GuiGraphicsExtractor.itemDecorations(font, stack, x, y)`: the slot's count, bar and
    /// cooldown; nothing for an empty slot.
    fn decorations(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32);
}

/// [`super::paint`], with each `Item` drawn by `items` (scaled about its pivot, then its
/// decorations) and the blits of every blend: `Vignette` darkens what is under it, `Nausea`
/// adds to it and `Crosshair` inverts it. The hotbar draws its items with it.
pub fn paint_items(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    draws: &[GuiDraw],
    items: &mut dyn GuiItems,
) {
    paint_with(batch, atlas, draws, Some(items));
}

#[cfg(test)]
#[path = "hud_layout_items_tests.rs"]
mod tests;
