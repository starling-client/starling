//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    #[cfg(test)]
    struct Tree;

    #[cfg(test)]
    impl mod_api::FeatureType for Tree {
        fn place(
            &self,
            _: &mut dyn mod_api::FeatureContext,
            _: [i32; 3],
        ) -> Result<bool, mod_api::FeatureError> {
            Ok(false)
        }
    }

    /// A feature type twice, one in an unowned namespace, one without a name, one taking a
    /// vanilla feature type's id, one with an invalid schema and one with a reference field:
    /// every invalid registration is refused at startup, naming the mod.
    #[test]
    pub(crate) fn invalid_feature_types_refuse_registration() {
        use mod_api::{DatapackField, DatapackFieldKind, DatapackValue};
        let height = || {
            vec![DatapackField::with_default(
                "height",
                DatapackFieldKind::Int { min: 1, max: 32 },
                DatapackValue::Int(4),
            )]
        };
        struct Valid(Vec<DatapackField>);
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_feature_type("tree", self.0.clone(), Tree);
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new(
            "test",
            Box::new(Valid(height())),
        )])
        .unwrap();
        let types = crate::__private::feature_type_declarations();
        assert_eq!(
            types
                .iter()
                .map(|(id, schema, _)| (id.as_str(), schema.len()))
                .collect::<Vec<_>>(),
            [("test:tree", 1)]
        );

        struct Invalid(Vec<DatapackField>);
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_feature_type("tree", self.0.clone(), Tree);
                r.register_feature_type("tree", self.0.clone(), Tree);
                r.register_feature_type("other:tree", vec![], Tree);
                r.register_feature_type("", vec![], Tree);
                r.register_feature_type("minecraft:tree", vec![], Tree);
                let twice = [self.0.clone(), self.0.clone()].concat();
                r.register_feature_type("doubled", twice, Tree);
                r.register_feature_type(
                    "linked",
                    vec![DatapackField::required(
                        "next",
                        DatapackFieldKind::Reference {
                            registry: "test:spell".into(),
                        },
                    )],
                    Tree,
                );
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid(height())))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 6, "{errors:?}");
        assert!(errors[0].contains("test registered feature type test:tree twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("invalid identifier \"test:\""));
        assert!(errors[3].contains("namespace"), "{}", errors[3]);
        assert!(errors[4].contains("test:doubled: invalid config schema: InvalidFieldName"));
        assert!(errors[5].contains("test:linked: config field \"next\" is a Reference"));
    }

    #[test]
    fn a_nested_config_is_refused_without_adding_the_feature() {
        use mod_api::{DatapackField, DatapackFieldKind, DatapackKeyKind};
        let mut registrar = PartRegistrar::new("test");
        registrar.add_feature_type(
            "nested",
            vec![DatapackField::required(
                "by_spell",
                DatapackFieldKind::Map {
                    key: DatapackKeyKind::Reference {
                        registry: "test:spell".into(),
                    },
                    value: Box::new(DatapackFieldKind::Bool),
                },
            )],
            Arc::new(Tree),
        );
        assert_eq!(registrar.refused.len(), 1);
        assert!(registrar.feature_types.is_empty());
    }
}

impl FeaturesRegistrar for PartRegistrar {
    fn add_feature_type(
        &mut self,
        name: &str,
        mut config: Vec<crate::DatapackField>,
        ty: Arc<dyn crate::FeatureType>,
    ) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refuse =
            |reason: String| format!("{} registered feature type {id}: {reason}", self.mod_id);
        let refusal = if self.feature_types.iter().any(|t| *t.id == *id) {
            Some(format!(
                "{} registered feature type {id} twice",
                self.mod_id
            ))
        } else if let Some(field) = config
            .iter()
            .find(|field| matches!(field.kind, crate::DatapackFieldKind::Reference { .. }))
        {
            Some(refuse(format!(
                "config field {:?} is a Reference, which feature configs don't resolve; use Identifier",
                field.name
            )))
        } else if let Some(field) = config.iter().find(|field| {
            matches!(
                field.kind,
                crate::DatapackFieldKind::List { .. }
                    | crate::DatapackFieldKind::Map { .. }
                    | crate::DatapackFieldKind::Record { .. }
                    | crate::DatapackFieldKind::Optional { .. }
            )
        }) {
            // A feature config is a flat record of scalars: nested kinds (and the references
            // they could hide, a reference map key's included) are a datapack registry's.
            Some(refuse(format!(
                "config field {:?} is nested, and a feature config is a flat record of scalars",
                field.name
            )))
        } else {
            mod_vocab::__private::parts_installation::normalize_datapack_defaults(&mut config)
                .and_then(|()| {
                    mod_vocab::__private::parts_installation::validate_datapack_schema(&config)
                })
                .err()
                .map(|error| refuse(format!("invalid config schema: {error:?}")))
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.feature_types.push(FeatureTypeInfo {
            id: id.into(),
            config,
            ty,
        });
    }
}
