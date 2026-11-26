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
fn serializer_registration_checks_ids_duplicates_and_builtin_reservation() {
    let mut registrar = PartRegistrar::new("test");
    let r = &mut registrar as &mut dyn crate::Registrar;
    r.register_global_loot_modifier_serializer("accepted", Codec);
    r.register_global_loot_modifier_serializer("accepted", Codec);
    r.register_global_loot_modifier_serializer("other:foreign", Codec);
    r.register_global_loot_modifier_serializer("minecraft:custom", Codec);
    r.register_global_loot_modifier_serializer("neoforge:custom", Codec);
    r.register_global_loot_modifier_serializer("sunrise:custom", Codec);
    r.register_global_loot_modifier_serializer("", Codec);
    r.register_global_loot_modifier_serializer("neoforge:add_table", Codec);
    assert_eq!(registrar.loot_modifier_serializers.len(), 5);
    assert_eq!(
        registrar
            .loot_modifier_serializers
            .iter()
            .map(|entry| (&*entry.id, entry.owner))
            .collect::<Vec<_>>(),
        [
            ("test:accepted", "test"),
            ("other:foreign", "test"),
            ("minecraft:custom", "test"),
            ("neoforge:custom", "test"),
            ("sunrise:custom", "test"),
        ]
    );
    assert_eq!(registrar.refused.len(), 3);
    assert!(registrar.refused[0].contains("registered by both test and test"));
    assert!(registrar.refused[1].contains("invalid identifier"));
    assert!(registrar.refused[2].contains("built-in"));
    let mut reserved = PartRegistrar::new("neoforge");
    (&mut reserved as &mut dyn crate::Registrar)
        .register_global_loot_modifier_serializer("add_table", Codec);
    assert!(reserved.loot_modifier_serializers.is_empty());
    assert!(reserved.refused[0].contains("built-in"));
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
