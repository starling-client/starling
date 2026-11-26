//! Validation for the shared load-owned custom statistic declarations.
use super::*;

impl CustomStatsRegistrar for PartRegistrar {
    fn register_custom_stat(&mut self, id: &str) -> Result<(), crate::DeclarationError> {
        use crate::DeclarationError;
        let id = qualified(self.mod_id, id);
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed()
                && !REGISTERING_KEYS.with_borrow(Option::is_some)
            {
                return Err(DeclarationError::RegistrationClosed { id });
            }
            if crate::Identifier::parse_namespaced(&id).is_none() {
                return Err(DeclarationError::InvalidId { id });
            }
            if self.custom_stats.iter().any(|declared| declared == &id) {
                return Err(DeclarationError::Rejected {
                    id,
                    reason: "duplicate custom statistic".into(),
                });
            }
            if self.has_builtin_entry("minecraft:custom_stat", &id) {
                return Err(DeclarationError::Rejected {
                    id,
                    reason: "built-in custom statistic".into(),
                });
            }
            self.custom_stats.push(id);
            self.custom_stat_owners.push(self.mod_id);
            Ok(())
        })();
        result.inspect_err(|error| {
            self.refused.push(format!(
                "mod {} custom statistic declaration refused: {error}",
                self.mod_id
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_stat_registration_accepts_explicit_namespaces_and_refuses_invalid_entries() {
        let mut registrar = PartRegistrar::new("demo");
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(registrar.custom_stats, ["demo:steps"]);
        assert!(registrar.register_custom_stat("demo:steps").is_err());
        for id in ["demo:", ":steps", "demo:Steps", "demo:a:b"] {
            assert_eq!(
                registrar.register_custom_stat(id),
                Err(crate::DeclarationError::InvalidId { id: id.into() })
            );
        }
        registrar.register_custom_stat("minecraft:steps").unwrap();
        registrar.register_custom_stat("other:steps").unwrap();
        assert_eq!(
            registrar.custom_stats,
            ["demo:steps", "minecraft:steps", "other:steps"]
        );
    }

    #[test]
    fn custom_stat_registration_refuses_a_builtin_key() {
        let data = mod_vocab::__private::RegistrationData::new(
            [(
                "minecraft:custom_stat".into(),
                vec!["minecraft:play_time".into()],
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
            let mut registrar = PartRegistrar::new("demo");
            assert_eq!(
                registrar.register_custom_stat("minecraft:play_time"),
                Err(crate::DeclarationError::Rejected {
                    id: "minecraft:play_time".into(),
                    reason: "built-in custom statistic".into(),
                })
            );
            assert!(registrar.custom_stats.is_empty());
        });
    }

    #[test]
    fn custom_stat_registration_refuses_reuse_across_mods() {
        let mut registrar = PartRegistrar::new("first");
        registrar.register_custom_stat("steps").unwrap();
        registrar.mod_id = "second";
        assert!(registrar.register_custom_stat("first:steps").is_err());
        registrar.register_custom_stat("first:other").unwrap();
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(
            registrar.custom_stats,
            ["first:steps", "first:other", "second:steps"]
        );
    }
}
