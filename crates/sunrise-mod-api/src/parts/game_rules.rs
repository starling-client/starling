//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    pub(crate) fn game_rule_id(
        &self,
        id: &str,
    ) -> Result<(String, crate::Identifier), crate::GameRuleError> {
        let id = qualified(self.mod_id, id);
        let parsed =
            crate::Identifier::parse_namespaced(&id).ok_or(crate::GameRuleError::InvalidId)?;
        Ok((id, parsed))
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;
    #[test]
    pub(crate) fn cross_mod_categories_resolve_after_all_registration() {
        use super::Registrar;
        struct Rule;
        #[mod_api::__subscriptions]
        impl Hooks for Rule {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_bool("active", "later:machines", true).unwrap();
            }
        }
        struct Category;
        #[mod_api::__subscriptions]
        impl Hooks for Category {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_category("machines").unwrap();
            }
        }
        let mods = [
            ModEntry::new("first", Box::new(Rule)),
            ModEntry::new("later", Box::new(Category)),
        ];
        super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        assert!(
            super::game_rules()
                .iter()
                .any(|rule| rule.name == "first:active")
        );
    }
    #[test]
    pub(crate) fn unknown_category_reports_rule_category_and_owner_without_installing() {
        use super::Registrar;
        struct Rule;
        #[mod_api::__subscriptions]
        impl Hooks for Rule {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_bool("active", "missing:machines", true)
                    .unwrap();
            }
        }
        let mods = [
            ModEntry::new("first", Box::new(Rule)),
            ModEntry::new("second", Box::new(Rule)),
        ];
        let errors = super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("first:active"));
        assert!(errors[1].contains("second:active"));
        assert!(
            errors
                .iter()
                .all(|error| error.contains("missing:machines"))
        );
        assert!(
            !super::game_rules()
                .iter()
                .any(|rule| rule.name == "first:active")
        );
    }

    #[test]
    pub(crate) fn bool_rule_declarations_validate_slots_and_fail_atomically() {
        let mut refused = super::PartRegistrar::new("example");
        refused
            .game_rule_bool("active", "missing:machines", true)
            .unwrap();
        assert_eq!(
            refused.game_rule_bool("foreign:Bad Name", "example:machines", true),
            Err(crate::GameRuleError::InvalidId)
        );
        let errors = super::install_registrar(refused, &crate::parts::tests::rule_schema())
            .unwrap_err()
            .join(" ");
        assert!(
            errors.contains("InvalidId")
                && errors.contains("example:active")
                && errors.contains("missing:machines")
        );
        let mut good = super::PartRegistrar::new("example");
        let _key = good
            .game_rule_bool("active", "example:machines", true)
            .unwrap();
        good.game_rule_category("machines").unwrap();
        assert_eq!(
            good.game_rule_bool("active", "example:machines", true),
            Err(crate::GameRuleError::Duplicate)
        );
        assert!(super::install_registrar(good, &crate::parts::tests::rule_schema()).is_err());
        let mut good = super::PartRegistrar::new("example");
        let key = good
            .game_rule_bool("active", "example:machines", true)
            .unwrap();
        good.game_rule_category("machines").unwrap();
        super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
        assert!(
            mod_vocab::__private::parts_installation::validate_bool_game_rule_key(
                mod_vocab::__private::parts_installation::bool_game_rule_key_slot(key)
            )
            .is_ok()
        );
        assert!(
            mod_vocab::__private::parts_installation::validate_bool_game_rule_key(
                mod_vocab::__private::parts_installation::bool_game_rule_key_slot(key) + 1
            )
            .is_err()
        );
        assert!(super::game_rule_bool_slot(key).is_some());
    }

    #[test]
    pub(crate) fn int_rule_declarations_check_their_range_and_install_with_slots() {
        let mut refused = super::PartRegistrar::new("example");
        refused.game_rule_category("machines").unwrap();
        assert_eq!(
            refused.game_rule_int("cap", "example:machines", 70, 0, 64),
            Err(crate::GameRuleError::InvalidRange)
        );
        assert_eq!(
            refused.game_rule_int("flipped", "example:machines", 0, 1, -1),
            Err(crate::GameRuleError::InvalidRange)
        );
        let errors = super::install_registrar(refused, &crate::parts::tests::rule_schema())
            .unwrap_err()
            .join(" ");
        assert!(errors.contains("InvalidRange") && errors.contains("flipped"));
        let mut good = super::PartRegistrar::new("example");
        good.game_rule_category("machines").unwrap();
        good.game_rule_int("cap", "example:machines", 8, 0, 64)
            .unwrap();
        assert_eq!(
            good.game_rule_int("cap", "example:machines", 8, 0, 64),
            Err(crate::GameRuleError::Duplicate)
        );
        let mut good = super::PartRegistrar::new("example");
        good.game_rule_category("machines").unwrap();
        let key = good
            .game_rule_int("cap", "example:machines", 8, 0, 64)
            .unwrap();
        super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
        let slot = mod_vocab::__private::parts_installation::int_game_rule_key_slot(key);
        assert!(mod_vocab::__private::parts_installation::validate_int_game_rule_key(slot).is_ok());
        assert!(super::game_rule_int_slot(key).is_some());
        assert!(
            super::game_rules()
                .iter()
                .any(|rule| rule.name == "example:cap"
                    && rule.default
                        == crate::GameRuleDefault::Int {
                            default: 8,
                            min: 0,
                            max: 64
                        })
        );
    }

    #[test]
    pub(crate) fn category_registration_refuses_ignored_errors_and_installs_snapshot() {
        let mut invalid = super::PartRegistrar::new("example");
        assert_eq!(
            invalid.game_rule_category("bad path"),
            Err(crate::GameRuleError::InvalidId)
        );
        assert!(super::install_registrar(invalid, &crate::parts::tests::rule_schema()).is_err());
        let mut good = super::PartRegistrar::new("example");
        assert_eq!(good.game_rule_category("machines"), Ok(()));
        assert!(super::install_registrar(good, &crate::parts::tests::rule_schema()).is_ok());
        assert!(super::game_rule_categories().contains(&"example:machines".to_owned()));
        let mut duplicate = super::PartRegistrar::new("example");
        duplicate.game_rule_category("machines").unwrap();
        assert_eq!(
            duplicate.game_rule_category("example:machines"),
            Err(crate::GameRuleError::Duplicate)
        );
        assert!(super::install_registrar(duplicate, &crate::parts::tests::rule_schema()).is_err());
    }

    #[test]
    pub(crate) fn cross_namespace_and_non_colliding_minecraft_categories_and_rules_succeed() {
        use super::Registrar;
        struct CrossNs;
        #[mod_api::__subscriptions]
        impl Hooks for CrossNs {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_category("foreign:machines").unwrap();
                r.game_rule_category("minecraft:custom_cat").unwrap();
                r.game_rule_bool("foreign:active", "foreign:machines", true)
                    .unwrap();
                r.game_rule_int("minecraft:custom_limit", "minecraft:custom_cat", 4, 0, 16)
                    .unwrap();
            }
        }
        let mods = [ModEntry::new("porter", Box::new(CrossNs))];
        super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        assert!(super::game_rule_categories().contains(&"foreign:machines".to_owned()));
        assert!(super::game_rule_categories().contains(&"minecraft:custom_cat".to_owned()));
        assert!(
            super::game_rules()
                .iter()
                .any(|rule| rule.name == "foreign:active" && rule.category == "foreign:machines")
        );
        assert!(super::game_rules().iter().any(|rule| {
            rule.name == "minecraft:custom_limit" && rule.category == "minecraft:custom_cat"
        }));
    }

    #[test]
    pub(crate) fn builtin_category_and_rule_collisions_refuse_naming_declaring_mod() {
        use super::Registrar;
        struct Collides;
        #[mod_api::__subscriptions]
        impl Hooks for Collides {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(
                    r.game_rule_category("minecraft:drops"),
                    Err(crate::GameRuleError::Duplicate)
                );
                assert_eq!(
                    r.game_rule_bool("minecraft:keep_inventory", "minecraft:drops", true),
                    Err(crate::GameRuleError::Duplicate)
                );
                assert_eq!(
                    r.game_rule_int("minecraft:random_tick_speed", "minecraft:drops", 3, 0, 100),
                    Err(crate::GameRuleError::Duplicate)
                );
            }
        }
        let schema = mod_vocab::__private::RuleSchema {
            category: &|id| id == "minecraft:drops",
            position: &|id| match id {
                "minecraft:keep_inventory" => Some(0),
                "minecraft:random_tick_speed" => Some(1),
                _ => None,
            },
            rule_count: 2,
        };
        let mods = [ModEntry::new("porter", Box::new(Collides))];
        let registrar =
            super::with_active_rule_schema(&schema, || super::register_all(&mods, |_| false));
        assert!(registrar.game_rule_categories.is_empty());
        assert!(registrar.game_rules.is_empty());
        assert!(
            registrar
                .registry_keys
                .borrow()
                .game_rules
                .entries
                .is_empty()
        );
        let errors = super::install_registrar(registrar, &schema).unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("mod porter") && errors[0].contains("Duplicate"));
        assert!(errors[1].contains("mod porter") && errors[1].contains("Duplicate"));
        assert!(errors[2].contains("mod porter") && errors[2].contains("Duplicate"));
    }

    #[test]
    pub(crate) fn cross_mod_duplicate_category_and_rule_ids_refuse() {
        use super::Registrar;
        struct First;
        #[mod_api::__subscriptions]
        impl Hooks for First {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_category("shared:machines").unwrap();
                r.game_rule_bool("shared:active", "shared:machines", true)
                    .unwrap();
            }
        }
        struct Second;
        #[mod_api::__subscriptions]
        impl Hooks for Second {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(
                    r.game_rule_category("shared:machines"),
                    Err(crate::GameRuleError::Duplicate)
                );
                assert_eq!(
                    r.game_rule_bool("shared:active", "shared:machines", false),
                    Err(crate::GameRuleError::Duplicate)
                );
            }
        }
        let mods = [
            ModEntry::new("first", Box::new(First)),
            ModEntry::new("second", Box::new(Second)),
        ];
        let errors = super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("mod second") && errors[0].contains("Duplicate"));
        assert!(errors[1].contains("mod second") && errors[1].contains("Duplicate"));
    }
}

impl GameRulesRegistrar for PartRegistrar {
    fn game_rule_category(&mut self, id: &str) -> Result<(), crate::GameRuleError> {
        let result = (|| {
            let (id, _) = self.game_rule_id(id)?;
            if super::active_rule_schema_has_category(&id)
                || self
                    .game_rule_categories
                    .iter()
                    .any(|(_, existing)| existing == &id)
            {
                return Err(crate::GameRuleError::Duplicate);
            }
            self.game_rule_categories.push((self.mod_id, id));
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} category {id:?}: {error:?}", self.mod_id));
        }
        result
    }

    fn game_rule_bool(
        &mut self,
        id: &str,
        category: &str,
        default: bool,
    ) -> Result<crate::BoolGameRuleKey, crate::GameRuleError> {
        let result = (|| {
            let (id, parsed) = self.game_rule_id(id)?;
            if super::active_rule_schema_has_rule(&id)
                || self.game_rules.iter().any(|(_, r)| r.name == id)
            {
                return Err(crate::GameRuleError::Duplicate);
            }
            let slot = self
                .registry_keys
                .borrow_mut()
                .game_rules
                .request_identifier(self.mod_id, &parsed)
                .map_err(|_| crate::GameRuleError::InvalidId)?;
            self.game_rules.push((
                self.mod_id,
                mod_vocab::__private::parts_installation::game_rule_declaration(
                    id,
                    category.into(),
                    crate::GameRuleDefault::Bool(default),
                ),
            ));
            Ok(mod_vocab::__private::parts_installation::bool_game_rule_key(slot))
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} rule {id:?}: {error:?}", self.mod_id));
        }
        result
    }

    fn game_rule_int(
        &mut self,
        id: &str,
        category: &str,
        default: i32,
        min: i32,
        max: i32,
    ) -> Result<crate::IntGameRuleKey, crate::GameRuleError> {
        let result = (|| {
            let (id, parsed) = self.game_rule_id(id)?;
            if super::active_rule_schema_has_rule(&id)
                || self.game_rules.iter().any(|(_, r)| r.name == id)
            {
                return Err(crate::GameRuleError::Duplicate);
            }
            if !(min..=max).contains(&default) {
                return Err(crate::GameRuleError::InvalidRange);
            }
            let slot = self
                .registry_keys
                .borrow_mut()
                .game_rules
                .request_identifier(self.mod_id, &parsed)
                .map_err(|_| crate::GameRuleError::InvalidId)?;
            self.game_rules.push((
                self.mod_id,
                mod_vocab::__private::parts_installation::game_rule_declaration(
                    id,
                    category.into(),
                    crate::GameRuleDefault::Int { default, min, max },
                ),
            ));
            Ok(mod_vocab::__private::parts_installation::int_game_rule_key(
                slot,
            ))
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} int rule {id:?}: {error:?}", self.mod_id));
        }
        result
    }
}
