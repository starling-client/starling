//! Concrete authoring of entity selector tokens: the checks `register_entity_selector` enforces.
use super::*;

/// Vanilla's selector letters, which a mod's token never replaces.
const VANILLA: [&str; 6] = ["p", "a", "r", "s", "e", "n"];

impl EntitySelectorsRegistrar for PartRegistrar {
    fn add_entity_selector(
        &mut self,
        name: &str,
        selector: Arc<dyn crate::EntitySelectorType>,
    ) -> Result<(), crate::EntitySelectorRegistrationError> {
        use crate::EntitySelectorRegistrationError as Error;
        if mod_vocab::__private::parts_installation::parts_installed() {
            return Err(Error::RegistrationClosed {
                name: name.to_owned(),
            });
        }
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
        {
            return Err(Error::InvalidToken {
                name: name.to_owned(),
            });
        }
        if VANILLA.contains(&name) {
            return Err(Error::ReservedVanillaToken {
                name: name.to_owned(),
            });
        }
        if self
            .parts
            .entity_selectors
            .iter()
            .any(|other| *other.name == *name)
        {
            return Err(Error::DuplicateToken {
                name: name.to_owned(),
            });
        }
        let index = self
            .parts
            .entity_selectors
            .partition_point(|info| info.name.as_ref() < name);
        self.parts.entity_selectors.insert(
            index,
            EntitySelectorInfo {
                owner: mod_vocab::__private::game::CallbackOwner::for_index(
                    self.mod_id,
                    self.mod_index,
                ),
                name: name.into(),
                selector,
            },
        );
        Ok(())
    }
}

/// Fixtures that tests of other crates use to install entity selector tokens.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use mod_vocab::EntitySelectorsRegistrar;

    #[test]
    fn registration_allows_over_256_kib_metadata_zero_limit_and_defers_base_evaluation() {
        struct Large;
        impl crate::EntitySelectorType for Large {
            fn base(&self) -> crate::SelectorBase {
                crate::SelectorBase::new(
                    false,
                    Some(0),
                    crate::SelectorSort::Nearest,
                    "x".repeat(32767),
                )
            }
            fn matches(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &crate::CommandSource,
                _: crate::EntityRef,
            ) -> bool {
                true
            }
        }
        struct DeferredPanic;
        impl crate::EntitySelectorType for DeferredPanic {
            fn base(&self) -> crate::SelectorBase {
                panic!("base must not be evaluated during registration")
            }
            fn matches(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &crate::CommandSource,
                _: crate::EntityRef,
            ) -> bool {
                false
            }
        }

        let _ = mod_vocab::testing::rebuild_parts(|| -> Result<(), ()> {
            let mut registrar = super::PartRegistrar::new("large_selectors");
            for n in 0..9 {
                registrar
                    .add_entity_selector(&format!("t{n}"), std::sync::Arc::new(Large))
                    .unwrap();
            }
            registrar
                .add_entity_selector("deferred_panic", std::sync::Arc::new(DeferredPanic))
                .unwrap();
            assert_eq!(registrar.parts.entity_selectors.len(), 10);
            assert!(registrar.refused.is_empty());
            Err(())
        });
    }

    #[test]
    fn registration_enforces_brigadier_tokens_and_returns_scoped_errors() {
        use crate::EntitySelectorRegistrationError as Error;

        struct Everyone(u32);
        impl crate::EntitySelectorType for Everyone {
            fn base(&self) -> crate::SelectorBase {
                crate::SelectorBase::new(
                    true,
                    Some(self.0),
                    crate::SelectorSort::Nearest,
                    "test.selector.allies",
                )
            }
            fn matches(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &crate::CommandSource,
                _: crate::EntityRef,
            ) -> bool {
                true
            }
        }

        let _ = mod_vocab::testing::rebuild_parts(|| -> Result<(), ()> {
            let mut registrar = super::PartRegistrar::new("scoped");
            registrar
                .add_entity_selector("Allies_1.0+team-a", std::sync::Arc::new(Everyone(1)))
                .unwrap();
            registrar
                .add_entity_selector("zero_limit", std::sync::Arc::new(Everyone(0)))
                .unwrap();

            assert_eq!(
                registrar.add_entity_selector("", std::sync::Arc::new(Everyone(1))),
                Err(Error::EmptyName)
            );
            assert_eq!(
                registrar.add_entity_selector("bad token", std::sync::Arc::new(Everyone(1))),
                Err(Error::InvalidToken {
                    name: "bad token".into()
                })
            );
            for vanilla in ["p", "a", "r", "s", "e", "n"] {
                assert_eq!(
                    registrar.add_entity_selector(vanilla, std::sync::Arc::new(Everyone(1))),
                    Err(Error::ReservedVanillaToken {
                        name: vanilla.into()
                    })
                );
            }
            assert_eq!(
                registrar
                    .add_entity_selector("Allies_1.0+team-a", std::sync::Arc::new(Everyone(1))),
                Err(Error::DuplicateToken {
                    name: "Allies_1.0+team-a".into()
                })
            );
            assert!(
                registrar.refused.is_empty(),
                "handled Err results must not poison unrelated registrations"
            );
            assert_eq!(
                registrar
                    .parts
                    .entity_selectors
                    .iter()
                    .map(|info| info.name.as_ref())
                    .collect::<Vec<_>>(),
                ["Allies_1.0+team-a", "zero_limit"]
            );

            struct Allies;
            #[mod_api::__subscriptions]
            impl crate::Hooks for Allies {
                fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                    r.register_entity_selector("Allies_1.0+team-a", Everyone(1))
                        .unwrap();
                }
            }

            crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
                "test",
                Box::new(Allies),
            )])
            .unwrap();
            let installed = mod_vocab::__private::parts_installation::testing_entity_selectors();
            assert_eq!(installed.len(), 1);
            assert_eq!(&*installed[0].name, "Allies_1.0+team-a");
            assert_eq!(installed[0].owner.id(), "test");
            assert_eq!(installed[0].selector.base().limit, Some(1));

            let mut closed = super::PartRegistrar::new("late");
            assert_eq!(
                closed.add_entity_selector("after_freeze", std::sync::Arc::new(Everyone(1))),
                Err(Error::RegistrationClosed {
                    name: "after_freeze".into()
                })
            );
            assert!(closed.refused.is_empty());
            Err(())
        });
    }
}
