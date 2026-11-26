mod loot_types;
pub use loot_types::{
    LootTypeContext, ModLootCondition, ModLootConditionCodec, ModLootFunction, ModLootFunctionCodec,
};
mod __private {
    pub use crate::parts::loot_type_declarations;
}
