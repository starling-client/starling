//! Painting a whole command buffer, items and every blit blend included: the hotbar's items and
//! a mod's `gui_paint`. A mod's buffer is checked first ([`undrawable`]): one op the paint
//! cannot draw refuses the buffer, and nothing of it is drawn.
use super::{Blend, GuiDraw, paint_with};
use crate::hud::{GuiSprite, HudAtlas, HudBatch};

/// What draws a `GuiDraw::Item`: the stack of hotbar slot `slot` (0-8, 40 = the off hand).
pub trait GuiItems {
    /// `GuiGraphicsExtractor.item(owner, stack, x, y, seed)`: the slot's stack with its top left at
    /// `(x, y)`, 16 GUI pixels square; nothing for an empty slot.
    fn item(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32, seed: i32);
    /// `GuiGraphicsExtractor.itemDecorations(font, stack, x, y)`: the slot's count, bar and
    /// cooldown; nothing for an empty slot.
    fn decorations(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32);
}

/// Why [`paint_checked`] drew nothing: the op it could not draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undrawable {
    /// An `Item` of a slot that is not a hotbar slot (0-8) or the off hand (40).
    ItemSlot(u32),
    /// An `Item` whose scale or pivot is NaN or infinite.
    ItemNotFinite,
    /// A `Blit` of a texture the GUI atlas does not hold.
    BlitTexture(String),
    /// A `Blit` whose texture size is not positive, or whose `uv` is NaN or infinite.
    BlitSize,
    /// A `Sprite` or `AtlasSprite` the GUI atlas does not hold, a mob effect sprite included.
    Sprite(String),
    /// A `Sprite` whose `region` has a zero (or negative) sprite width or height.
    SpriteRegion,
    /// A `Sprite` with a blend the sprite painter cannot draw (`Nausea`, `Vignette`).
    SpriteBlend(Blend),
}

/// Whether `slot` names a hotbar slot or the off hand, as `GuiDraw::Item` takes it.
pub fn item_slot(slot: u32) -> bool {
    slot <= 8 || slot == 40
}

/// The first op of `draws` [`paint_checked`] cannot draw, with its index. It reads the buffer
/// and allocates a sprite's name only to report its refusal.
pub fn undrawable(atlas: &HudAtlas, draws: &[GuiDraw]) -> Option<(usize, Undrawable)> {
    draws
        .iter()
        .enumerate()
        .find_map(|(i, d)| refusal(atlas, d).map(|why| (i, why)))
}

fn refusal(atlas: &HudAtlas, draw: &GuiDraw) -> Option<Undrawable> {
    match draw {
        GuiDraw::Item {
            slot, scale, pivot, ..
        } => {
            if !item_slot(*slot) {
                Some(Undrawable::ItemSlot(*slot))
            } else if !scale.iter().chain(pivot).all(|v| v.is_finite()) {
                Some(Undrawable::ItemNotFinite)
            } else {
                None
            }
        }
        GuiDraw::Blit {
            texture, uv, tex, ..
        } => {
            if atlas.blit_rect(texture.as_ref()).is_none() {
                Some(Undrawable::BlitTexture(texture.to_string()))
            } else if tex[0] <= 0 || tex[1] <= 0 || !uv.iter().all(|v| v.is_finite()) {
                Some(Undrawable::BlitSize)
            } else {
                None
            }
        }
        GuiDraw::Sprite {
            sprite,
            region,
            blend,
            ..
        } => {
            if matches!(blend, Blend::Nausea | Blend::Vignette) {
                return Some(Undrawable::SpriteBlend(*blend));
            }
            if region.is_some_and(|[sw, sh, ..]| sw <= 0 || sh <= 0) {
                return Some(Undrawable::SpriteRegion);
            }
            // As `paint` looks it up, by GUI id. A sprite the GUI atlas lists as missing, or does
            // not own, is refused, not drawn as the missing texture.
            let known = matches!(atlas.gui_lookup(sprite), Some(GuiSprite::Present { .. }));
            (!known).then(|| Undrawable::Sprite(sprite.to_string()))
        }
        GuiDraw::AtlasSprite { sprite, .. } => {
            let known = atlas.sprites.contains_key(sprite.as_ref());
            (!known).then(|| Undrawable::Sprite(sprite.to_string()))
        }
        GuiDraw::Fill { .. }
        | GuiDraw::FillGradient { .. }
        | GuiDraw::Text { .. }
        | GuiDraw::Push(_)
        | GuiDraw::Pop => None,
    }
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

/// [`paint_items`] for a buffer a mod sent: an op it cannot draw ([`undrawable`]) refuses the
/// whole buffer, with that op's index, and nothing of it is drawn.
pub fn paint_checked(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    draws: &[GuiDraw],
    items: &mut dyn GuiItems,
) -> Result<(), (usize, Undrawable)> {
    if let Some(refused) = undrawable(atlas, draws) {
        return Err(refused);
    }
    paint_items(batch, atlas, draws, items);
    Ok(())
}

#[cfg(test)]
#[path = "hud_layout_items_tests.rs"]
mod tests;
