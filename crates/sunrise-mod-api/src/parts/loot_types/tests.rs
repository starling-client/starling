use super::*;
use crate::{LootEntryData, LootStack, LootTypeContext};
struct Codec;
impl crate::ModLootFunction for Codec {
    fn validate(&self, _: &LootEntryData) -> Result<(), String> {
        Ok(())
    }
    fn apply(
        &self,
        _: &LootEntryData,
        stack: LootStack,
        _: &mut dyn LootTypeContext,
    ) -> Result<LootStack, String> {
        Ok(stack)
    }
}
impl crate::ModLootCondition for Codec {
    fn validate(&self, _: &LootEntryData) -> Result<(), String> {
        Ok(())
    }
    fn test(&self, _: &LootEntryData, _: &mut dyn LootTypeContext) -> Result<bool, String> {
        Ok(true)
    }
}
#[test]
fn type_ids_are_owned_unique_per_registry_and_rolled_back() {
    let id = crate::Identifier::parse("test:entry").unwrap();
    let mut registrar = PartRegistrar::new("test");
    let r = &mut registrar as &mut dyn crate::Registrar;
    r.register_loot_function_type(&id, Codec);
    r.register_loot_condition_type(&id, Codec);
    r.register_loot_function_type(&id, Codec);
    r.register_loot_condition_type(&crate::Identifier::parse("other:entry").unwrap(), Codec);
    assert_eq!(registrar.loot_function_types.len(), 1);
    assert_eq!(registrar.loot_condition_types.len(), 1);
    assert!(registrar.refused[0].contains("test:entry registered by both test and test"));
    assert!(registrar.refused[1].contains("namespace"));
    let rollback = registrar.registration_checkpoint();
    (&mut registrar as &mut dyn crate::Registrar)
        .register_loot_function_type(&crate::Identifier::parse("test:discarded").unwrap(), Codec);
    rollback(&mut registrar);
    assert_eq!(registrar.loot_function_types.len(), 1);
    for namespace in ["minecraft", "neoforge"] {
        let mut reserved = PartRegistrar::new(namespace);
        (&mut reserved as &mut dyn crate::Registrar).register_loot_function_type(
            &crate::Identifier::parse(&format!("{namespace}:set_count")).unwrap(),
            Codec,
        );
        assert!(reserved.loot_function_types.is_empty());
        assert!(reserved.refused[0].contains("reserved loot type"));
    }
}
