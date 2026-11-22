mod loot_types;
pub use loot_types::{LootEntryData, LootTypeContext, ModLootCondition, ModLootFunction};
mod __private {
    pub use crate::parts::loot_type_declarations;
}
