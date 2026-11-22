//! Static, manifest-owned loot map-codec declarations.
use super::*;
#[cfg(test)]
#[path = "loot_types/tests.rs"]
mod tests;

impl PartRegistrar {
    fn loot_type_id(&mut self, id: &crate::Identifier, condition: bool) -> Option<String> {
        let id = match self.owned_declaration_id(&id.to_string()) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return None;
            }
        };
        if id.starts_with("minecraft:") || id.starts_with("neoforge:") {
            self.refused.push(format!(
                "mod {} claimed reserved loot type {id}",
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
        codec: Arc<dyn crate::ModLootFunction>,
    ) {
        let Some(id) = self.loot_type_id(id, false) else {
            return;
        };
        self.loot_function_types.push(LootFunctionTypeInfo {
            id,
            owner: self.mod_id,
            owner_index: self.mod_index,
            codec,
        });
    }
    fn add_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootCondition>,
    ) {
        let Some(id) = self.loot_type_id(id, true) else {
            return;
        };
        self.loot_condition_types.push(LootConditionTypeInfo {
            id,
            owner: self.mod_id,
            owner_index: self.mod_index,
            codec,
        });
    }
}
