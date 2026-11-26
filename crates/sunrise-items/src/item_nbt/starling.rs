//! Starling's part of item_nbt.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Starling: the item tables of the world being played (`set_current_items`), for the one-argument
/// `item_to_nbt` Horizon's game had, which mixin mods made for Horizon call ([`item_to_nbt_current`]).
pub(super) static CURRENT_ITEMS: std::sync::RwLock<Option<Arc<crate::items::Items>>> =
    std::sync::RwLock::new(None);

/// Starling: the item tables a world's resources loaded, for [`item_to_nbt_current`].
pub fn set_current_items(items: Arc<crate::items::Items>) {
    *CURRENT_ITEMS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(items);
}

/// Starling: Horizon's `item_to_nbt(stack)`, with the tables of the world being played; before any
/// world loaded, the stack's id and count alone.
pub fn item_to_nbt_current(stack: &crate::items::ItemStack) -> Tag {
    let items = CURRENT_ITEMS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    match items {
        Some(items) => item_to_nbt(stack, &items),
        None => Tag::compound([
            ("id", Tag::String((*stack.item).into())),
            ("count", Tag::Int(stack.count as i32)),
        ]),
    }
}
