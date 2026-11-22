//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl DatapackRegistriesRegistrar for PartRegistrar {
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        mut fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::loaded_mods()
                .get(self.mod_id)
                .is_some_and(|package| package.side() != crate::Side::Both)
            {
                return Err(DatapackRegistrationError::WrongSide);
            }
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            let key = format!("{namespace}:{name}");
            if namespace == "minecraft"
                || !owns_namespace(self.mod_id, namespace)
                || !mod_vocab::__private::parts_installation::valid_datapack_id(&key)
            {
                return Err(DatapackRegistrationError::InvalidKey);
            }
            if self.declaration_side_not_both(&key) {
                return Err(DatapackRegistrationError::WrongSide);
            }
            if self
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

    /// A key in the mod's namespace stands; foreign, vanilla and malformed keys, a
    /// duplicate and a bad schema refuse, naming the mod.
    #[test]
    pub(crate) fn datapack_registry_declarations_validate() {
        use DatapackRegistrationError as E;
        let mut registrar = PartRegistrar::new("example");
        let sync = DatapackRegistrySync::Required;
        registrar
            .declare_datapack_registry("spell", sync, power())
            .unwrap();
        assert_eq!(
            registrar.datapack_registries[0].key, "example:spell",
            "a bare path is in the declaring mod's namespace"
        );
        assert_eq!(
            registrar.declare_datapack_registry("example:spell", sync, power()),
            Err(E::Duplicate)
        );
        assert_eq!(
            registrar.declare_datapack_registry("minecraft:spell", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("addon:spell", sync, power()),
            Err(E::InvalidKey)
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
        assert_eq!(registrar.datapack_registries.len(), 1);
        assert_eq!(registrar.refused.len(), 6);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod example"))
        );
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
            assert_eq!(
                registrar.declare_datapack_registry("minecraft:spell", sync, power()),
                Err(E::InvalidKey)
            );
            registrar.mod_id = "other";
            assert_eq!(
                registrar.declare_datapack_registry("runes:other", sync, power()),
                Err(E::InvalidKey)
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
