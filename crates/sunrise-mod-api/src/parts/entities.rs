//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use crate::parts::tests::*;
    use crate::*;

    /// The types mods name to `entity_tick_each` are told one at a time, every type once a mod
    /// names them all, and a type that is not a namespaced id refuses the registration.
    #[test]
    fn entity_tick_each_names_the_declared_types() {
        let mut registrar = crate::parts::PartRegistrar::new("first");
        registrar.entity_tick_each("minecraft:zombie");
        registrar.mod_id = "second";
        registrar.entity_tick_each("mymod:golem");
        registrar.entity_tick_each("minecraft:zombie");
        registrar.entity_tick_each("Zombie");
        assert_eq!(registrar.refused.len(), 1);
        let each = mod_vocab::__private::EntityTickEach::of(&registrar.entity_tick_each);
        assert!(each.names("minecraft:zombie") && each.names("mymod:golem"));
        assert!(!each.names("minecraft:item") && !each.all());
        registrar.entity_tick_each_entity();
        let every = mod_vocab::__private::EntityTickEach::of(&registrar.entity_tick_each);
        assert!(every.all() && every.names("minecraft:item"));
        assert!(!mod_vocab::__private::EntityTickEach::of(&[]).any());
    }

    #[test]
    pub(crate) fn entity_parts_refuse_unnamespaced_ids_and_doubles() {
        struct Once;
        #[mod_api::sunrise_part]
        impl super::EntityPart for Once {
            fn register_goals(&self, goals: &mut crate::GoalRegistrar<'_>) {
                let _ = goals;
            }
        }
        struct Twice;
        #[mod_api::sunrise_part]
        impl super::EntityPart for Twice {
            fn register_goals(&self, goals: &mut crate::GoalRegistrar<'_>) {
                let _ = goals;
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

    pub(crate) struct EntityBattery(pub(crate) EntityRef);
    impl EnergyHandler for EntityBattery {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            ROBOT_ENERGY
                .get_existing(host, Holder::Entity(self.0.id()))
                .unwrap()
                .unwrap_or(0)
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            1000
        }
        fn insert(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            let moved = amount.min(1000 - held);
            ROBOT_ENERGY
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            moved
        }
        fn extract(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            let moved = amount.min(held);
            ROBOT_ENERGY
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            moved
        }
    }

    pub(crate) struct EntityTank(pub(crate) EntityRef);
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
            ROBOT_FLUID
                .get_existing(host, Holder::Entity(self.0.id()))
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
            _: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let held = self.amount(host, index);
            let moved = amount.min(
                self.capacity(host, index, Some(resource))
                    .saturating_sub(held),
            );
            ROBOT_FLUID
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            moved
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let held = self.amount(host, index);
            let moved = if self.capacity(host, index, Some(resource)) > 0 {
                amount.min(held)
            } else {
                0
            };
            ROBOT_FLUID
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            moved
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
        match self.check_part_state(declaration) {
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
        provider: EntityProviderOf,
    ) {
        self.capability(
            |p| &mut p.entity_capabilities,
            None,
            name,
            ProviderTarget::Id(kind),
            before_game,
            {
                let handler =
                    mod_vocab::__private::parts_installation::entity_provider_handler(&provider);
                (provider, handler)
            },
        );
    }

    fn entity_tick_each(&mut self, entity: &str) {
        if crate::Identifier::parse_namespaced(entity).is_none() {
            let message = format!(
                "mod {} entity_tick_each {entity:?}: lowercase resource-id namespace and path required",
                self.mod_id
            );
            self.refused.push(message);
            return;
        }
        self.entity_tick_each.push(Some(entity.to_owned()));
    }

    fn entity_tick_each_entity(&mut self) {
        self.entity_tick_each.push(None);
    }
}
