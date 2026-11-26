//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    /// Explicit namespaces succeed while a structure type twice, a built-in structure type and
    /// one without a name refuse at startup, naming the mod.
    #[test]
    pub(crate) fn duplicate_and_builtin_structure_types_refuse_registration() {
        struct Tower;
        impl mod_api::StructureType for Tower {
            fn find_generation_point(
                &self,
                _: &mut dyn mod_api::GenerationContext,
            ) -> Option<mod_api::FoundStructure> {
                None
            }
            fn load_piece(
                &self,
                _: &dyn mod_api::BlockMetadata,
                _: &mod_api::Compound,
            ) -> Result<std::sync::Arc<dyn mod_api::StructurePiece>, mod_api::StructureError>
            {
                Err(mod_api::StructureError::new(std::io::Error::other(
                    "fixture has no pieces",
                )))
            }
        }
        struct Valid;
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
                r.register_structure_type("other:tower", Tower);
                r.register_structure_type("minecraft:custom_tower", Tower);
            }
        }
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
                r.register_structure_type("tower", Tower);
                r.register_structure_type("minecraft:jigsaw", Tower);
                r.register_structure_type("", Tower);
            }
        }
        let data = mod_vocab::__private::RegistrationData::new(
            [(
                "minecraft:worldgen/structure_type".into(),
                vec!["minecraft:jigsaw".into()],
            )]
            .into(),
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
            crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Valid))])
                .unwrap();
            let types = crate::__private::structure_type_declarations();
            assert_eq!(
                types
                    .iter()
                    .map(|(id, owner, _)| (id.as_str(), *owner))
                    .collect::<Vec<_>>(),
                [
                    ("test:tower", "test"),
                    ("other:tower", "test"),
                    ("minecraft:custom_tower", "test")
                ]
            );

            let errors = install_parts(
                &[ModEntry::new("test", Box::new(Invalid))],
                &crate::parts::tests::rule_schema(),
                |_| false,
            )
            .unwrap_err();
            assert_eq!(errors.len(), 3, "{errors:?}");
            assert!(errors[0].contains("test registered structure type test:tower twice"));
            assert!(errors[1].contains("test registered built-in structure type minecraft:jigsaw"));
            assert!(errors[2].contains("invalid identifier \"test:\""));
        });
    }

    /// Explicit namespaces succeed. A structure processor type registered twice or without a name
    /// is refused at startup, naming the mod.
    #[test]
    pub(crate) fn duplicate_and_invalid_structure_processor_types_refuse_registration() {
        #[derive(Debug)]
        struct NoopProcessor;
        impl mod_api::StructureProcessor for NoopProcessor {}

        struct Weathering;
        impl mod_api::StructureProcessorType for Weathering {
            fn decode(
                &self,
                _: &dyn mod_api::ProcessorDecodeContext,
                _: &sunrise_core::nbt::Tag,
            ) -> Result<std::sync::Arc<dyn mod_api::StructureProcessor>, mod_api::StructureError>
            {
                Ok(std::sync::Arc::new(NoopProcessor))
            }
        }
        struct Valid;
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_processor_type("weathering", Weathering);
                r.register_structure_processor_type("other:weathering", Weathering);
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Valid))])
            .unwrap();
        let types = crate::__private::structure_processor_type_declarations();
        assert_eq!(
            types.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["test:weathering", "other:weathering"]
        );

        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_processor_type("weathering", Weathering);
                r.register_structure_processor_type("weathering", Weathering);
                r.register_structure_processor_type("", Weathering);
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("test registered structure processor type test:weathering twice")
        );
        assert!(errors[1].contains("invalid identifier \"test:\""));
    }

    /// Rule test and position rule test types registered twice or with an
    /// empty name are refused at startup, naming the mod.
    #[test]
    pub(crate) fn duplicate_and_invalid_rule_test_types_refuse_registration() {
        #[derive(Debug)]
        struct AlwaysRule;
        impl mod_api::RuleTest for AlwaysRule {
            fn test(
                &self,
                _: &dyn mod_api::RuleTestContext,
                _: mod_api::BlockState,
                _: [i32; 3],
                _: &mut dyn mod_api::RandomSource,
            ) -> bool {
                true
            }
        }
        struct AlwaysRuleType;
        impl mod_api::RuleTestType for AlwaysRuleType {
            fn decode(
                &self,
                _: &dyn mod_api::RuleTestDecodeContext,
                _: &sunrise_core::nbt::Tag,
            ) -> Result<std::sync::Arc<dyn mod_api::RuleTest>, mod_api::StructureError>
            {
                Ok(std::sync::Arc::new(AlwaysRule))
            }
        }
        #[derive(Debug)]
        struct AlwaysPos;
        impl mod_api::PosRuleTest for AlwaysPos {
            fn test(
                &self,
                _: &dyn mod_api::PosRuleTestContext,
                _: [i32; 3],
                _: [i32; 3],
                _: [i32; 3],
                _: &mut dyn mod_api::RandomSource,
            ) -> bool {
                true
            }
        }
        struct AlwaysPosType;
        impl mod_api::PosRuleTestType for AlwaysPosType {
            fn decode(
                &self,
                _: &dyn mod_api::PosRuleTestDecodeContext,
                _: &sunrise_core::nbt::Tag,
            ) -> Result<std::sync::Arc<dyn mod_api::PosRuleTest>, mod_api::StructureError>
            {
                Ok(std::sync::Arc::new(AlwaysPos))
            }
        }
        struct Valid;
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_rule_test_type("phase", AlwaysRuleType);
                r.register_pos_rule_test_type("y_band", AlwaysPosType);
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Valid))])
            .unwrap();
        let rules = crate::__private::rule_test_type_declarations();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].0, "test:phase");
        assert_eq!(rules[0].1, "test");
        let pos_rules = crate::__private::pos_rule_test_type_declarations();
        assert_eq!(pos_rules.len(), 1);
        assert_eq!(pos_rules[0].0, "test:y_band");
        assert_eq!(pos_rules[0].1, "test");

        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_rule_test_type("phase", AlwaysRuleType);
                r.register_rule_test_type("phase", AlwaysRuleType);
                r.register_rule_test_type("other:phase", AlwaysRuleType);
                r.register_rule_test_type("", AlwaysRuleType);
                r.register_pos_rule_test_type("y_band", AlwaysPosType);
                r.register_pos_rule_test_type("y_band", AlwaysPosType);
                r.register_pos_rule_test_type("other:y_band", AlwaysPosType);
                r.register_pos_rule_test_type("", AlwaysPosType);
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        // A code registry key may take any namespace (RegisterEvent.register), so
        // `other:phase` and `other:y_band` register.
        assert_eq!(errors.len(), 4);
        assert!(errors[0].contains("test registered rule test type test:phase twice"));
        assert!(errors[1].contains("invalid identifier \"test:\""));
        assert!(errors[2].contains("test registered position rule test type test:y_band twice"));
        assert!(errors[3].contains("invalid identifier \"test:\""));
    }
}

impl StructuresRegistrar for PartRegistrar {
    fn add_structure_type(&mut self, name: &str, ty: Arc<dyn crate::StructureType>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if self.structure_types.iter().any(|t| *t.id == *id) {
            Some(format!(
                "{} registered structure type {id} twice",
                self.mod_id
            ))
        } else if self.has_builtin_entry("minecraft:worldgen/structure_type", &id) {
            Some(format!(
                "{} registered built-in structure type {id}",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.structure_types.push(StructureTypeInfo {
            id: id.into(),
            mod_id: self.mod_id,
            ty,
        });
    }
    fn add_pool_element_type(&mut self, name: &str, ty: Arc<dyn crate::PoolElementType>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        if self.pool_element_types.iter().any(|t| *t.id == *id) {
            self.refused.push(format!(
                "{} registered pool element type {id} twice",
                self.mod_id
            ));
            return;
        }
        self.pool_element_types.push(PoolElementTypeInfo {
            id: id.into(),
            mod_id: self.mod_id,
            ty,
        });
    }
    fn add_structure_processor_type(
        &mut self,
        name: &str,
        ty: Arc<dyn crate::StructureProcessorType>,
    ) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        if self.structure_processor_types.iter().any(|t| *t.id == *id) {
            self.refused.push(format!(
                "{} registered structure processor type {id} twice",
                self.mod_id
            ));
            return;
        }
        self.structure_processor_types
            .push(StructureProcessorTypeInfo { id: id.into(), ty });
    }
    fn add_rule_test_type(&mut self, name: &str, ty: Arc<dyn crate::RuleTestType>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        if self.rule_test_types.iter().any(|t| *t.id == *id) {
            self.refused.push(format!(
                "{} registered rule test type {id} twice",
                self.mod_id
            ));
            return;
        }
        self.rule_test_types.push(RuleTestTypeInfo {
            id: id.into(),
            mod_id: self.mod_id,
            ty,
        });
    }
    fn add_pos_rule_test_type(&mut self, name: &str, ty: Arc<dyn crate::PosRuleTestType>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        if self.pos_rule_test_types.iter().any(|t| *t.id == *id) {
            self.refused.push(format!(
                "{} registered position rule test type {id} twice",
                self.mod_id
            ));
            return;
        }
        self.pos_rule_test_types.push(PosRuleTestTypeInfo {
            id: id.into(),
            mod_id: self.mod_id,
            ty,
        });
    }
}
