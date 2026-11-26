//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Vanilla 26.3 `RegistryDataLoader` datapack registries (`WORLD_REGISTRIES`,
/// `DIMENSION_REGISTRIES`, `RELOADABLE_REGISTRIES`).
const BUILTIN_DATAPACK_REGISTRIES: &[&str] = &[
    "minecraft:dimension_type",
    "minecraft:worldgen/biome",
    "minecraft:chat_type",
    "minecraft:worldgen/carver",
    "minecraft:worldgen/feature",
    "minecraft:worldgen/placed_feature",
    "minecraft:worldgen/structure",
    "minecraft:worldgen/structure_set",
    "minecraft:worldgen/processor_list",
    "minecraft:worldgen/template_pool",
    "minecraft:worldgen/noise_settings",
    "minecraft:worldgen/noise",
    "minecraft:worldgen/density_function",
    "minecraft:worldgen/material_rule",
    "minecraft:worldgen/material_condition",
    "minecraft:worldgen/world_preset",
    "minecraft:worldgen/flat_level_generator_preset",
    "minecraft:trim_pattern",
    "minecraft:trim_material",
    "minecraft:trial_spawner",
    "minecraft:wolf_variant",
    "minecraft:wolf_sound_variant",
    "minecraft:pig_variant",
    "minecraft:pig_sound_variant",
    "minecraft:frog_variant",
    "minecraft:cat_variant",
    "minecraft:cat_sound_variant",
    "minecraft:cow_variant",
    "minecraft:cow_sound_variant",
    "minecraft:sulfur_cube_archetype",
    "minecraft:chicken_variant",
    "minecraft:chicken_sound_variant",
    "minecraft:zombie_nautilus_variant",
    "minecraft:painting_variant",
    "minecraft:damage_type",
    "minecraft:worldgen/multi_noise_biome_source_parameter_list",
    "minecraft:banner_pattern",
    "minecraft:enchantment",
    "minecraft:enchantment_provider",
    "minecraft:jukebox_song",
    "minecraft:instrument",
    "minecraft:test_environment",
    "minecraft:test_instance",
    "minecraft:dialog",
    "minecraft:world_clock",
    "minecraft:timeline",
    "minecraft:villager_trade",
    "minecraft:trade_set",
    "minecraft:decorated_pot_pattern",
    "minecraft:block_transformer",
    "minecraft:worldgen/block_state_provider",
    "minecraft:dimension",
    "minecraft:loot_table",
    "minecraft:predicate",
    "minecraft:context_float_provider",
    "minecraft:context_int_provider",
    "minecraft:item_modifier",
    "minecraft:slot_source",
    "minecraft:advancement",
    "minecraft:recipe",
];

impl DatapackRegistriesRegistrar for PartRegistrar {
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        mut fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError> {
        let result = (|| {
            let side = mod_vocab::__private::parts_installation::loaded_mods()
                .get(self.mod_id)
                .map(|package| package.side());
            if side == Some(crate::Side::Client) {
                return Err(DatapackRegistrationError::WrongSide);
            }
            let key = qualified(self.mod_id, path);
            if !mod_vocab::__private::parts_installation::valid_datapack_id(&key) {
                return Err(DatapackRegistrationError::InvalidKey);
            }
            // A server-only declarer is checked by the package join-content rule;
            // a both-side declarer may not add a required registry for a server-only supplier.
            if sync == DatapackRegistrySync::Required
                && side != Some(crate::Side::Server)
                && self.declaration_side_not_both(&key)
            {
                return Err(DatapackRegistrationError::WrongSide);
            }
            if BUILTIN_DATAPACK_REGISTRIES.contains(&key.as_str())
                || self.has_builtin_registry(&key)
                || self
                    .datapack_registries
                    .iter()
                    .any(|declared| declared.key == key)
            {
                return Err(DatapackRegistrationError::Duplicate);
            }
            mod_vocab::__private::parts_installation::normalize_datapack_defaults(&mut fields)?;
            mod_vocab::__private::parts_installation::validate_datapack_schema(&fields)?;
            self.datapack_registries.push(DatapackRegistryDeclaration {
                mod_id: self.mod_id.to_owned(),
                key,
                fields,
                sync,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} datapack registry {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatapackFieldKind;

    fn power() -> Vec<DatapackField> {
        vec![DatapackField::required(
            "power",
            DatapackFieldKind::Int { min: 0, max: 100 },
        )]
    }

    /// Bare paths qualify with the declaring mod; valid explicit namespaces (including
    /// non-colliding `minecraft:` keys) stand under the declaring mod while malformed keys,
    /// duplicates, built-in registries and bad schemas refuse, naming the mod.
    #[test]
    pub(crate) fn datapack_registry_declarations_validate() {
        use DatapackRegistrationError as E;
        let data = mod_vocab::__private::RegistrationData::new(
            [("minecraft:block".into(), vec!["minecraft:stone".into()])].into(),
            Default::default(),
            crate::InstanceDirectory::from_path(std::path::Path::new("unused")).unwrap(),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );
        crate::load_state::with_test_registration_data(data, || {
            let mut registrar = PartRegistrar::new("example");
            let sync = DatapackRegistrySync::Required;
            registrar
                .declare_datapack_registry("spell", sync, power())
                .unwrap();
            assert_eq!(
                registrar.datapack_registries[0].key, "example:spell",
                "a bare path is in the declaring mod's namespace"
            );
            assert_eq!(registrar.datapack_registries[0].mod_id, "example");
            registrar
                .declare_datapack_registry("addon:spell", sync, power())
                .unwrap();
            assert_eq!(registrar.datapack_registries[1].key, "addon:spell");
            assert_eq!(registrar.datapack_registries[1].mod_id, "example");
            registrar
                .declare_datapack_registry("minecraft:spell", sync, power())
                .unwrap();
            assert_eq!(registrar.datapack_registries[2].key, "minecraft:spell");
            assert_eq!(registrar.datapack_registries[2].mod_id, "example");
            assert_eq!(
                registrar.declare_datapack_registry("example:spell", sync, power()),
                Err(E::Duplicate)
            );
            assert_eq!(
                registrar.declare_datapack_registry("addon:spell", sync, power()),
                Err(E::Duplicate)
            );
            assert_eq!(
                registrar.declare_datapack_registry("minecraft:damage_type", sync, power()),
                Err(E::Duplicate)
            );
            assert_eq!(
                registrar.declare_datapack_registry("minecraft:block", sync, power()),
                Err(E::Duplicate)
            );
            assert_eq!(
                registrar.declare_datapack_registry("Not A Key", sync, power()),
                Err(E::InvalidKey)
            );
            assert_eq!(
                registrar.declare_datapack_registry("", sync, power()),
                Err(E::InvalidKey)
            );
            let mut bad = power();
            bad.push(bad[0].clone());
            assert_eq!(
                registrar.declare_datapack_registry("other", sync, bad),
                Err(E::InvalidFieldName)
            );
            // A refused schema leaves no half-declared registry behind.
            assert_eq!(registrar.datapack_registries.len(), 3);
            assert_eq!(registrar.refused.len(), 7);
            assert!(
                registrar
                    .refused
                    .iter()
                    .all(|refusal| refusal.contains("mod example"))
            );
        });
    }

    /// Through the registrar: an id default is normalised inside a record and as a map key as
    /// at the top, and a default map naming one id twice is refused as a duplicate.
    #[test]
    fn nested_id_defaults_normalise_through_the_registrar() {
        use crate::{DatapackKeyKind, DatapackValue};
        use DatapackRegistrationError as E;
        let mut registrar = PartRegistrar::new("example");
        let sync = DatapackRegistrySync::Required;
        let by_mob = |pairs: Vec<(&str, &str)>| {
            DatapackField::with_default(
                "by_mob",
                DatapackFieldKind::Map {
                    key: DatapackKeyKind::Identifier,
                    value: Box::new(DatapackFieldKind::Identifier),
                },
                DatapackValue::Map(
                    pairs
                        .into_iter()
                        .map(|(k, v)| (k.to_owned(), DatapackValue::Text(v.to_owned())))
                        .collect(),
                ),
            )
        };
        let record = |fields| {
            vec![DatapackField::required(
                "drop",
                DatapackFieldKind::Record { fields },
            )]
        };
        registrar
            .declare_datapack_registry(
                "drops",
                sync,
                record(vec![
                    DatapackField::with_default(
                        "item",
                        DatapackFieldKind::Identifier,
                        DatapackValue::Text("stone".into()),
                    ),
                    by_mob(vec![("zombie", "rotten_flesh")]),
                ]),
            )
            .unwrap();
        let DatapackFieldKind::Record { fields } = &registrar.datapack_registries[0].fields[0].kind
        else {
            panic!("a record");
        };
        assert_eq!(
            fields[0].default,
            Some(DatapackValue::Text("minecraft:stone".into()))
        );
        assert_eq!(
            fields[1].default,
            Some(DatapackValue::Map(vec![(
                "minecraft:zombie".into(),
                DatapackValue::Text("minecraft:rotten_flesh".into())
            )]))
        );
        assert_eq!(
            registrar.declare_datapack_registry(
                "twice",
                sync,
                record(vec![by_mob(vec![
                    ("zombie", "a"),
                    ("minecraft:zombie", "b")
                ])]),
            ),
            Err(E::DuplicateKey)
        );
        assert_eq!(registrar.datapack_registries.len(), 1);
    }

    #[test]
    fn server_package_requires_clients_only_for_required_registries() {
        use DatapackRegistrySync::{Optional, Required, ServerOnly};

        let package = crate::PackageInfo::new(
            "example".into(),
            "1".into(),
            crate::PackageKind::Code,
            crate::Side::Server,
            vec!["example".into()],
            vec![],
        );
        crate::load_state::with_test_package_inventory(vec![package], || {
            crate::testing::with_mods(Default::default(), || {
                for (required, reason) in [
                    (false, None),
                    (
                        true,
                        Some("declares the required datapack registry example:shared"),
                    ),
                ] {
                    let mut registrar = PartRegistrar::new("example");
                    registrar
                        .declare_datapack_registry("local", ServerOnly, power())
                        .unwrap();
                    registrar
                        .declare_datapack_registry("offered", Optional, power())
                        .unwrap();
                    if required {
                        registrar
                            .declare_datapack_registry("shared", Required, power())
                            .unwrap();
                    }
                    // No payload channels are registered in either installation.
                    super::super::install_registrar(registrar, &crate::parts::tests::rule_schema())
                        .unwrap();
                    assert_eq!(
                        crate::__private::client_requirement("example").as_deref(),
                        reason
                    );
                    assert_eq!(crate::__private::client_requirement("unrelated"), None);
                }
            });
        });
    }

    #[test]
    fn datapack_declarations_round_trip_the_wire() {
        use crate::wire::{Reader, Wire, Writer};
        let declaration = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: power(),
            sync: DatapackRegistrySync::Optional,
        };
        let mut writer = Writer::new();
        declaration.put(&mut writer);
        let bytes = writer.into_bytes();
        assert_eq!(
            DatapackRegistryDeclaration::take(&mut Reader::new(&bytes)).unwrap(),
            declaration
        );
    }

    #[test]
    fn served_by_datapack_registry_delegates_and_enforces_supplier_side() {
        use DatapackRegistrationError as E;
        let packages = vec![
            crate::PackageInfo::new(
                "host".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["host".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "other".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["other".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Both,
                vec!["runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
            crate::PackageInfo::new(
                "srv_runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Server,
                vec!["srv_runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
        ];
        crate::load_state::with_test_package_inventory(packages, || {
            let sync = DatapackRegistrySync::Required;
            let mut registrar = PartRegistrar::new("host");
            registrar
                .declare_datapack_registry("runes:spell", sync, power())
                .unwrap();
            assert_eq!(registrar.datapack_registries[0].mod_id, "host");
            assert_eq!(registrar.datapack_registries[0].key, "runes:spell");
            assert_eq!(
                registrar.datapack_registries[0].entry_file("addon:fire"),
                Some("data/addon/runes/spell/fire.json".to_owned())
            );
            assert_eq!(
                registrar.declare_datapack_registry("srv_runes:spell", sync, power()),
                Err(E::WrongSide)
            );
            assert_eq!(registrar.datapack_registries.len(), 1);
        });
    }
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn registered_identifier_defaults_match_explicit_values() {
        for kind in [
            crate::DatapackFieldKind::Identifier,
            crate::DatapackFieldKind::Reference {
                registry: "minecraft:item".into(),
            },
        ] {
            let mut registrar = PartRegistrar::new("defaults");
            registrar
                .declare_datapack_registry(
                    "entry",
                    DatapackRegistrySync::default(),
                    vec![DatapackField::with_default(
                        "id",
                        kind,
                        crate::DatapackValue::Text("ice".into()),
                    )],
                )
                .unwrap();
            let declaration = &registrar.datapack_registries[0];
            assert_eq!(
                declaration.decode_entry("{}").unwrap(),
                declaration.decode_entry(r#"{"id":"ice"}"#).unwrap()
            );
        }
    }
}
