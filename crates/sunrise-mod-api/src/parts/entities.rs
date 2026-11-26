//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use crate::*;

    #[test]
    pub(crate) fn entity_parts_refuse_unnamespaced_ids_and_doubles() {
        struct Once;
        #[mod_api::sunrise_part]
        impl super::EntityPart for Once {
            fn register_goals(
                &self,
                _host: &mut dyn crate::ServerHost,
                event: &mut crate::RegisterGoalsEvent<'_, '_>,
            ) {
                let _ = event;
            }
        }
        struct Twice;
        #[mod_api::sunrise_part]
        impl super::EntityPart for Twice {
            fn register_goals(
                &self,
                _host: &mut dyn crate::ServerHost,
                event: &mut crate::RegisterGoalsEvent<'_, '_>,
            ) {
                let _ = event;
            }
        }
        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                r.entity_part("zombie", Twice);
                r.entity_part("minecraft:zombie", Once);
                r.entity_part("minecraft:zombie", Once);
            }
        }
        let errors = crate::parts::install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("to \"zombie\", which is not a namespaced id"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("attached to minecraft:zombie twice"),
            "{errors:?}"
        );
    }

    #[test]
    fn entity_part_with_state_refuses_stale_key_and_accepts_live_key() {
        struct Charged(AttachmentKey<u32>);
        #[mod_api::sunrise_part]
        impl super::EntityPart for Charged {}
        impl EntityPartState for Charged {
            type Data = u32;
            fn state_key(&self) -> &AttachmentKey<u32> {
                &self.0
            }
        }
        struct StaleMod(AttachmentKey<u32>);
        #[mod_api::__subscriptions]
        impl crate::Hooks for StaleMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let err = r
                    .entity_part_with_state("minecraft:cow", Charged(self.0.clone()))
                    .unwrap_err();
                assert!(matches!(
                    err,
                    EntityPartStateError::State {
                        kind: "minecraft:cow",
                        namespace: "test",
                        name: "charge",
                        reason: PartStateRefusal::StaleKey,
                        ..
                    }
                ));
            }
        }
        struct LiveMod;
        #[mod_api::__subscriptions]
        impl crate::Hooks for LiveMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let live = r
                    .register_attachment_type(AttachmentDescriptor::new("test:charge", || 0_u32))
                    .unwrap();
                r.entity_part_with_state("minecraft:cow", Charged(live))
                    .unwrap();
            }
        }
        let stale = crate::testing::registered_attachment_fixture(
            "test",
            AttachmentDescriptor::new("test:charge", || 0_u32),
        )
        .1;
        let refused_registrar = crate::parts::shared::register_all(
            &[ModEntry::new("test", Box::new(StaleMod(stale)))],
            |_| false,
        );
        assert_eq!(refused_registrar.refused.len(), 1);
        assert!(refused_registrar.refused[0].contains("test:charge"));

        let ok_registrar =
            crate::parts::shared::register_all(&[ModEntry::new("test", Box::new(LiveMod))], |_| {
                false
            });
        assert!(ok_registrar.refused.is_empty());
        assert_eq!(ok_registrar.entity_part_states.len(), 1);
    }

    pub(crate) struct EntityBattery(pub(crate) EntityRef, pub(crate) AttachmentKey<u32>);
    impl EnergyHandler for EntityBattery {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            self.1
                .get_existing(host, Holder::Entity(self.0.id()), |value| value.copied())
                .unwrap()
                .unwrap_or(0)
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            1000
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            assert!(host.transaction_matches(Some(tx)));
            let held = self.amount(host);
            let moved = amount.min(1000 - held);
            self.1
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            Ok(moved)
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            assert!(host.transaction_matches(Some(tx)));
            let held = self.amount(host);
            let moved = amount.min(held);
            self.1
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            Ok(moved)
        }
    }

    pub(crate) struct EntityTank(pub(crate) EntityRef, pub(crate) AttachmentKey<u32>);
    impl ResourceHandler<FluidResource> for EntityTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<FluidResource> {
            (self.amount(host, 0) > 0).then(|| {
                FluidResource::of(&FluidStack::new(
                    mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
                    1,
                ))
            })
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.1
                .get_existing(host, Holder::Entity(self.0.id()), |value| value.copied())
                .unwrap()
                .unwrap_or(0)
        }
        fn capacity(
            &self,
            _: &mut dyn ServerHost,
            index: usize,
            resource: Option<&FluidResource>,
        ) -> u32 {
            if index == 0
                && resource.is_none_or(|fluid| {
                    *fluid
                        == FluidResource::of(&FluidStack::new(
                            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
                            1,
                        ))
                })
            {
                1000
            } else {
                0
            }
        }
        fn insert_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            assert!(host.transaction_matches(Some(tx)));
            let held = self.amount(host, index);
            let moved = amount.min(
                self.capacity(host, index, Some(resource))
                    .saturating_sub(held),
            );
            self.1
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            Ok(moved)
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            tx: &TransactionContext,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            assert!(host.transaction_matches(Some(tx)));
            let held = self.amount(host, index);
            let moved = if self.capacity(host, index, Some(resource)) > 0 {
                amount.min(held)
            } else {
                0
            };
            self.1
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            Ok(moved)
        }
    }
}

impl EntitiesRegistrar for PartRegistrar {
    fn attach_entity_part(&mut self, entity: &str, kind: &'static str, part: Arc<dyn EntityPart>) {
        self.attach(|p| &mut p.entities, kind, entity, part);
    }

    fn declare_entity_part_state(
        &mut self,
        entity: &str,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::EntityPartStateError> {
        // 🤖 Agent: one leaked id per declaration, once at startup, so a refusal can name the
        // type in the `Copy` refusal it shares with block entities.
        let kind: &'static str = Box::leak(entity.into());
        match self.check_part_state(&declaration) {
            Ok(()) => {
                self.entity_part_states.push(
                    mod_vocab::__private::EntityPartStateDeclaration::new(
                        self.mod_id.to_owned(),
                        part,
                        kind,
                        declaration,
                    ),
                );
                Ok(())
            }
            Err(reason) => {
                let error = declaration.refused_on_entity(part, kind, reason);
                self.refused.push(format!("{}: {error}", self.mod_id));
                Err(error)
            }
        }
    }

    fn provide_entity_capability(
        &mut self,
        name: &'static str,
        kind: &str,
        before_game: bool,
        provider: NativeCapabilityProvider,
    ) {
        self.capability(name, ProviderTarget::Entity(kind), before_game, provider);
    }
}
