//! Static loot map-codec declarations.
use super::*;
#[cfg(test)]
#[path = "loot_types/tests.rs"]
mod tests;

/// Built-in NeoForge loot condition types handled directly by the engine.
const BUILTIN_NEOFORGE_LOOT_CONDITIONS: &[&str] = &[
    "neoforge:loot_table_id",
    "neoforge:can_item_perform_ability",
];

impl PartRegistrar {
    fn loot_type_id(&mut self, id: &crate::Identifier, condition: bool) -> Option<String> {
        let id = id.to_string();
        let builtin = if condition {
            BUILTIN_NEOFORGE_LOOT_CONDITIONS.contains(&id.as_str())
                || self.has_builtin_entry("minecraft:loot_condition_type", &id)
        } else {
            self.has_builtin_entry("minecraft:loot_function_type", &id)
        };
        if builtin {
            self.refused.push(format!(
                "mod {} claimed built-in loot type {id}",
                self.mod_id
            ));
            return None;
        }
        let previous = if condition {
            self.loot_condition_types
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.owner)
        } else {
            self.loot_function_types
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.owner)
        };
        if let Some(previous) = previous {
            self.refused.push(format!(
                "loot type {id} registered by both {previous} and {}",
                self.mod_id
            ));
            return None;
        }
        Some(id)
    }
}
impl LootTypesRegistrar for PartRegistrar {
    fn add_loot_function_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootFunctionCodec>,
    ) {
        let Some(id) = self.loot_type_id(id, false) else {
            return;
        };
        self.loot_function_types.push(LootFunctionTypeInfo {
            id,
            owner: self.mod_id,
            callback_owner: mod_vocab::__private::parts_installation::callback_owner_for_index(
                self.mod_id,
                self.mod_index,
            ),
            codec,
        });
    }
    fn add_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootConditionCodec>,
    ) {
        let Some(id) = self.loot_type_id(id, true) else {
            return;
        };
        self.loot_condition_types.push(LootConditionTypeInfo {
            id,
            owner: self.mod_id,
            callback_owner: mod_vocab::__private::parts_installation::callback_owner_for_index(
                self.mod_id,
                self.mod_index,
            ),
            codec,
        });
    }
}
