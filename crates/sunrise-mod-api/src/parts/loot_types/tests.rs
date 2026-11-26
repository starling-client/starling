use super::*;
use crate::{LootStackEditor, LootTypeContext};
struct Codec;
impl crate::ModLootFunctionCodec for Codec {
    fn decode(&self, _: &serde_json::Value) -> Result<Arc<dyn crate::ModLootFunction>, String> {
        Ok(Arc::new(Rule))
    }
}
impl crate::ModLootConditionCodec for Codec {
    fn decode(&self, _: &serde_json::Value) -> Result<Arc<dyn crate::ModLootCondition>, String> {
        Ok(Arc::new(Rule))
    }
}
struct Rule;
impl crate::ModLootFunction for Rule {
    fn apply(
        &self,
        _: &mut LootStackEditor<'_>,
        _: &mut dyn LootTypeContext,
    ) -> Result<(), String> {
        Ok(())
    }
}
impl crate::ModLootCondition for Rule {
    fn test(&self, _: &mut dyn LootTypeContext) -> Result<bool, String> {
        Ok(true)
    }
}
#[test]
fn type_ids_accept_explicit_namespaces_refuse_builtins_and_duplicates_and_roll_back() {
    let data = mod_vocab::__private::RegistrationData::new(
        [
            (
                "minecraft:loot_function_type".into(),
                vec!["minecraft:set_count".into()],
            ),
            (
                "minecraft:loot_condition_type".into(),
                vec!["minecraft:random_chance".into()],
            ),
        ]
        .into(),
        Default::default(),
        crate::InstanceDirectory::from_path(std::path::Path::new("unused")).unwrap(),
        crate::RuntimeEnvironment::new(crate::Dist::Client, "26.3".into(), "test".into(), false),
    );
    crate::load_state::with_test_registration_data(data, || {
        let id = crate::Identifier::parse("test:entry").unwrap();
        let mut registrar = PartRegistrar::new("test");
        let r = &mut registrar as &mut dyn crate::Registrar;
        r.register_loot_function_type(&id, Codec);
        r.register_loot_condition_type(&id, Codec);
        r.register_loot_function_type(&id, Codec);
        r.register_loot_condition_type(&crate::Identifier::parse("other:entry").unwrap(), Codec);
        r.register_loot_function_type(
            &crate::Identifier::parse("minecraft:custom_fn").unwrap(),
            Codec,
        );
        r.register_loot_function_type(
            &crate::Identifier::parse("neoforge:custom_fn").unwrap(),
            Codec,
        );
        assert_eq!(registrar.loot_function_types.len(), 3);
        assert_eq!(registrar.loot_condition_types.len(), 2);
        assert_eq!(registrar.loot_condition_types[1].id, "other:entry");
        assert_eq!(registrar.loot_condition_types[1].owner, "test");
        assert_eq!(registrar.refused.len(), 1);
        assert!(registrar.refused[0].contains("test:entry registered by both test and test"));
        let rollback = registrar.registration_checkpoint();
        (&mut registrar as &mut dyn crate::Registrar).register_loot_function_type(
            &crate::Identifier::parse("test:discarded").unwrap(),
            Codec,
        );
        rollback(&mut registrar);
        assert_eq!(registrar.loot_function_types.len(), 3);

        let mut builtins = PartRegistrar::new("test");
        let r = &mut builtins as &mut dyn crate::Registrar;
        r.register_loot_function_type(
            &crate::Identifier::parse("minecraft:set_count").unwrap(),
            Codec,
        );
        r.register_loot_condition_type(
            &crate::Identifier::parse("minecraft:random_chance").unwrap(),
            Codec,
        );
        r.register_loot_condition_type(
            &crate::Identifier::parse("neoforge:loot_table_id").unwrap(),
            Codec,
        );
        r.register_loot_condition_type(
            &crate::Identifier::parse("neoforge:can_item_perform_ability").unwrap(),
            Codec,
        );
        assert!(builtins.loot_function_types.is_empty());
        assert!(builtins.loot_condition_types.is_empty());
        assert_eq!(builtins.refused.len(), 4);
        assert!(
            builtins
                .refused
                .iter()
                .all(|reason| reason.contains("built-in loot type"))
        );
    });
}
