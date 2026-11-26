//! Registration of typed native capability providers.
use super::*;
use mod_vocab::__private::parts_installation::{
    block_entity_type_key_slot, block_key_slot, capability_provider_binding,
};

pub(crate) enum ProviderTarget<'a> {
    Block(crate::BlockKey),
    BlockEntity(crate::BlockEntityTypeKey),
    Item(&'a str),
    Entity(&'a str),
}

impl PartRegistrar {
    /// Register one native provider after validating its name, target and handler/context binding.
    pub(crate) fn capability(
        &mut self,
        name: &'static str,
        target: ProviderTarget<'_>,
        before_game: bool,
        provider: NativeCapabilityProvider,
    ) {
        let me = self.mod_id;
        let registry_keys = Rc::clone(&self.registry_keys);
        let mut keys = registry_keys.borrow_mut();
        let (by, id) = match target {
            ProviderTarget::Block(block) => (
                ProvidedBy::Block,
                self.capability_key_target(name, "block", &keys.blocks, block_key_slot(block)),
            ),
            ProviderTarget::BlockEntity(kind) => (
                ProvidedBy::BlockEntity,
                self.capability_key_target(
                    name,
                    "block entity type",
                    &keys.block_entity_types,
                    block_entity_type_key_slot(kind),
                ),
            ),
            ProviderTarget::Item(id) | ProviderTarget::Entity(id) => {
                let (by, kind, registry) = if matches!(target, ProviderTarget::Item(_)) {
                    (ProvidedBy::Item, "item", &mut keys.items)
                } else {
                    (ProvidedBy::Entity, "entity type", &mut keys.entity_types)
                };
                let Some(identifier) = crate::Identifier::parse_namespaced(id) else {
                    self.refused.push(format!(
                        "mod {me} answered capability {name} for invalid {kind} id {id:?}: expected a namespaced identifier"
                    ));
                    return;
                };
                let slot = match registry.request_identifier(me, &identifier) {
                    Ok(slot) => slot,
                    Err(error) => {
                        self.refused.push(error);
                        return;
                    }
                };
                (by, Some(registry.entries[slot as usize].id.as_str()))
            }
        };
        let Some(id) = id else {
            return;
        };
        if self
            .note_capability(by, name, capability_provider_binding(&provider))
            .is_none()
        {
            return;
        }
        let table = match by {
            ProvidedBy::Block => &mut self.parts.block_capabilities,
            ProvidedBy::BlockEntity => &mut self.parts.capabilities,
            ProvidedBy::Item => &mut self.parts.item_capabilities,
            ProvidedBy::Entity => &mut self.parts.entity_capabilities,
        };
        let providers = table.entry(name).or_default().entry(id.into()).or_default();
        self.registered += 1;
        let order = self.registered;
        // Those asked ahead of the game first, each kind in registration order.
        let at = if before_game {
            providers.partition_point(|part| part.before_game)
        } else {
            providers.len()
        };
        providers.insert(
            at,
            CapabilityPart {
                mod_id: me,
                mod_index: self.mod_index,
                before_game,
                order,
                poisoned: Default::default(),
                provider,
            },
        );
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::{FakeContainer, FakeHost};

    #[cfg(test)]
    /// `test:ordered` block entities from x = 7 to 10, where the game has slots of its own
    /// from x = 8 on.
    pub(crate) fn ordered_world() -> FakeHost {
        let mut host = FakeHost::new();
        for x in 7..=10 {
            let ordered = host.level.block_entity([x, 64, 0]).of_type("test:ordered");
            if x >= 8 {
                ordered.with_items(27);
            }
        }
        host
    }

    #[cfg(test)]
    /// A pipe at (0, 64, 0), and east of it a vanilla chest of one slot that holds one item,
    /// answered by the game.
    pub(crate) fn pipe_world() -> FakeHost {
        let mut host = FakeHost::new();
        host.level.block_entity(PIPE).of_type("test:pipe");
        host.level.block_entity(CHEST).items = Some({
            let mut value = FakeContainer::new(0);
            value.slots = vec![None];
            value.limit = 1;
            value
        });
        host
    }

    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    /// A chemical, as Mekanism's: the test mods' own resource, which the game knows nothing of.
    #[derive(Clone, Debug, PartialEq)]
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct Chemical(pub String);

    /// The test mods' own capability, as Mekanism's `Capabilities.CHEMICAL`: the game has no
    /// answer for it, so only their providers do.
    pub const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> =
        CapabilityKey::new("test:chemical");

    /// A mod block entity's one tank of 1000 of a chemical, kept in its saved data (`chemical`
    /// and `amount`), whose writes the game's journal keeps for a transaction.
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct ChemTank(pub BlockSide);
    impl ChemTank {
        fn entity(&self, host: &dyn Host) -> Option<BlockEntityRef> {
            host.block_entity(self.0.level, self.0.pos)
        }
        fn held(&self, host: &dyn Host) -> Option<(Chemical, u32)> {
            let data = self.entity(host)?.data(host)?;
            let Some(Tag::String(chemical)) = data.get(&"chemical".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = data.get(&"amount".into()) else {
                return None;
            };
            let amount = u32::try_from(amount).ok().filter(|&a| a > 0)?;
            Some((Chemical(chemical.to_string_lossy()), amount))
        }
        fn set(&self, host: &mut dyn ServerHost, chemical: &Chemical, amount: u32) {
            let Some(entity) = self.entity(host) else {
                return;
            };
            let mut data = Compound::new();
            data.insert("chemical".into(), Tag::String(chemical.0.as_str().into()));
            data.insert("amount".into(), Tag::Int(amount as i32));
            let _ = entity.merge_data(host, data);
        }
    }
    impl ResourceHandler<Chemical> for ChemTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
            self.held(host).map(|(chemical, _)| chemical)
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.held(host).map_or(0, |(_, amount)| amount)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&Chemical>) -> u32 {
            1000
        }
        fn insert_at(
            &self,
            host: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            let held = match self.held(host) {
                Some((other, _)) if other != *chemical => return Ok(0),
                Some((_, held)) => held,
                None => 0,
            };
            let went = amount.min(1000 - held);
            if went > 0 {
                self.set(host, chemical, held + went);
            }
            Ok(went)
        }
        fn extract_at(
            &self,
            host: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            let Some((held_chemical, held)) = self.held(host).filter(|(c, _)| c == chemical) else {
                return Ok(0);
            };
            let came = amount.min(held);
            self.set(host, &held_chemical, held - came);
            Ok(came)
        }
    }

    /// Attachment stored by the active Doubler fixture.
    pub fn robot_fluid_key() -> AttachmentKey<u32> {
        crate::testing::attachment_key("doubler", "fluid")
    }

    /// A second test mod, sharing `CHEMICALS` and `Chemical` with the first, as a mod depending
    /// on another's root crate does: its vats answer the same capability.
    pub(crate) struct Chemist;
    #[mod_api::__subscriptions]
    impl Hooks for Chemist {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let chem_vat = r.block_entity_type_key("test:chem_vat");
            r.block_entity_capability(&CHEMICALS, &chem_vat, |_, at, _, _, offer| {
                offer.offer(|| Box::new(ChemTank(at)))
            });
        }
    }

    pub(crate) const CAPABILITY_TYPES: &[&str] = &[
        "pipe",
        "greedy",
        "item_overstates",
        "ordered",
        "tank",
        "gives_more",
        "takes_more",
        "battery_gives_more",
        "battery_takes_more",
        "chem_tank",
        "chem_vat",
    ];

    #[test]
    pub(crate) fn a_level_not_in_place_asks_no_provider() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        // The pipe's type has a provider, which answers in the level in place; parked while
        // the server ticks another, the same block entity's level gives none.
        let parked = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::NETHER);
        assert!(here().capability(host, &ITEMS, PIPE, None).is_some());
        assert!(parked.capability(host, &ITEMS, PIPE, None).is_none());
    }

    #[test]
    pub(crate) fn a_mod_reads_a_vanilla_chests_items_from_the_game_with_no_provider_for_it() {
        game_thread();
        let mut world = pipe_world();
        let chest = world.level.block_entity(CHEST).items.as_mut().unwrap();
        chest.slots[0] = Some(stick());
        let host: &mut dyn ServerHost = &mut world;
        // No mod registered an items provider for the chest's type: the game's hoppers have no
        // mod's handler to look for there, yet a mod reading it gets the game's own slots.
        assert!(block_providers(&ITEMS, host, on(CHEST, None)).is_none());
        let chest = here().capability(host, &ITEMS, CHEST, None).unwrap();
        assert_eq!(chest.owner(), "the game");
        assert_eq!(chest.size(host), 1);
        assert_eq!(chest.resource(host, 0), Some(ItemResource::of(&stick())));
    }

    #[test]
    pub(crate) fn a_capability_part_answers_by_side_and_forwards_to_its_neighbours_capability() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        let up = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::Up))
            .unwrap();
        assert_eq!(up.size(host), 0);
        let pipe = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::North))
            .expect("the pipe's part");
        assert_eq!(pipe.size(host), 1);
        let stick_ = ItemResource::of(&stick());
        // The pipe calls the chest's capability, the game's own, with the host it was given:
        // in an aborted transaction the stick goes in, and comes back out.
        let aborted: Result<(), crate::TransactionError<u32>> =
            host.transaction(|host, tx| Err(pipe.insert_at(host, tx, 0, &stick_, 1).unwrap()));
        assert_eq!(aborted, Err(crate::TransactionError::Aborted(1)));
        let backend = backend_of(&*host).unwrap();
        backend.lend_container_stack(on(CHEST, None), 0, &mut |_| {
            panic!("empty slot must not be lent");
        });
        let went = host.transaction(|host, tx| pipe.insert_at(host, tx, 0, &stick_, 1));
        assert_eq!(went, Ok(1));
        let mut stack = None;
        backend_of(&*host)
            .unwrap()
            .lend_container_stack(on(CHEST, None), 0, &mut |view| stack = view.project());
        assert_eq!(stack, Some(stick()));
        let went = host.transaction(|host, tx| pipe.insert_at(host, tx, 0, &stick_, 1));
        assert_eq!(went, Ok(0));
        // No block entity with slots: no answer. A mod's own capability has no game answer.
        assert!(here().capability(host, &ITEMS, [5, 64, 0], None).is_none());
        const CHEMICALS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:chemicals");
        assert!(here().capability(host, &CHEMICALS, CHEST, None).is_none());
    }

    #[cfg(test)]
    /// Puts `amount` of `item` into slot 0 of [`ITEM_ITEMS`]' handler for the item at `place`, in a
    /// transaction committed when all went in: how much did.
    pub(crate) fn fill(
        host: &mut dyn ServerHost,
        place: &ItemPlace,
        item: ItemStack,
        amount: u32,
    ) -> u32 {
        let handler = place
            .capability(host, &ITEM_ITEMS)
            .expect("the item's part");
        let resource = ItemResource::of(&item);
        let went =
            host.transaction(
                |host, tx| match handler.insert_at(host, tx, 0, &resource, amount) {
                    Ok(went) if went == amount => Ok(went),
                    Ok(went) => Err(went),
                    Err(_) => Err(0),
                },
            );
        match went {
            Ok(went) | Err(crate::TransactionError::Aborted(went)) => went,
            Err(
                crate::TransactionError::InvalidContext
                | crate::TransactionError::Unavailable
                | crate::TransactionError::CommitRefused,
            ) => 0,
        }
    }

    /// A mod declaring its own key under a name another mod's providers answer with another
    /// handler type is refused at load, naming both mods and both types, as is a key whose name
    /// has no namespace; the same name with the same type, from another mod, is not.
    #[test]
    pub(crate) fn a_capability_name_answered_with_two_handler_types_is_refused() {
        const NOT_CHEMICALS: CapabilityKey<dyn ResourceHandler<FluidResource>> =
            CapabilityKey::new("test:chemical");
        const UNNAMESPACED: CapabilityKey<dyn ResourceHandler<Chemical>> =
            CapabilityKey::new("chemical");
        const MALFORMED: CapabilityKey<dyn ResourceHandler<Chemical>> =
            CapabilityKey::new("test:Chemical");
        let mut r = super::PartRegistrar::new("first");
        let chem_tank = r.block_entity_type_key("test:chem_tank");
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &CHEMICALS,
            &chem_tank,
            |_, at, _, _, offer| offer.offer(|| Box::new(ChemTank(at))),
        );
        r.mod_id = "second";
        r.mod_index = 1;
        let chem_vat = r.block_entity_type_key("test:chem_vat");
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &NOT_CHEMICALS,
            &chem_vat,
            |_, at, _, _, offer| offer.offer(|| Box::new(ModTank { at, lie: None })),
        );
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &CHEMICALS,
            &chem_vat,
            |_, at, _, _, offer| offer.offer(|| Box::new(ChemTank(at))),
        );
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &UNNAMESPACED,
            &chem_vat,
            |_, at, _, _, offer| offer.offer(|| Box::new(ChemTank(at))),
        );
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &MALFORMED,
            &chem_vat,
            |_, at, _, _, offer| offer.offer(|| Box::new(ChemTank(at))),
        );
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        let two_types = &r.refused[0];
        assert!(
            two_types.starts_with("capability test:chemical is answered with two handler types")
        );
        let chemicals = type_name::<dyn ResourceHandler<Chemical>>();
        let fluids = type_name::<dyn ResourceHandler<FluidResource>>();
        assert!(
            two_types.contains(&format!("{chemicals} by first")),
            "{two_types}"
        );
        assert!(
            two_types.contains(&format!("{fluids} by second")),
            "{two_types}"
        );
        assert!(r.refused[1].contains("capability \"chemical\", which is not a namespaced name"));
        assert!(
            r.refused[2].contains("capability \"test:Chemical\", which is not a namespaced name")
        );
    }

    #[test]
    fn an_entity_capability_name_with_two_context_types_is_refused() {
        const UNSIDED: EntityCapabilityKey<dyn ResourceHandler<ItemResource>> =
            EntityCapabilityKey::new("test:pig_items");
        const SIDED: EntityCapabilityKey<dyn ResourceHandler<ItemResource>, Option<Direction>> =
            EntityCapabilityKey::new("test:pig_items");

        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}

        crate::testing::with_mods(
            vec![
                ModEntry::new("first", Box::new(Empty)),
                ModEntry::new("second", Box::new(Empty)),
            ]
            .into(),
            || {
                let mut registrar = PartRegistrar::new("first");
                (&mut registrar as &mut dyn Registrar).entity_capability(
                    &UNSIDED,
                    "minecraft:pig",
                    |_, _, ()| Some(Box::new(Marked(3))),
                );
                assert!(registrar.refused.is_empty());
                registrar.mod_id = "second";
                registrar.mod_index = 1;
                (&mut registrar as &mut dyn Registrar).entity_capability(
                    &SIDED,
                    "minecraft:pig",
                    |_, _, _| Some(Box::new(Marked(99))),
                );
                // A refusal fails loading. Install only the accepted registrations to check
                // that refusing the second callback did not change what the first one answers.
                let refused = std::mem::take(&mut registrar.refused);
                let [refusal] = refused.as_slice() else {
                    panic!("expected one refusal, got {refused:?}");
                };
                for detail in ["test:pig_items", "context types", "by first", "by second"] {
                    assert!(refusal.contains(detail), "{refusal}");
                }
                install_registrar(registrar, &rule_schema()).unwrap();
                assert!(has_entity_providers(&UNSIDED));
                assert!(!has_entity_providers(&SIDED));
                let mut host = player_world(None);
                let pig = EntityRef::new(WorldEpoch::new(1), 2);
                let found = pig.capability(&mut host, &UNSIDED, ()).unwrap();
                assert_eq!(found.owner(), "first");
                assert_eq!(found.size(&mut host), 3);
                assert!(
                    pig.capability(&mut host, &SIDED, Some(Direction::North))
                        .is_none()
                );
            },
        );
    }

    #[test]
    fn builtin_keys_reject_incompatible_first_providers_and_coverage_queries() {
        const WRONG_BLOCK: CapabilityKey<dyn EnergyHandler> = CapabilityKey::new(ITEMS.name);
        const WRONG_ITEM: ItemCapabilityKey<dyn EnergyHandler> =
            ItemCapabilityKey::new(ITEM_ITEMS.name);
        const UNSIDED: EntityCapabilityKey<dyn ResourceHandler<ItemResource>> =
            EntityCapabilityKey::new(ENTITY_AUTOMATION.name);

        let mut registrar = PartRegistrar::new("wrong");
        let registration = &mut registrar as &mut dyn Registrar;
        let block = registration.block_key("test:container");
        registration.block_capability(&WRONG_BLOCK, &block, |_, _, _, _, _| {});
        registration.item_capability(&WRONG_ITEM, "test:backpack", |_, _| None);
        registration.entity_capability(&UNSIDED, "minecraft:pig", |_, _, ()| None);
        assert_eq!(registrar.refused.len(), 3);
        assert!(
            registrar
                .refused
                .iter()
                .all(|error| error.contains("by the game"))
        );
        assert!(registrar.refused[2].contains("context types"));

        registrar.mod_id = "right";
        let registration = &mut registrar as &mut dyn Registrar;
        registration.block_capability(&ITEMS, &block, |_, _, _, _, _| {});
        registration.item_capability(&ITEM_ITEMS, "test:backpack", |_, _| None);
        registration.entity_capability(&ENTITY_AUTOMATION, "minecraft:pig", |_, _, _| None);
        let item = registration.item_key("test:backpack");
        let pig = registration.entity_type_key("minecraft:pig");
        assert!(registration.is_block_capability_registered(&ITEMS, &block));
        assert!(registration.is_item_capability_registered(&ITEM_ITEMS, &item));
        assert!(registration.is_entity_capability_registered(&ENTITY_AUTOMATION, &pig));
        assert!(!registration.is_block_capability_registered(&WRONG_BLOCK, &block));
        assert!(!registration.is_item_capability_registered(&WRONG_ITEM, &item));
        assert!(!registration.is_entity_capability_registered(&UNSIDED, &pig));
        assert_eq!(registrar.refused.len(), 6);
    }

    #[test]
    fn a_different_block_binding_cannot_mark_or_inherit_proxyability() {
        const WRONG: CapabilityKey<dyn EnergyHandler> = CapabilityKey::new(ITEMS.name);
        let mut registrar = PartRegistrar::new("forwarder");
        let registration = &mut registrar as &mut dyn Registrar;
        registration.set_capability_proxyable(&WRONG, true);
        assert!(!registration.block_capability_proxyable(ITEMS.name));
        registration.set_capability_proxyable(&ITEMS, true);
        assert_eq!(std::mem::take(&mut registrar.refused).len(), 1);
        install_registrar(registrar, &rule_schema()).unwrap();

        let host = FakeHost::new();
        assert!(ITEMS.is_proxyable(&host));
        assert!(!WRONG.is_proxyable(&host));
        assert!(block_capability_proxyable(&host, ITEMS.name));
    }

    #[test]
    fn item_and_entity_provider_ids_are_startup_registry_requests() {
        let mut registrar = PartRegistrar::new("provider");
        let registration = &mut registrar as &mut dyn Registrar;
        registration.item_capability(&ITEM_ITEMS, "test:later_item", |_, _| None);
        registration.item_capability_before_game(&ITEM_ITEMS, "test:typo_item", |_, _| None);
        registration.entity_capability_before_game(
            &ENTITY_ITEMS,
            "test:later_entity",
            |_, _, ()| None,
        );
        registration.entity_capability(&ENTITY_ITEMS, "minecraft:pigg", |_, _, ()| None);
        install_registrar(registrar, &rule_schema()).unwrap();

        let errors = mod_vocab::__private::resolve_items(["test:later_item"]).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].mod_id, "provider");
        assert_eq!(errors[0].id, "test:typo_item");
        let errors = mod_vocab::__private::resolve_entity_types(["test:later_entity"], |_| None)
            .unwrap_err();
        assert_eq!(
            errors,
            ["mod provider requested unknown entity type minecraft:pigg"]
        );
        mod_vocab::__private::resolve_items(["test:later_item", "test:typo_item"]).unwrap();
        mod_vocab::__private::resolve_entity_types(["test:later_entity", "minecraft:pigg"], |_| {
            None
        })
        .unwrap();
    }

    /// A handler that only tells which provider made it, by its slot count.
    pub(crate) struct Marked(pub(crate) usize);
    impl ResourceHandler<ItemResource> for Marked {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            self.0
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
            0
        }
        fn insert_at(
            &self,
            _: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            Ok(0)
        }
        fn extract_at(
            &self,
            _: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            Ok(0)
        }
    }

    /// A trash can's one slot, which takes any amount of anything and keeps none of it.
    pub(crate) struct VoidBin;
    impl ResourceHandler<ItemResource> for VoidBin {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
            u32::MAX
        }
        fn insert_at(
            &self,
            _: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            _: &ItemResource,
            amount: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            Ok(amount)
        }
        fn extract_at(
            &self,
            _: &mut dyn ServerHost,
            _: &TransactionContext,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> Result<u32, TransactionError<Overanswered>> {
            Ok(0)
        }
    }

    #[test]
    pub(crate) fn a_block_with_no_block_entity_answers_through_its_block_provider() {
        game_thread();
        let bin = [0, 64, 0];
        let mut world = FakeHost::new();
        world.level.set_block(bin, "test:void_bin");
        world.level.set_block([1, 64, 0], "minecraft:stone");
        let host: &mut dyn ServerHost = &mut world;
        let voids = here()
            .capability(host, &ITEMS, bin, Some(Direction::Up))
            .expect("the bin's provider");
        assert_eq!(voids.owner(), "doubler");
        let stick_ = ItemResource::of(&stick());
        let went = host.transaction(|host, tx| voids.insert_at(host, tx, 0, &stick_, 64));
        assert_eq!(went, Ok(64));
        assert_eq!(voids.amount(host, 0), 0);
        // A block no provider is registered for has none, as before.
        assert!(here().capability(host, &ITEMS, [1, 64, 0], None).is_none());
        // A hopper's lookup finds the bin where the game has no slots of its own.
        let hopper = block_providers(&ITEMS, host, on(bin, Some(Direction::Up)))
            .expect("the bin's providers")
            .ask(host, false)
            .expect("the bin's handler");
        assert_eq!(hopper.size(host), 1);
        // Providers chosen for the bin don't answer once the block there is another (an earlier
        // provider replaced it, say).
        let chosen = block_providers(&ITEMS, host, on(bin, None)).expect("the bin's providers");
        world.level.set_block(bin, "minecraft:stone");
        let host: &mut dyn ServerHost = &mut world;
        assert!(chosen.ask(host, false).is_none());
        world.level.set_block(bin, "test:void_bin");
        // The provider is given the block entity standing there, and this one declines then.
        world.level.block_entity(bin);
        let host: &mut dyn ServerHost = &mut world;
        assert!(here().capability(host, &ITEMS, bin, None).is_none());
    }

    /// A position's providers, of the block and of the block entity type standing there, are one
    /// list in registration order across both kinds: those ahead of the game, the game's own,
    /// then the others.
    #[test]
    pub(crate) fn a_positions_block_and_block_entity_type_providers_follow_registration_order() {
        game_thread();
        let mut world = FakeHost::new();
        for x in [3, 4, 5, 6, 11, 12] {
            world.level.set_block([x, 64, 0], "test:ordered");
        }
        for x in [4, 5, 6, 11, 12] {
            world.level.block_entity([x, 64, 0]).of_type("test:ordered");
        }
        world.level.block_entity([4, 64, 0]).with_items(27);
        let host: &mut dyn ServerHost = &mut world;
        let size = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            found.size(host)
        };
        // Ahead of the game, the block's (8), registered before the block entity type's (4).
        assert_eq!(size(host, 12), 8);
        // Ahead of the game, the block entity type's (4) before the block's registered later (5).
        assert_eq!(size(host, 11), 4);
        // The block's ahead of the game (5) before every provider after it.
        assert_eq!(size(host, 6), 5);
        // After the game, the block's (7), registered before the block entity type's (1).
        assert_eq!(size(host, 5), 7);
        // The game's 27 slots before every provider after it.
        assert_eq!(size(host, 4), 27);
        // No block entity: the block's alone, the first that answers there (6).
        assert_eq!(size(host, 3), 6);
        // A hopper's lookup asks them in the same order.
        let ask = |host: &mut dyn ServerHost, x, game| {
            let found = block_providers(&ITEMS, host, on([x, 64, 0], None))?.ask(host, game)?;
            Some(found.size(host))
        };
        assert_eq!(ask(host, 12, true), Some(8));
        assert_eq!(ask(host, 5, false), Some(7));
        assert_eq!(ask(host, 6, true), Some(5));
        assert_eq!(ask(host, 4, true), None);
    }

    /// One capability name has one handler type across a block's and a block entity type's
    /// providers, as they answer the same lookup: the second type is refused at load, whichever
    /// kind registered first.
    #[test]
    pub(crate) fn a_block_and_a_block_entity_type_answering_one_name_with_two_types_are_refused() {
        const SAME_ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:same");
        const SAME_ENERGY: CapabilityKey<dyn EnergyHandler> = CapabilityKey::new("test:same");
        let items = type_name::<dyn ResourceHandler<ItemResource>>();
        let energy = type_name::<dyn EnergyHandler>();
        let battery = |at| {
            Box::new(ModBattery {
                at,
                lie: Lie::GivesMore,
            }) as Box<dyn EnergyHandler>
        };

        let mut r = super::PartRegistrar::new("first");
        let chest = r.block_entity_type_key("test:ordered");
        let bin = r.block_key("test:void_bin");
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &SAME_ITEMS,
            &chest,
            |_, _, _, _, offer| offer.offer(|| Box::new(Marked(1))),
        );
        r.mod_id = "second";
        r.mod_index = 1;
        (&mut r as &mut dyn Registrar).block_capability(
            &SAME_ENERGY,
            &bin,
            move |_, at, _, _, offer| offer.offer(|| battery(at)),
        );
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert!(
            r.refused[0].contains(&format!("{items} by first and {energy} by second")),
            "{}",
            r.refused[0]
        );
        assert!(!r.parts.block_capabilities.contains_key("test:same"));

        let mut r = super::PartRegistrar::new("first");
        let chest = r.block_entity_type_key("test:ordered");
        let bin = r.block_key("test:void_bin");
        (&mut r as &mut dyn Registrar).block_capability(
            &SAME_ENERGY,
            &bin,
            move |_, at, _, _, offer| offer.offer(|| battery(at)),
        );
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &SAME_ITEMS,
            &chest,
            |_, _, _, _, offer| offer.offer(|| Box::new(Marked(1))),
        );
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert!(
            r.refused[0].contains(&format!("{energy} by first and {items} by first")),
            "{}",
            r.refused[0]
        );
        assert!(!r.parts.capabilities.contains_key("test:same"));
    }

    #[test]
    fn block_and_block_entity_providers_walk_past_65535_in_merged_precedence_order() {
        use std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        };

        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}

        const BULK_AFTER_GAME: usize = u16::MAX as usize - 2;

        crate::testing::with_mods(
            vec![
                ModEntry::new("enabled", Box::new(Empty)),
                ModEntry::new("disabled", Box::new(Empty)),
            ]
            .into(),
            || {
                assert!(crate::__private::set_mod_enabled("disabled", false));
                let block_calls = Arc::new(AtomicUsize::new(0));
                let entity_calls = Arc::new(AtomicUsize::new(0));
                let boundary_order = Arc::new(Mutex::new(Vec::<&'static str>::new()));

                let mut registrar = PartRegistrar::new("enabled");
                let block = registrar.block_key("test:target");
                let entity = registrar.block_entity_type_key("test:target");

                {
                    let calls = Arc::clone(&block_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_capability_before_game(
                        &ITEMS,
                        &block,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("block:before_game");
                        },
                    );
                }
                {
                    let calls = Arc::clone(&entity_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability_before_game(
                        &ITEMS,
                        &entity,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("be:before_game");
                        },
                    );
                }

                for _ in 0..BULK_AFTER_GAME {
                    let b_calls = Arc::clone(&block_calls);
                    let e_calls = Arc::clone(&entity_calls);
                    let r = &mut registrar as &mut dyn Registrar;
                    r.block_entity_capability(&ITEMS, &entity, move |_, _, _, _, _| {
                        e_calls.fetch_add(1, Ordering::Relaxed);
                    });
                    r.block_capability(&ITEMS, &block, move |_, _, _, _, _| {
                        b_calls.fetch_add(1, Ordering::Relaxed);
                    });
                }

                registrar.mod_id = "disabled";
                registrar.mod_index = 1;
                (&mut registrar as &mut dyn Registrar).block_capability(
                    &ITEMS,
                    &block,
                    |_, _, _, _, _| panic!("disabled block provider must not run"),
                );

                registrar.mod_id = "enabled";
                registrar.mod_index = 0;
                {
                    let calls = Arc::clone(&entity_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability(
                        &ITEMS,
                        &entity,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("be:65534");
                        },
                    );
                }
                {
                    let calls = Arc::clone(&block_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_capability(
                        &ITEMS,
                        &block,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("block:65535");
                        },
                    );
                }

                registrar.mod_id = "disabled";
                registrar.mod_index = 1;
                (&mut registrar as &mut dyn Registrar).block_entity_capability(
                    &ITEMS,
                    &entity,
                    |_, _, _, _, _| panic!("disabled block-entity provider must not run"),
                );

                registrar.mod_id = "enabled";
                registrar.mod_index = 0;
                {
                    let calls = Arc::clone(&block_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_capability(
                        &ITEMS,
                        &block,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("block:65536");
                        },
                    );
                }
                {
                    let calls = Arc::clone(&entity_calls);
                    let order = Arc::clone(&boundary_order);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability(
                        &ITEMS,
                        &entity,
                        move |_, _, _, _, offer| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            order.lock().unwrap().push("be:65536");
                            offer.offer(|| Box::new(Marked(42)));
                        },
                    );
                }

                assert!(registrar.refused.is_empty(), "{:?}", registrar.refused);
                install_registrar(registrar, &rule_schema()).unwrap();
                mod_vocab::__private::resolve_block_keys(["test:target"], |_| {
                    sunrise_mod_runtime::block_definitions::BlockDefinition::new(
                        &[&Default::default()],
                        0,
                    )
                    .map_err(|e| format!("{e:?}"))
                })
                .unwrap();

                let pos = [0, 64, 0];
                let mut world = FakeHost::new();
                world.level.set_block(pos, "test:target");
                world.level.block_entity(pos).of_type("test:target");

                assert!(
                    block_providers(&ITEMS, &world, on(pos, None))
                        .unwrap()
                        .ask(&mut world, true)
                        .is_none()
                );
                assert_eq!(block_calls.swap(0, Ordering::Relaxed), 1);
                assert_eq!(entity_calls.swap(0, Ordering::Relaxed), 1);
                assert_eq!(
                    std::mem::take(&mut *boundary_order.lock().unwrap()),
                    vec!["block:before_game", "be:before_game"]
                );

                let found = here()
                    .capability(&mut world, &ITEMS, pos, None)
                    .expect("final provider past 65,535 should answer");
                assert_eq!(found.owner(), "enabled");
                assert_eq!(found.size(&mut world), 42);
                assert_eq!(block_calls.load(Ordering::Relaxed), 65_536);
                assert_eq!(entity_calls.load(Ordering::Relaxed), 65_536);
                assert_eq!(
                    *boundary_order.lock().unwrap(),
                    vec![
                        "block:before_game",
                        "be:before_game",
                        "be:65534",
                        "block:65535",
                        "block:65536",
                        "be:65536",
                    ]
                );
            },
        );
    }

    #[test]
    pub(crate) fn disabled_capability_providers_are_skipped_and_resume() {
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marked(99)))
                });
                r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                    Some(Box::new(Marked(99)))
                });
                r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _, ()| {
                    Some(Box::new(Marked(99)))
                });
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new(
                "doubler",
                Box::new(Doubler {
                    mock_subjects: true,
                }),
            ),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();

        // These lookup objects keep their indexed registrations, not a snapshot of enabled mods.
        let at = BlockSide {
            level: here(),
            pos: [7, 64, 0],
            side: None,
        };
        let direct_block = || block_providers(&ITEMS, &ordered_world(), at).unwrap();
        let robot = EntityRef::new(WorldEpoch::new(1), 3);
        let mut player = player_world(None);
        let direct_entity =
            || entity_providers(&ENTITY_ITEMS, &player_world(None), robot, ()).unwrap();
        let block_before_disable = direct_block();
        let entity_before_disable = direct_entity();
        let check = |without_game: Option<usize>, with_game: usize, robot_size: Option<usize>| {
            let host: &mut dyn ServerHost = &mut ordered_world();
            let block = here().capability(host, &ITEMS, [7, 64, 0], None);
            assert_eq!(block.map(|found| found.size(host)), without_game);
            let block = here().capability(host, &ITEMS, [9, 64, 0], None).unwrap();
            assert_eq!(block.size(host), with_game);
            assert_eq!(
                direct_block()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                without_game
            );
            assert_eq!(
                direct_block().ask(host, true).map(|found| found.size(host)),
                None
            );
            for (damage, expected) in [(0, without_game), (1, Some(with_game))] {
                let host: &mut dyn ServerHost =
                    &mut HeldItem(stack("test:ordered_item", 1, damage));
                let item = HAND.capability(host, &ITEM_ITEMS);
                assert_eq!(item.map(|found| found.size(host)), expected);
            }
            let host: &mut dyn ServerHost = &mut player_world(None);
            let entity = robot.capability(host, &ENTITY_ITEMS, ());
            assert_eq!(entity.map(|found| found.size(host)), robot_size);
            assert_eq!(
                direct_entity()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                robot_size
            );
        };
        // At x = 9 Doubler replaces the game's block provider, while the item with damage 1
        // uses the game's handler (27), so check that special block answer separately.
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [9, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            3
        );
        assert!(crate::__private::set_mod_enabled("doubler", false));
        check(Some(99), 27, Some(99));
        assert_eq!(
            block_before_disable
                .ask(&mut ordered_world(), false)
                .unwrap()
                .size(&mut ordered_world()),
            99
        );
        let entity = entity_before_disable.ask(&mut player, false).unwrap();
        assert_eq!(entity.size(&mut player), 99);
        assert!(crate::__private::set_mod_enabled("later", false));
        check(None, 27, None);
        assert!(crate::__private::set_mod_enabled("later", true));
        check(Some(99), 27, Some(99));
        assert!(crate::__private::set_mod_enabled("doubler", true));
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            1
        );
        assert_eq!(
            robot
                .capability(&mut player, &ENTITY_ITEMS, ())
                .unwrap()
                .size(&mut player),
            3
        );
        let host: &mut dyn ServerHost = &mut HeldItem(stack("test:ordered_item", 1, 0));
        assert_eq!(HAND.capability(host, &ITEM_ITEMS).unwrap().size(host), 1);
    }

    #[cfg(test)]
    const SPARSE_POS: [i32; 3] = [0, 64, 0];
    #[cfg(test)]
    const SPARSE_HOLDER: Holder = Holder::Chunk([0, 0]);
    #[cfg(test)]
    const SPARSE_ITEMS_SIZE: usize = usize::MAX / 2;

    #[cfg(test)]
    fn with_sparse_providers(run: impl FnOnce(&mut FakeHost, LevelRef)) {
        struct Sparse;
        #[mod_api::__subscriptions]
        impl Hooks for Sparse {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let items_key = r
                    .register_attachment_type(AttachmentDescriptor::new(
                        "test:sparse_items",
                        StackList::<ItemResource>::new,
                    ))
                    .unwrap();
                let fluids_key = r
                    .register_attachment_type(
                        AttachmentDescriptor::new(
                            "test:sparse_fluids",
                            StackList::<FluidResource>::new,
                        )
                        .serialize(FLUID_STACKS),
                    )
                    .unwrap();
                let kind = r.block_entity_type_key("test:sparse");
                r.block_entity_capability(&ITEMS, &kind, move |_, _, _, _, offer| {
                    offer.offer(|| {
                        Box::new(AttachedItems::new(
                            &items_key,
                            SPARSE_HOLDER,
                            SPARSE_ITEMS_SIZE,
                            4,
                            |_| true,
                        ))
                    });
                });
                r.block_entity_capability_before_game(&FLUIDS, &kind, move |_, _, _, _, offer| {
                    offer.offer(|| {
                        Box::new(AttachedFluids::new(
                            &fluids_key,
                            SPARSE_HOLDER,
                            usize::MAX,
                            1000,
                            |_| true,
                        ))
                    });
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("test", Box::new(Sparse))].into();
        crate::testing::with_mods(mods.clone(), || {
            install_parts_fixture(&mods).unwrap();
            let mut host = FakeHost::new();
            let level = host.level.level();
            host.level.block_entity(SPARSE_POS).of_type("test:sparse");
            run(&mut host, level);
        });
    }

    #[test]
    fn sparse_items_provider_adapts_to_bulk_storage() {
        use crate::prelude::ItemsHost;
        use std::ops::ControlFlow;

        with_sparse_providers(|host, level| {
            let id = Identifier::parse("minecraft:dirt").unwrap();
            let cobble = Identifier::parse("minecraft:cobblestone").unwrap();
            host.registries
                .resolve_items([id.clone(), cobble.clone()])
                .unwrap();
            host.registries.item_component(
                "minecraft:dirt",
                "minecraft:max_stack_size",
                Tag::Int(2),
            );
            let dirt = ItemResource::of(&ItemStack::new(host.item_key(&id).unwrap(), 1).unwrap());
            let cobble =
                ItemResource::of(&ItemStack::new(host.item_key(&cobble).unwrap(), 1).unwrap());
            let items = level.capability(host, &ITEMS, SPARSE_POS, None).unwrap();
            let storage = level.capability(host, &STORAGE, SPARSE_POS, None).unwrap();
            let last = SPARSE_ITEMS_SIZE - 1;
            let total_capacity = 2 * SPARSE_ITEMS_SIZE as u64;
            assert_eq!(storage.amount(host, &dirt), Ok(0));
            assert_eq!(storage.space_for(host, &dirt), Ok(total_capacity));
            assert_eq!(
                storage.space_for(host, &cobble),
                (SPARSE_ITEMS_SIZE as u64)
                    .checked_mul(4)
                    .ok_or(StorageError::Overflow)
            );
            host.transaction(|host, tx| {
                assert_eq!(storage.insert(host, tx, &dirt, 1), Ok(1));
                assert_eq!(items.extract_at(host, tx, 0, &dirt, 1), Ok(1));
                assert_eq!(items.insert_at(host, tx, last, &dirt, 1), Ok(1));
                assert_eq!(storage.insert(host, tx, &dirt, 2), Ok(2));
                assert_eq!(items.amount(host, last), 2, "top up before filling a gap");
                assert_eq!(items.amount(host, 0), 1);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(storage.amount(host, &dirt), Ok(3));
            assert_eq!(storage.space_for(host, &dirt), Ok(total_capacity - 3));
            let mut entries = Vec::new();
            storage
                .visit_entries(host, &mut |resource, amount| {
                    entries.push((resource.clone(), amount));
                    ControlFlow::Continue(())
                })
                .unwrap();
            assert_eq!(entries, [(dirt.clone(), 1), (dirt.clone(), 2)]);
            assert_eq!(
                host.transaction(|host, tx| Ok::<_, ()>(storage.extract(host, tx, &dirt, 2))),
                Ok(Ok(2))
            );
            assert_eq!(items.amount(host, last), 1);
        });
    }

    #[test]
    fn picking_up_sparse_fluids_moves_only_the_first_occupied_tank() {
        use crate::fluid_util::try_pickup_fluid;
        use crate::prelude::BlockStatesHost;
        use crate::testing::FakeTank;

        with_sparse_providers(|host, level| {
            let destination = [1, 64, 0];
            host.registries.resolve_fluids(["minecraft:water"]).unwrap();
            let key = host.fluid_key("minecraft:water").unwrap();
            let water = FluidResource::of(&FluidStack::new(key, 1));
            host.level
                .block_entity(destination)
                .tanks
                .push(FakeTank::new(1000));
            let target = level.capability(host, &FLUIDS, destination, None).unwrap();
            let tanks = level.capability(host, &FLUIDS, SPARSE_POS, None).unwrap();
            assert_eq!(
                try_pickup_fluid(host, &target, None, level, SPARSE_POS, None),
                None
            );
            assert!(host.game_events.is_empty());
            let high = i32::MAX as usize;
            host.transaction(|host, tx| {
                assert_eq!(tanks.insert_at(host, tx, high, &water, 500), Ok(500));
                assert_eq!(tanks.insert_at(host, tx, high - 1, &water, 250), Ok(250));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(
                try_pickup_fluid(host, &target, None, level, SPARSE_POS, None),
                Some(water.to_stack(250))
            );
            assert_eq!(target.amount(host, 0), 250);
            assert_eq!(tanks.amount(host, high), 500);
            assert_eq!(host.game_events.len(), 1);
            assert_eq!(host.game_events[0].event, "minecraft:fluid_pickup");
        });
    }

    #[test]
    fn item_capability_presence_selects_without_constructing() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct Conditional(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Conditional {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let tank = r.block_entity_type_key("test:tank");
                let open = r.block_key("test:open");
                let _ = r.block_key("test:closed");
                let built = self.0.clone();
                r.block_entity_capability_before_game(
                    &ITEMS,
                    &tank,
                    move |level, at, state, _, offer| {
                        assert_eq!(level.block_state(at.pos), Some(state));
                        if at.side.is_none() && state.block() == open {
                            offer.offer(|| {
                                built.fetch_add(1, Ordering::Relaxed);
                                Box::new(Marked(1))
                            });
                        }
                    },
                );
            }
        }
        struct Later(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let tank = r.block_entity_type_key("test:tank");
                let built = self.0.clone();
                r.block_entity_capability(&ITEMS, &tank, move |_, _, _, _, offer| {
                    offer.offer(|| {
                        built.fetch_add(1, Ordering::Relaxed);
                        Box::new(Marked(2))
                    });
                });
                let port = r.block_key("test:port");
                let built = self.0.clone();
                r.block_capability(&ITEMS, &port, move |_, at, _, entity, offer| {
                    if entity.is_none() && at.side.is_none() {
                        offer.offer(|| {
                            built.fetch_add(1, Ordering::Relaxed);
                            Box::new(Marked(3))
                        });
                    }
                });
            }
        }

        let built = Arc::new(AtomicUsize::new(0));
        let mods: Mods = vec![
            ModEntry::new("first", Box::new(Conditional(built.clone()))),
            ModEntry::new("later", Box::new(Later(built.clone()))),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
        let open = [1, 64, 0];
        let closed = [2, 64, 0];
        let port = [3, 64, 0];
        let mut host = FakeHost::new();
        for (pos, block) in [
            (open, "test:open"),
            (closed, "test:closed"),
            (port, "test:port"),
        ] {
            host.level.set_block(pos, block);
        }
        for pos in [open, closed] {
            host.level.block_entity(pos).of_type("test:tank");
        }
        for (enabled, pos, side, slots) in [
            (true, open, None, Some(1)),
            (true, open, Some(Direction::North), Some(2)),
            (true, closed, None, Some(2)),
            (true, port, None, Some(3)),
            (true, port, Some(Direction::North), None),
            (false, open, None, Some(2)),
        ] {
            crate::__private::set_mod_enabled("first", enabled);
            let before = built.load(Ordering::Relaxed);
            assert_eq!(host.has_item_capability(pos, side), Some(slots.is_some()));
            assert_eq!(built.load(Ordering::Relaxed), before);
            let capability = here().capability(&mut host, &ITEMS, pos, side);
            assert_eq!(capability.map(|cap| cap.size(&mut host)), slots);
            assert_eq!(
                built.load(Ordering::Relaxed),
                before + usize::from(slots.is_some())
            );
        }
    }

    #[test]
    fn removing_a_block_entity_before_provider_dispatch_skips_its_providers() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Providers(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Providers {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let target = r.block_entity_type_key("test:removable");
                let visits = Arc::clone(&self.0);
                r.block_entity_capability_before_game(&ITEMS, &target, move |_, _, _, _, _| {
                    visits.fetch_add(1, Ordering::Relaxed);
                });
                let visits = Arc::clone(&self.0);
                r.block_entity_capability(&ITEMS, &target, move |_, _, _, _, offer| {
                    visits.fetch_add(1, Ordering::Relaxed);
                    offer.offer(|| Box::new(Marked(99)));
                });
            }
        }

        let visits = Arc::new(AtomicUsize::new(0));
        crate::testing::with_mods(
            vec![ModEntry::new(
                "provider",
                Box::new(Providers(visits.clone())),
            )]
            .into(),
            || {
                install_parts(crate::__private::mods(), &rule_schema(), |_| false).unwrap();
                let mut host = FakeHost::new();
                let pos = [0, 64, 0];
                host.level.block_entity(pos).of_type("test:removable");
                let at = on(pos, None);
                let selected = block_providers(&ITEMS, &host, at).unwrap();
                assert_eq!(selected.ask(&mut host, false).unwrap().size(&mut host), 99);
                let selected = block_providers(&ITEMS, &host, at).unwrap();

                // Block discovery has only a shared level borrow; removal can happen between
                // selecting the provider list and lending the site for dispatch.
                assert!(host.level.remove_block_entity(pos).is_some());
                assert!(selected.ask(&mut host, false).is_none());
                assert!(here().capability(&mut host, &ITEMS, pos, None).is_none());
                assert_eq!(visits.load(Ordering::Relaxed), 2);
            },
        );
    }

    #[test]
    pub(crate) fn disabling_a_provider_during_lookup_does_not_skip_the_next_mod() {
        struct SwitchOff;
        #[mod_api::__subscriptions]
        impl Hooks for SwitchOff {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _, _, _, _| {
                    assert!(crate::__private::set_mod_enabled("switch", false));
                });
                r.block_entity_capability(&ITEMS, &ordered, |_, _, _, _, _| {
                    panic!("a disabled mod's later provider must not run")
                });
            }
        }
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marked(99)))
                });
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new("switch", Box::new(SwitchOff)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
        let found = here()
            .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
            .unwrap();
        assert_eq!(
            (found.owner(), found.size(&mut ordered_world())),
            ("later".into(), 99)
        );
    }

    #[test]
    pub(crate) fn providers_are_asked_ahead_of_the_game_then_the_game_then_the_mods_in_order() {
        game_thread();
        let host: &mut dyn ServerHost = &mut ordered_world();
        let answer = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            (
                mod_vocab::__private::parts_installation::capability_is_owned(&found),
                found.size(host),
            )
        };
        // No game slots: the first mod provider that answers; the one before it declined.
        assert_eq!(answer(host, 7), (true, 1));
        // The game's slots come before the mods' providers.
        assert_eq!(answer(host, 8), (false, 27));
        // Providers registered before the game come first where they answer, in registration
        // order: at x = 9 both answer, at x = 10 only the second.
        assert_eq!(answer(host, 9), (true, 3));
        assert_eq!(answer(host, 10), (true, 4));
        // The game's own lookup takes a mod's handler only where the game has none or a mod
        // comes first.
        let mod_only = |host: &mut dyn ServerHost, x| {
            let at = BlockSide {
                level: here(),
                pos: [x, 64, 0],
                side: None,
            };
            let game_slots = backend_of(&*host)
                .and_then(|backend| backend.container_size(at))
                .is_some();
            let providers = block_providers(&ITEMS, host, at)?;
            providers
                .ask(host, game_slots)
                .map(|found| found.size(host))
        };
        assert_eq!(mod_only(host, 7), Some(1));
        assert_eq!(mod_only(host, 8), None);
        assert_eq!(mod_only(host, 9), Some(3));
        // With no provider for the block entity's type, or no type (a vanilla one), there are
        // no providers to ask, so the game needn't work out its own answer: a hopper's lookup
        // costs one table check.
        let at = BlockSide {
            level: here(),
            pos: [8, 64, 0],
            side: None,
        };
        let mut vanilla = FakeHost::new();
        vanilla.level.block_entity(at.pos);
        assert!(block_providers(&ITEMS, &vanilla, at).is_none());
        let mut unregistered = FakeHost::new();
        unregistered
            .level
            .block_entity(at.pos)
            .of_type("test:unregistered");
        assert!(block_providers(&ITEMS, &unregistered, at).is_none());

        // Item providers in the same order, for the item in a player's hand: with no game slots
        // (damage 0), the first mod's that answers; with them (damage 1), the game's; for two
        // items, the one ahead of the game.
        let item = |damage, count| {
            let mut host = HeldItem(stack("test:ordered_item", count, damage));
            let host: &mut dyn ServerHost = &mut host;
            let found = HAND.capability(host, &ITEM_ITEMS).unwrap();
            (
                mod_vocab::__private::parts_installation::capability_is_owned(&found),
                found.size(host),
            )
        };
        assert_eq!(item(0, 1), (true, 1));
        assert_eq!(item(1, 1), (false, 27));
        assert_eq!(item(1, 2), (true, 3));
    }

    #[test]
    fn panicking_before_game_provider_is_isolated_to_its_registration() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Empty;
        #[mod_api::__subscriptions]
        impl Hooks for Empty {}

        crate::testing::with_mods(
            vec![
                ModEntry::new("faulty", Box::new(Empty)),
                ModEntry::new("healthy", Box::new(Empty)),
            ]
            .into(),
            || {
                let bad_calls = Arc::new(AtomicUsize::new(0));
                let none_calls = Arc::new(AtomicUsize::new(0));
                let winner_calls = Arc::new(AtomicUsize::new(0));
                let sibling_calls = Arc::new(AtomicUsize::new(0));

                let mut registrar = PartRegistrar::new("faulty");
                let target = registrar.block_entity_type_key("test:panic_target");
                let sibling = registrar.block_entity_type_key("test:sibling_target");

                {
                    let calls = Arc::clone(&bad_calls);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability_before_game(
                        &ITEMS,
                        &target,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            panic!("boom in before-game capability provider");
                        },
                    );
                }
                {
                    let calls = Arc::clone(&sibling_calls);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability(
                        &ITEMS,
                        &sibling,
                        move |_, _, _, _, offer| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            offer.offer(|| Box::new(Marked(55)));
                        },
                    );
                }

                registrar.mod_id = "healthy";
                registrar.mod_index = 1;
                {
                    let calls = Arc::clone(&none_calls);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability_before_game(
                        &ITEMS,
                        &target,
                        move |_, _, _, _, _| {
                            calls.fetch_add(1, Ordering::Relaxed);
                        },
                    );
                }
                {
                    let calls = Arc::clone(&winner_calls);
                    (&mut registrar as &mut dyn Registrar).block_entity_capability(
                        &ITEMS,
                        &target,
                        move |_, _, _, _, offer| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            offer.offer(|| Box::new(Marked(77)));
                        },
                    );
                }

                assert!(registrar.refused.is_empty(), "{:?}", registrar.refused);
                install_registrar(registrar, &rule_schema()).unwrap();

                let target_pos = [0, 64, 0];
                let sibling_pos = [1, 64, 0];
                let mut world = FakeHost::new();
                world
                    .level
                    .block_entity(target_pos)
                    .of_type("test:panic_target");
                world
                    .level
                    .block_entity(sibling_pos)
                    .of_type("test:sibling_target");

                for visit in [1, 2] {
                    let found = here()
                        .capability(&mut world, &ITEMS, target_pos, None)
                        .expect("later legitimate provider should win");
                    assert_eq!(found.owner(), "healthy");
                    assert_eq!(found.size(&mut world), 77);
                    assert_eq!(bad_calls.load(Ordering::Relaxed), 1);
                    assert_eq!(none_calls.load(Ordering::Relaxed), visit);
                    assert_eq!(winner_calls.load(Ordering::Relaxed), visit);
                }

                let same_mod = here()
                    .capability(&mut world, &ITEMS, sibling_pos, None)
                    .expect("separately registered callback of the same mod should still work");
                assert_eq!(same_mod.owner(), "faulty");
                assert_eq!(same_mod.size(&mut world), 55);
                assert_eq!(sibling_calls.load(Ordering::Relaxed), 1);
            },
        );
    }
}
