//! A block loot table's conditions. `LootItemCondition.CODEC` is a
//! `RegistryFileCodec` over the reloadable `Registries.PREDICATE`, so a
//! `condition` string is a predicate id — the file
//! `data/<ns>/predicate/<path>.json`, which a pack replaces
//! (`minecraft:tool/can_silk_touch`) or adds (`mypack:my_silk`) — and an
//! inline object is the condition itself (`LootItemCondition.DIRECT_CODEC`,
//! dispatched on its `type`).
use super::LootContext;
use serde_json::Value;
use sunrise_core::random::RandomSource;

impl<R: RandomSource> LootContext<'_, R> {
    /// The condition `id` names in `Registries.PREDICATE`. A root whose
    /// `predicate/` files were never loaded (`Items::for_tests`, a scratch
    /// root) keeps the two ids vanilla's own block tables name readable as
    /// their own kinds, as this engine read every string before the registry.
    pub(super) fn named_condition(&self, id: &str, random: &mut R) -> bool {
        match self.items.predicates.get(id) {
            Some(condition) => self.condition(condition, random),
            None => match id {
                "minecraft:tool/can_silk_touch" => {
                    self.enchantment_level("minecraft:silk_touch") > 0
                }
                "minecraft:tool/can_shear" => {
                    self.held.is_some_and(|h| &*h.item == "minecraft:shears")
                }
                _ => false,
            },
        }
    }

    /// A pool's, an entry's or a conditional function's `condition` (`LootPool.CODEC`,
    /// `LootPoolEntryContainer.CODEC`, `LootItemConditionalFunction`'s codec). Those codecs have
    /// no `conditions` field: the list old pack data writes is ignored, as vanilla ignores any
    /// unknown field.
    pub(super) fn conditions(&self, o: &Value, random: &mut R) -> bool {
        o.get("condition").is_none_or(|c| self.condition(c, random))
    }
}
