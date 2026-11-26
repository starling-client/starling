//! Starling's part of recipe_book.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Book {
    /// Which book a recipe id belongs to (Starling's recipe placement, game/recipe_book.rs).
    pub fn book_of(&self, id: &str) -> Option<BookType> {
        let e = self.entries.iter().find(|e| e.id == id)?;
        Some(match e.category {
            Category::CraftingBuilding
            | Category::CraftingRedstone
            | Category::CraftingEquipment
            | Category::CraftingMisc => BookType::Crafting,
            Category::FurnaceFood | Category::FurnaceBlocks | Category::FurnaceMisc => {
                BookType::Furnace
            }
            Category::BlastBlocks | Category::BlastMisc => BookType::BlastFurnace,
            Category::SmokerFood => BookType::Smoker,
        })
    }
}
