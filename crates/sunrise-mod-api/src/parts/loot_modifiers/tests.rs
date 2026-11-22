use super::*;
use crate::{DataLoadContext, GlobalLootModifier, GlobalLootModifierSerializer, LootModifierError};

struct Codec;
impl GlobalLootModifierSerializer for Codec {
    fn decode(
        &self,
        _: &serde_json::Value,
        _: &dyn DataLoadContext,
    ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
        Err(LootModifierError::new(
            "$",
            "this registration fixture has no instances",
        ))
    }
}

#[test]
fn serializer_registration_checks_namespace_duplicates_and_reserved_ids() {
    let mut registrar = PartRegistrar::new("test");
    let r = &mut registrar as &mut dyn crate::Registrar;
    r.register_global_loot_modifier_serializer("accepted", Codec);
    r.register_global_loot_modifier_serializer("accepted", Codec);
    r.register_global_loot_modifier_serializer("other:foreign", Codec);
    r.register_global_loot_modifier_serializer("", Codec);
    assert_eq!(registrar.loot_modifier_serializers.len(), 1);
    assert_eq!(&*registrar.loot_modifier_serializers[0].id, "test:accepted");
    assert_eq!(registrar.refused.len(), 3);
    assert!(registrar.refused[0].contains("registered by both test and test"));
    assert!(registrar.refused[1].contains("namespace"));
    assert!(registrar.refused[2].contains("invalid identifier"));
    let mut reserved = PartRegistrar::new("neoforge");
    (&mut reserved as &mut dyn crate::Registrar)
        .register_global_loot_modifier_serializer("add_table", Codec);
    assert!(reserved.loot_modifier_serializers.is_empty());
    assert!(reserved.refused[0].contains("reserved"));
}

#[test]
fn refused_registration_does_not_leave_a_serializer_or_reserve_its_id() {
    let mut registrar = PartRegistrar::new("test");
    (&mut registrar as &mut dyn crate::Registrar)
        .register_global_loot_modifier_serializer("kept", Codec);
    let rollback = registrar.registration_checkpoint();
    (&mut registrar as &mut dyn crate::Registrar)
        .register_global_loot_modifier_serializer("refused", Codec);
    rollback(&mut registrar);
    assert_eq!(registrar.loot_modifier_serializers.len(), 1);
    assert_eq!(&*registrar.loot_modifier_serializers[0].id, "test:kept");
    (&mut registrar as &mut dyn crate::Registrar)
        .register_global_loot_modifier_serializer("refused", Codec);
    assert!(registrar.refused.is_empty());
    assert_eq!(registrar.loot_modifier_serializers.len(), 2);
}
