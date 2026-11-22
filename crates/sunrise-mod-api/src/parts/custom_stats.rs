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
            let Some(identifier) = crate::Identifier::parse_namespaced(&id) else {
                return Err(DeclarationError::InvalidId { id });
            };
            if self.custom_stats.iter().any(|declared| declared == &id) {
                return Err(DeclarationError::Rejected {
                    id,
                    reason: "duplicate custom statistic".into(),
                });
            }
            let namespace = identifier.namespace();
            if namespace == "minecraft" || !owns_namespace(self.mod_id, namespace) {
                return Err(DeclarationError::UnownedNamespace {
                    id,
                    namespace: namespace.to_owned(),
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
    fn custom_stat_registration_qualifies_owned_ids_and_refuses_invalid_entries() {
        let mut registrar = PartRegistrar::new("demo");
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(registrar.custom_stats, ["demo:steps"]);
        assert!(registrar.register_custom_stat("demo:steps").is_err());
        for id in [
            "demo:",
            ":steps",
            "demo:Steps",
            "minecraft:steps",
            "other:steps",
            "demo:a:b",
        ] {
            assert!(registrar.register_custom_stat(id).is_err(), "{id}");
        }
        assert_eq!(registrar.custom_stats, ["demo:steps"]);
    }

    #[test]
    fn custom_stat_registration_refuses_reuse_across_mods() {
        let mut registrar = PartRegistrar::new("first");
        registrar.register_custom_stat("steps").unwrap();
        registrar.mod_id = "second";
        assert!(registrar.register_custom_stat("first:steps").is_err());
        assert_eq!(
            registrar.register_custom_stat("first:other"),
            Err(crate::DeclarationError::UnownedNamespace {
                id: "first:other".into(),
                namespace: "first".into()
            }),
        );
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(registrar.custom_stats, ["first:steps", "second:steps"]);
    }
}
