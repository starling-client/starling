//! Static registration for data-enabled global loot modifier types.
use super::*;

#[cfg(test)]
#[path = "loot_modifiers/tests.rs"]
mod tests;

impl GlobalLootModifiersRegistrar for PartRegistrar {
    fn add_global_loot_modifier_serializer(
        &mut self,
        name: &str,
        serializer: Arc<dyn crate::GlobalLootModifierSerializer>,
    ) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        if ["minecraft:", "neoforge:", "sunrise:"]
            .iter()
            .any(|namespace| id.starts_with(namespace))
        {
            self.refused.push(format!(
                "{} claimed reserved global loot modifier serializer {id}",
                self.mod_id
            ));
            return;
        }
        if let Some(previous) = self
            .loot_modifier_serializers
            .iter()
            .find(|entry| *entry.id == id)
        {
            self.refused.push(format!(
                "global loot modifier serializer {id} registered by both {} and {}",
                previous.owner, self.mod_id,
            ));
            return;
        }
        self.loot_modifier_serializers
            .push(LootModifierSerializerInfo {
                id: id.into(),
                owner: self.mod_id,
                owner_index: self.mod_index,
                serializer,
            });
    }
}
