//! Qualify and validate code registry keys.
use super::*;

impl PartRegistrar {
    /// Bare names use this mod's ID; explicit namespaces remain unchanged.
    pub(crate) fn declaration_id(&self, name: &str) -> Result<String, String> {
        declaration_id(self.mod_id, name)
    }

    /// Whether the startup snapshot's built-in `registry` already contains `id`.
    pub(crate) fn has_builtin_entry(&self, registry: &str, id: &str) -> bool {
        use mod_vocab::__private::game::RegistrationContextNew;
        crate::RegistrationContext::new()
            .registry(registry)
            .is_some_and(|names| names.iter().any(|known| known == id))
    }

    /// Whether the startup snapshot already has a built-in registry keyed by `registry`.
    pub(crate) fn has_builtin_registry(&self, registry: &str) -> bool {
        use mod_vocab::__private::game::RegistrationContextNew;
        crate::RegistrationContext::new()
            .registry(registry)
            .is_some()
    }

    /// Whether the declaring package or any package supplying `id`'s namespace in
    /// `loaded_mods()` has a side other than `Side::Both`.
    ///
    /// Used by registrars whose output assigns client-required registry entries or
    /// network-synced schemas (`declare_general_entity`, `declare_attribute`,
    /// `widen_attribute_bounds`, `declare_datapack_registry`).
    pub(crate) fn declaration_side_not_both(&self, id: &str) -> bool {
        let loaded = mod_vocab::__private::parts_installation::loaded_mods();
        if loaded
            .get(self.mod_id)
            .is_some_and(|package| package.side() != crate::Side::Both)
        {
            return true;
        }
        let Some((namespace, _)) = id.split_once(':') else {
            return false;
        };
        loaded.iter().any(|package| {
            package.side() != crate::Side::Both
                && (package.id() == self.mod_id || package.served_by() == Some(self.mod_id))
                && package.namespaces().any(|owned| owned == namespace)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use mod_vocab::__private::game::RegistrationContextNew;

    fn with_porter(f: impl FnOnce()) {
        crate::load_state::with_test_package_inventory(
            vec![PackageInfo::new(
                "porter".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["porter".into(), "legacy".into()],
                vec![],
            )],
            f,
        );
    }

    /// A Press whose state has no codec: kept for the session by a native mod.
    struct Session(AttachmentKey<u32>);
    #[mod_api::sunrise_part]
    impl BlockPart for Session {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Session {}
    impl BlockEntityPartState for Session {
        type Data = u32;
        fn state_key(&self) -> &AttachmentKey<u32> {
            &self.0
        }
    }
    struct DeclaresSession;
    #[mod_api::__subscriptions]
    impl Hooks for DeclaresSession {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let session = r
                .register_attachment_type(crate::AttachmentDescriptor::new(
                    "porter:session",
                    || 0_u32,
                ))
                .unwrap();
            let kind = r.block_entity_type("press").blocks(&[]).build();
            r.block_entity_part_with_state(&kind, Session(session))
                .unwrap();
        }
    }

    /// Unsaved Part state does not require a codec.
    #[test]
    fn codec_less_part_state_is_accepted() {
        with_porter(|| {
            let native = [ModEntry::new("porter", Box::new(DeclaresSession))];
            assert!(
                super::super::register_all(&native, |_| false)
                    .refused
                    .is_empty()
            );
        });
    }

    #[test]
    fn part_state_keys_use_owned_namespaces_and_refuse_stale_keys() {
        struct Checks(AttachmentKey<i64>);
        #[mod_api::__subscriptions]
        impl Hooks for Checks {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let legacy_key = r
                    .register_attachment_type(AttachmentDescriptor::new("legacy:progress", || {
                        0_u32
                    }))
                    .unwrap();
                let primary_key = r
                    .register_attachment_type(AttachmentDescriptor::new("porter:progress", || {
                        0_u32
                    }))
                    .unwrap();
                let progress = PartStateDeclaration::of(&legacy_key);
                assert_eq!(
                    r.declare_block_entity_part_state("porter::Press", progress.clone()),
                    Ok(())
                );
                assert_eq!(
                    r.declare_block_entity_part_state("porter::Press", progress),
                    Ok(()),
                    "one key, one type, two parts"
                );
                let stale = PartStateDeclaration::of(&self.0);
                assert_eq!(
                    r.declare_block_entity_part_state("porter::Press", stale.clone()),
                    Err(stale.refused("porter::Press", PartStateRefusal::StaleKey))
                );
                assert_eq!(
                    r.declare_block_entity_part_state(
                        "porter::Press",
                        PartStateDeclaration::of(&primary_key),
                    ),
                    Ok(())
                );
            }
        }
        with_porter(|| {
            let stale_key = crate::testing::registered_attachment_fixture(
                "porter",
                AttachmentDescriptor::new("legacy:progress", || 0_i64),
            )
            .1;
            let r = super::super::register_all(
                &[ModEntry::new("porter", Box::new(Checks(stale_key)))],
                |_| false,
            );
            assert_eq!(
                r.refused.len(),
                1,
                "the stale key is the registration refusal"
            );
            assert!(r.refused[0].contains("legacy:progress"));
        });
    }

    #[test]
    fn declarations_keep_explicit_namespaces_and_qualify_bare_names() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            r.dimension_shape = |_| true;
            assert_eq!(r.declaration_id("bare").unwrap(), "porter:bare");
            assert_eq!(r.declaration_id("legacy:pot").unwrap(), "legacy:pot");
            assert_eq!(r.declaration_id("foreign:pot").unwrap(), "foreign:pot");
            assert_eq!(
                r.declaration_id("minecraft:custom_pot").unwrap(),
                "minecraft:custom_pot"
            );
            let kind = r.declare_block_entity_type("legacy:pot", vec!["legacy:pot".into()]);
            assert_ne!(
                kind,
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            assert_eq!(r.block_entity_types[0].1.id, "legacy:pot");
            let fluid = r.declare_fluid("legacy:soup");
            assert_ne!(
                fluid,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert!(
                r.registry_keys
                    .borrow()
                    .declared_fluids
                    .contains_key("legacy:soup")
            );
            let world_key = r.declare_world_fluid("legacy:broth", {
                let mut value: WorldFluid = Default::default();
                value.tick_delay = 20;
                value.drop_off = 2;
                value.slope_find_distance = 3;
                value.source_conversion = true;
                value.bucket = false;
                value
            });
            assert_ne!(
                world_key,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert_eq!(world_key, r.fluid_key("legacy:broth"));
            let keys = r.registry_keys.borrow();
            assert!(keys.declared_fluids.contains_key("legacy:broth"));
            assert!(keys.declared_fluids.contains_key("legacy:flowing_broth"));
            assert!(!keys.declared_fluids.contains_key("porter:broth"));
            drop(keys);
            r.declare_attribute(
                "legacy:stamina",
                1.0,
                0.0,
                10.0,
                true,
                Sentiment::Neutral,
                Default::default(),
            )
            .unwrap();
            r.declare_entity_attributes("legacy:cook", "minecraft:zombie", vec![])
                .unwrap();
            r.add_dimension("legacy:kitchen", DimensionSettings::default());
            assert!(r.recipe_book_type("legacy:kitchen").is_ok());
            assert_eq!(r.attributes[0].id, "legacy:stamina");
            assert_eq!(r.entity_attributes[0].id, "legacy:cook");
            assert_eq!(&*r.dimensions[0].id, "legacy:kitchen");
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            let foreign = r.declare_block_entity_type("foreign:pot", vec![]);
            assert_ne!(
                foreign,
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            assert_eq!(r.block_entity_types[1].0, "porter");
            assert_eq!(r.block_entity_types[1].1.id, "foreign:pot");
            assert_eq!(
                r.declaration_id("legacy:Bad Name"),
                Err("mod porter declares invalid identifier \"legacy:Bad Name\"".into())
            );
        });
    }

    #[test]
    fn bare_block_entity_type_blocks_are_the_declaring_mods() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            (&mut r as &mut dyn Registrar)
                .block_entity_type("press")
                .blocks(&["press", "legacy:pot", "minecraft:stone"])
                .build();
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            assert_eq!(r.block_entity_types[0].1.id, "porter:press");
            assert_eq!(
                r.block_entity_types[0].1.blocks,
                ["porter:press", "legacy:pot", "minecraft:stone"]
            );
        });
    }

    #[test]
    fn registration_open_is_true_inside_register_and_false_after() {
        struct ChecksPhase;
        #[mod_api::__subscriptions]
        impl Hooks for ChecksPhase {
            fn register(&self, _: &mut dyn Registrar, _: &RegistrationContext) {
                assert!(registration_open());
            }
        }
        let mods = [ModEntry::new("phase", Box::new(ChecksPhase))];
        assert!(!registration_open());
        super::super::register_all(&mods, |_| false);
        assert!(!registration_open());
    }

    #[test]
    fn late_block_declaration_is_refused_with_its_id() {
        struct Late;
        #[mod_api::__subscriptions]
        impl Hooks for Late {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                assert_eq!(
                    event.register(Block::new("late:pot", "minecraft:stone")),
                    Err(DeclarationError::RegistrationClosed {
                        id: "late:pot".into()
                    })
                );
            }
        }
        crate::testing::with_mods(vec![ModEntry::new("late", Box::new(Late))].into(), || {
            let mut blocks = Vec::new();
            // Call the engine dispatcher out of phase; a mod cannot retain the event.
            crate::dispatch_blocks(&RegistrationContext::new(), &mut |listen| {
                assert!(listen(&mut |block| {
                    blocks.push(block);
                    Ok(())
                }));
            });
            assert!(blocks.is_empty());
        });
    }

    #[test]
    fn panicking_block_declaration_drops_its_batch_and_is_skipped_next_time() {
        struct Declares(bool);
        #[crate::__subscriptions]
        impl Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new("stone_variant", "minecraft:stone"))
                    .unwrap();
                if self.0 {
                    panic!("declare then panic");
                }
            }
        }
        let mods = vec![
            ModEntry::new("broken", Box::new(Declares(true))),
            ModEntry::new("later", Box::new(Declares(false))),
        ]
        .into();
        crate::testing::with_mods(mods, || {
            let _registration = crate::__private::RegistrationGuard::open();
            let context = RegistrationContext::new();
            for expected in [[false, true].as_slice(), [true].as_slice()] {
                let mut outcomes = Vec::new();
                let mut published = Vec::new();
                crate::dispatch_blocks(&context, &mut |listen| {
                    let mut pending = Vec::new();
                    let completed = listen(&mut |declaration| {
                        pending.push(declaration.answer.id);
                        Ok(())
                    });
                    outcomes.push(completed);
                    if completed {
                        published.extend(pending);
                    }
                });
                assert_eq!(outcomes, expected);
                assert_eq!(published, ["later:stone_variant"]);
            }
            assert_eq!(
                mod_vocab::__private::take_hook_panics(),
                [mod_vocab::__private::HookPanicReport {
                    mod_id: "broken",
                    hook: "blocks",
                }]
            );
        });
    }

    #[test]
    fn blocks_and_items_keep_their_owner_and_report_bad_ids_at_the_call() {
        struct Declares;
        #[crate::__subscriptions]
        impl Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                let mut value = Block::new("foreign:pot", "minecraft:stone");
                value.block_item = Some("spoon".into());
                value.pick_item = Some("minecraft:stick".into());
                value.wall_block = Some("legacy:wall_pot".into());
                value.base_block = Some("soil".into());
                event.register(value).unwrap();
                assert_eq!(
                    event.register(Block::new("foreign:Bad Name", "minecraft:stone")),
                    Err(DeclarationError::InvalidId {
                        id: "foreign:Bad Name".into(),
                    })
                );
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                let mut value = Item::new("spoon", "minecraft:stick");
                value.crafting_remainder = Some("bowl".into());
                value.block = Some("minecraft:stone".into());
                event.register(value).unwrap();
                event
                    .register(Item::new("minecraft:custom_spoon", "minecraft:stick"))
                    .unwrap();
                assert_eq!(
                    event.register(Item::new("Bad Name", "minecraft:stick")),
                    Err(DeclarationError::InvalidId {
                        id: "porter:Bad Name".into()
                    })
                );
            }
        }
        let mods: Mods = vec![ModEntry::new("porter", Box::new(Declares))].into();
        crate::testing::with_mods(mods, || {
            with_porter(|| {
                let _registration = crate::__private::RegistrationGuard::open();
                let context = RegistrationContext::new();
                let mut blocks = Vec::new();
                let mut items = Vec::new();
                crate::dispatch_blocks(&context, &mut |listen| {
                    assert!(listen(&mut |block| {
                        blocks.push(block);
                        Ok(())
                    }));
                });
                crate::dispatch_items(&context, &mut |listen| {
                    assert!(listen(&mut |item| {
                        items.push(item);
                        Ok(())
                    }));
                });
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].mod_id, "porter");
                let pot = &blocks[0].answer;
                assert_eq!(pot.id, "foreign:pot");
                // Bare references use the declaring mod, not the declaration's other namespace.
                assert_eq!(pot.block_item.as_deref(), Some("porter:spoon"));
                assert_eq!(pot.pick_item.as_deref(), Some("minecraft:stick"));
                assert_eq!(pot.wall_block.as_deref(), Some("legacy:wall_pot"));
                assert_eq!(pot.base_block.as_deref(), Some("porter:soil"));
                assert_eq!(items.len(), 2);
                assert_eq!(items[1].mod_id, "porter");
                assert_eq!(items[1].answer.id, "minecraft:custom_spoon");
                let item = &items[0].answer;
                assert_eq!(item.id, "porter:spoon");
                assert_eq!(item.crafting_remainder.as_deref(), Some("porter:bowl"));
                assert_eq!(item.block.as_deref(), Some("minecraft:stone"));
                use crate::__private::{DeclarationKind, DeclarationRefusal};
                assert_eq!(
                    take_declaration_refusals(&context),
                    [
                        DeclarationRefusal::new(
                            "porter",
                            DeclarationKind::Block,
                            DeclarationError::InvalidId {
                                id: "foreign:Bad Name".into(),
                            },
                        ),
                        DeclarationRefusal::new(
                            "porter",
                            DeclarationKind::Item,
                            DeclarationError::InvalidId {
                                id: "porter:Bad Name".into()
                            },
                        ),
                    ]
                );
            })
        });
    }

    #[test]
    fn served_by_grants_namespace_ownership_only_to_the_serving_code_mod() {
        let packages = vec![
            PackageInfo::new(
                "horizon_java".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["horizon_java".into()],
                vec![],
            ),
            PackageInfo::new(
                "third".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["third".into()],
                vec![],
            ),
            PackageInfo::new(
                "runes".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["runes".into(), "runes_extra".into()],
                vec![],
            )
            .with_served_by(Some("horizon_java".into())),
            PackageInfo::new(
                "plain".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["plain".into()],
                vec![],
            ),
        ];

        struct HostDeclares;
        #[crate::__subscriptions]
        impl Hooks for HostDeclares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new("runes:altar", "minecraft:stone"))
                    .unwrap();
                event
                    .register(Block::new("runes_extra:pillar", "minecraft:stone"))
                    .unwrap();
                event
                    .register(Block::new("minecraft:altar", "minecraft:stone"))
                    .unwrap();
                event
                    .register(Block::new("plain:altar", "minecraft:stone"))
                    .unwrap();
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                event
                    .register(Item::new("runes:fire_rune", "minecraft:stick"))
                    .unwrap();
            }
        }
        struct ThirdDeclares;
        #[crate::__subscriptions]
        impl Hooks for ThirdDeclares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new("runes:stolen", "minecraft:stone"))
                    .unwrap();
            }
        }

        crate::load_state::with_test_package_inventory(packages, || {
            let mut host = PartRegistrar::new("horizon_java");
            assert_eq!(
                host.declaration_id("runes:crafting").unwrap(),
                "runes:crafting"
            );
            assert_eq!(
                host.declaration_id("runes_extra:infusing").unwrap(),
                "runes_extra:infusing"
            );
            assert_eq!(
                host.declaration_id("plain:block").as_deref(),
                Ok("plain:block")
            );
            assert_eq!(
                host.declaration_id("minecraft:custom").as_deref(),
                Ok("minecraft:custom")
            );
            assert_eq!(host.declaration_id("c:custom").as_deref(), Ok("c:custom"));
            assert_eq!(
                host.declaration_id("sunrise:custom").as_deref(),
                Ok("sunrise:custom")
            );

            (&mut host as &mut dyn Registrar).register_recipe_type("runes:crafting");
            (&mut host as &mut dyn Registrar)
                .register_raw_recipe_serializer("runes:crafting_json", "runes:crafting");
            (&mut host as &mut dyn Registrar).register_recipe_type("runes_extra:infusing");
            (&mut host as &mut dyn Registrar).register_raw_recipe_serializer(
                "runes_extra:infusing_json",
                "runes_extra:infusing",
            );
            let book = host.recipe_book_type("runes:book").unwrap();
            host.menu_recipe_book(
                "runes:altar",
                &book,
                "runes:crafting",
                crate::RecipeBookLayout::Furnace {
                    input: 0,
                    fuel: 1,
                    result: 2,
                },
            )
            .unwrap();
            assert!(host.refused.is_empty(), "{:?}", host.refused);

            let mut third = PartRegistrar::new("third");
            assert_eq!(
                third.declaration_id("runes:crafting").as_deref(),
                Ok("runes:crafting")
            );
            assert_eq!(
                third.declaration_id("runes_extra:infusing").as_deref(),
                Ok("runes_extra:infusing")
            );
            assert_eq!(
                third.declaration_id("minecraft:crafting").as_deref(),
                Ok("minecraft:crafting")
            );
            (&mut third as &mut dyn Registrar).register_recipe_type("runes:crafting");
            (&mut third as &mut dyn Registrar)
                .register_raw_recipe_serializer("runes:crafting_json", "runes:crafting");
            assert!(third.refused.is_empty(), "{:?}", third.refused);

            let mods: Mods = vec![
                ModEntry::new("horizon_java", Box::new(HostDeclares)),
                ModEntry::new("third", Box::new(ThirdDeclares)),
            ]
            .into();
            crate::testing::with_mods(mods, || {
                let _registration = crate::__private::RegistrationGuard::open();
                let context = RegistrationContext::new();
                let mut blocks = Vec::new();
                let mut items = Vec::new();
                crate::dispatch_blocks(&context, &mut |listen| {
                    assert!(listen(&mut |block| {
                        blocks.push(block);
                        Ok(())
                    }));
                });
                crate::dispatch_items(&context, &mut |listen| {
                    assert!(listen(&mut |item| {
                        items.push(item);
                        Ok(())
                    }));
                });
                assert_eq!(
                    blocks
                        .iter()
                        .map(|b| b.answer.id.as_str())
                        .collect::<Vec<_>>(),
                    [
                        "runes:altar",
                        "runes_extra:pillar",
                        "minecraft:altar",
                        "plain:altar",
                        "runes:stolen"
                    ]
                );
                assert_eq!(
                    items
                        .iter()
                        .map(|i| i.answer.id.as_str())
                        .collect::<Vec<_>>(),
                    ["runes:fire_rune"]
                );
                assert!(take_declaration_refusals(&context).is_empty());
            });
        });
    }
}
