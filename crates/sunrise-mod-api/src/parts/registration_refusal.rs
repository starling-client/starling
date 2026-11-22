//! Native registration diagnostics, savepoint, and rollback.
use super::*;

impl DiagnosticsRegistrar for PartRegistrar {
    fn refuse(&mut self, reason: &str) {
        self.current_refusals.push(reason.to_owned());
    }

    fn warn(&mut self, key: &str, args: &[&str]) {
        self.current_warnings.push(LoadingMessage::new(key, args));
    }
}

impl PartRegistrar {
    pub(crate) fn registration_checkpoint(&self) -> impl FnOnce(&mut Self) + use<> {
        let parts = self.parts.clone();
        let part_states = self.part_states.clone();
        let attachment_types = self.attachment_types.clone();
        let attachment_owners = self.attachment_owners.clone();
        let tags = self.tags.clone();
        let keys = self.registry_keys.borrow().clone();
        let menu_screen_factories = self.menu_screen_factories.clone();
        let menu_recipe_books = self.menu_recipe_books.clone();
        let recipe_book_types = self.recipe_book_types.clone();
        let recipe_book_fields = self.recipe_book_fields.clone();
        let recipe_book_filters = self.recipe_book_filters.clone();
        let registered = self.registered;
        let entity_selector_bytes = self.entity_selector_bytes;
        let block_entity_transforms = self.block_entity_transforms.clone();
        // Append-only tables retain previous mods' entries without requiring
        // Clone on runtime callback objects or their eventual frozen caches.
        macro_rules! checkpoint {
            ($($field:ident),* $(,)?) => {{
                $(let $field = self.$field.len();)*
                move |registrar: &mut Self| {
                    let owner = registrar.mod_id;
                    $(let $field = registrar.$field.split_off($field);)*
                    let rejected_parts = std::mem::replace(&mut registrar.parts, parts);
                    let rejected_tags = std::mem::replace(&mut registrar.tags, tags);
                    let rejected_transforms = std::mem::replace(
                        &mut registrar.block_entity_transforms,
                        block_entity_transforms,
                    );
                    let rejected_factories = std::mem::replace(
                        &mut registrar.menu_screen_factories,
                        menu_screen_factories,
                    );
                    registrar.part_states = part_states;
                    registrar.attachment_types = attachment_types;
                    registrar.attachment_owners = attachment_owners;
                    *registrar.registry_keys.borrow_mut() = keys;
                    registrar.menu_recipe_books = menu_recipe_books;
                    registrar.recipe_book_types = recipe_book_types;
                    registrar.recipe_book_fields = recipe_book_fields;
                    registrar.recipe_book_filters = recipe_book_filters;
                    registrar.registered = registered;
                    registrar.entity_selector_bytes = entity_selector_bytes;
                    // The checkpoint is fully restored before destroying even one value.
                    // Drop individual owned entries, never a collection containing multiple
                    // mod destructors that could panic while unwinding the first destructor.
                    $(discard_startup_registrations(owner, $field);)*
                    discard_startup_registrations(owner, rejected_tags.blocks);
                    discard_startup_registrations(owner, rejected_tags.items);
                    discard_startup_registrations(owner, rejected_transforms.into_values());
                    discard_startup_registrations(
                        owner,
                        rejected_factories.into_values(),
                    );
                    discard_rejected_parts(owner, rejected_parts);
                }
            }};
        }
        checkpoint!(
            game_rule_categories,
            game_rules,
            custom_stats,
            custom_stat_owners,
            entity_tick_each,
            attachment_uses,
            block_entity_types,
            entity_attributes,
            general_entity_types,
            world_fluids,
            fluid_interactions,
            entity_attribute_modifications,
            attributes,
            attribute_widenings,
            spawn_placements,
            entity_part_states,
            biome_regions,
            biome_modifiers,
            datapack_registries,
            controllers,
            arm_poses,
            consume_effects,
            structure_types,
            feature_types,
            loot_modifier_serializers,
            loot_function_types,
            loot_condition_types,
            dimensions,
            chunk_generators,
            biome_sources,
            refused
        )
    }
}

/// The restored snapshot still holds the earlier mods' Arcs. Drop each rejected callback
/// independently so a second destructor panic cannot unwind through the first one.
fn discard_rejected_parts(owner: &'static str, parts: Parts) {
    macro_rules! discard {
        ($($values:expr),* $(,)?) => { $(discard_startup_registrations(owner, $values);)* };
    }
    discard!(
        parts.blocks.into_values().flatten(),
        parts.block_entities.into_values().flatten(),
        parts.block_entity_kinds.into_values().flatten(),
        parts.items.into_values().flatten(),
        parts.client_item_uses.into_values(),
        parts.menus.into_values().flatten(),
        parts.effects.into_values().flatten(),
        parts.fluids.into_values().flatten(),
        parts.entities.into_values().flatten(),
        parts
            .capabilities
            .into_values()
            .flat_map(HashMap::into_values)
            .flatten(),
        parts
            .block_capabilities
            .into_values()
            .flat_map(HashMap::into_values)
            .flatten(),
        parts.default_item_containers.into_values().flatten(),
        parts
            .item_capabilities
            .into_values()
            .flat_map(HashMap::into_values)
            .flatten(),
        parts
            .entity_capabilities
            .into_values()
            .flat_map(HashMap::into_values)
            .flatten(),
        parts.place_providers.into_values(),
        parts.tooltip_appenders,
        parts.crash_sections,
        parts.command_arguments,
        parts.recipe_serializers.into_values(),
        parts.entity_selectors,
    );
    if let Some(payloads) = parts.payloads.and_then(Arc::into_inner) {
        for row in payloads.phases.into_iter().flat_map(HashMap::into_values) {
            discard!(row.server, row.client);
        }
        discard!(payloads.raw.into_values().flatten());
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Callback;
    impl crate::BlockPart for Callback {
        fn block_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }
    impl crate::BlockEntityPart for Callback {
        fn block_entity_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }

    fn region() -> crate::BiomeRegion {
        crate::BiomeRegion::new(
            "minecraft:overworld".into(),
            5,
            vec![crate::BiomePoint {
                biome: "minecraft:plains".into(),
                temperature: (0.0, 0.0),
                humidity: (0.0, 0.0),
                continentalness: (0.0, 0.0),
                erosion: (0.0, 0.0),
                depth: (0.0, 0.0),
                weirdness: (0.0, 0.0),
                offset: 0.0,
            }],
        )
    }

    struct Refuses;
    impl Hooks for Refuses {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.recipe_book_type_with_fields("broken:book", "reused_open", "reused_filter")
                .unwrap();
            r.declare_fluid("before");
            r.declare_biome_region("rejected", region()).unwrap();
            let kind = r
                .block_entity_type("callback")
                .blocks(&[])
                .transform(|data: &crate::Compound, _: crate::StructureTransform| Ok(data.clone()))
                .build();
            r.block_entity_part(&kind, Callback).unwrap();
            r.refuse("bootstrap failed: café\nmissing bridge");
            r.refuse("second refusal: missing config");
            r.warn("test.warn.bridge", &["missing"]);
            r.declare_fluid("foreign:after");
        }
    }
    struct Healthy;
    impl Hooks for Healthy {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.declare_fluid("soup");
            r.declare_biome_region("kept", region()).unwrap();
        }
    }

    struct Warns;
    impl Hooks for Warns {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.warn("test.warn.optional", &["jei"]);
            r.declare_fluid("soup");
        }
    }

    #[test]
    fn a_warning_keeps_the_mod_loading_and_is_published_for_it() {
        let mods = [ModEntry::new("warns", Box::new(Warns))];
        let registrar = super::super::register_all(&mods, |_| false);
        assert!(registrar.refused.is_empty());
        assert!(
            registrar
                .registry_keys
                .borrow()
                .declared_fluids
                .contains_key("warns:soup")
        );
        let [(owner, warnings)] = registrar.warnings.as_slice() else {
            panic!("expected warnings from one mod");
        };
        assert_eq!(*owner, "warns");
        let [warning] = warnings.as_slice() else {
            panic!("expected one warning");
        };
        assert_eq!(warning.key(), "test.warn.optional");
        assert_eq!(warning.args(), ["jei"]);
    }

    struct WarnsPastTheCap;
    impl Hooks for WarnsPastTheCap {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for i in 0..18 {
                r.warn("test.warn.many", &[&i.to_string()]);
            }
        }
    }

    #[test]
    fn warnings_are_published_under_the_mod_that_gave_them() {
        let mods = [
            ModEntry::new("warns", Box::new(Warns)),
            ModEntry::new("quiet", Box::new(Healthy)),
            ModEntry::new("many", Box::new(WarnsPastTheCap)),
        ];
        crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap();
        let warns = crate::__private::registration_warnings("warns");
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].key(), "test.warn.optional");
        assert_eq!(warns[0].args(), ["jei"]);
        assert!(crate::__private::registration_warnings("quiet").is_empty());
        let many = crate::__private::registration_warnings("many");
        assert_eq!(many.len(), 18);
        assert_eq!(many[0].key(), "test.warn.many");
        assert_eq!(many[0].args(), ["0"]);
        assert_eq!(many[17].args(), ["17"]);
    }

    #[test]
    fn refusal_rolls_back_only_its_owner_and_preserves_all_reasons() {
        let mods = [
            ModEntry::new("healthy", Box::new(Healthy)),
            ModEntry::new("broken", Box::new(Refuses)),
            ModEntry::new("later", Box::new(Healthy)),
        ];
        let mut registrar = super::super::register_all(&mods, |_| false);
        assert!(!registrar.recipe_book_types.contains_key("broken:book"));
        assert!(registrar.recipe_book_fields.is_empty());
        registrar.mod_id = "later";
        crate::RecipeBooksRegistrar::recipe_book_type_with_fields(
            &mut registrar,
            "later:book",
            "reused_open",
            "reused_filter",
        )
        .unwrap();
        assert_eq!(
            registrar.refused,
            [
                "mod broken refused registration: bootstrap failed: café\nmissing bridge",
                "mod broken refused registration: second refusal: missing config",
            ]
        );
        assert_eq!(registrar.warnings.len(), 1);
        let (owner, warnings) = &registrar.warnings[0];
        assert_eq!(*owner, "broken");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].key(), "test.warn.bridge");
        assert_eq!(warnings[0].args(), ["missing"]);
        assert_eq!(
            registrar
                .biome_regions
                .iter()
                .map(|d| (d.mod_id.as_str(), d.id.as_str(), d.region.weight))
                .collect::<Vec<_>>(),
            [("healthy", "healthy:kept", 5), ("later", "later:kept", 5)]
        );
        assert!(registrar.block_entity_types.is_empty());
        assert!(registrar.block_entity_transforms.is_empty());
        assert!(registrar.parts.block_entity_kinds.is_empty());
        let keys = registrar.registry_keys.borrow();
        assert!(keys.declared_fluids.contains_key("healthy:soup"));
        assert!(keys.declared_fluids.contains_key("later:soup"));
        assert!(!keys.declared_fluids.contains_key("broken:before"));
        assert!(!keys.declared_fluids.contains_key("foreign:after"));
    }

    #[test]
    fn long_utf8_refusals_and_warnings_are_kept_verbatim() {
        let long_reason = format!("{}é!", "r".repeat(4096));
        let long_key = format!("{}é!", "k".repeat(4096));
        let long_arg = format!("{}é!", "a".repeat(4096));
        struct Verbatim {
            reason: String,
            key: String,
            arg: String,
        }
        impl Hooks for Verbatim {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.refuse(&self.reason);
                r.refuse("");
                r.warn(&self.key, &[&self.arg]);
            }
        }
        let mods = [ModEntry::new(
            "verbatim",
            Box::new(Verbatim {
                reason: long_reason.clone(),
                key: long_key.clone(),
                arg: long_arg.clone(),
            }),
        )];
        let registrar = super::super::register_all(&mods, |_| false);
        assert_eq!(
            registrar.refused,
            [
                format!("mod verbatim refused registration: {long_reason}"),
                String::from("mod verbatim refused registration: "),
            ]
        );
        let [(owner, warnings)] = registrar.warnings.as_slice() else {
            panic!("expected warnings from one mod");
        };
        assert_eq!(*owner, "verbatim");
        let [warning] = warnings.as_slice() else {
            panic!("expected one warning");
        };
        assert_eq!(warning.key(), long_key);
        assert_eq!(warning.args(), [long_arg]);
    }

    #[test]
    fn panic_in_hooks_register_rolls_back_only_its_owner_and_queues_report() {
        crate::__private::initialize().expect("test registration requires the mod API runtime");
        struct Batch(Option<Arc<AtomicUsize>>);
        impl Hooks for Batch {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
                r.declare_biome_modifier(
                    "no_zombies",
                    crate::BiomeModifier {
                        biomes: vec!["minecraft:plains".into()],
                        operation: crate::BiomeModifierOperation::RemoveSpawns {
                            entity_types: vec!["minecraft:zombie".into()],
                        },
                    },
                )
                .unwrap();
                r.declare_datapack_registry(
                    "spell",
                    crate::DatapackRegistrySync::Required,
                    vec![crate::DatapackField::required(
                        "power",
                        crate::DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
                r.widen_attribute_bounds("minecraft:max_health", 1.0, 1000.0)
                    .unwrap();
                if let Some(calls) = &self.0 {
                    calls.fetch_add(1, Ordering::Relaxed);
                    r.declare_fluid("before");
                    r.declare_biome_region("aborted", region()).unwrap();
                    panic!("boom during registration");
                }
                Healthy.register(r, cx);
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: crate::Mods = vec![
            ModEntry::new("healthy", Box::new(Batch(None))),
            ModEntry::new("broken", Box::new(Batch(Some(calls.clone())))),
            ModEntry::new("later", Box::new(Batch(None))),
        ]
        .into();
        crate::testing::with_dispatch(mods, || {
            for _ in 0..2 {
                let registrar = super::super::register_all(mod_vocab::__private::mods(), |_| false);
                assert!(registrar.refused.is_empty());
                assert_eq!(
                    registrar
                        .biome_regions
                        .iter()
                        .map(|d| (d.mod_id.as_str(), d.id.as_str(), d.region.weight))
                        .collect::<Vec<_>>(),
                    [("healthy", "healthy:kept", 5), ("later", "later:kept", 5)]
                );
                assert_eq!(
                    registrar
                        .biome_modifiers
                        .iter()
                        .map(|d| d.id.as_str())
                        .collect::<Vec<_>>(),
                    ["healthy:no_zombies", "later:no_zombies"]
                );
                assert_eq!(
                    registrar
                        .datapack_registries
                        .iter()
                        .map(|d| d.key.as_str())
                        .collect::<Vec<_>>(),
                    ["healthy:spell", "later:spell"]
                );
                assert_eq!(
                    registrar
                        .attribute_widenings
                        .iter()
                        .map(|d| d.mod_id.as_str())
                        .collect::<Vec<_>>(),
                    ["healthy", "later"]
                );
                let keys = registrar.registry_keys.borrow();
                assert!(keys.declared_fluids.contains_key("healthy:soup"));
                assert!(keys.declared_fluids.contains_key("later:soup"));
                assert!(!keys.declared_fluids.contains_key("broken:before"));
            }
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                mod_vocab::__private::take_hook_panics(),
                [mod_vocab::__private::HookPanicReport {
                    mod_id: "broken",
                    hook: "register",
                }]
            );
        });
    }

    #[test]
    fn destructor_panics_in_rejected_parts_do_not_interrupt_registration() {
        crate::__private::initialize().expect("test registration requires the mod API runtime");
        struct PanicsOnDrop(Arc<AtomicUsize>);
        impl crate::BlockPart for PanicsOnDrop {
            fn block_subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
        }
        impl Drop for PanicsOnDrop {
            fn drop(&mut self) {
                // Count independent drops without aborting the test binary if a regression
                // drops a second Part while unwinding from the first.
                if !std::thread::panicking() {
                    self.0.fetch_add(1, Ordering::Relaxed);
                    panic!("rejected Part's destructor panicked");
                }
            }
        }
        struct Broken(Arc<AtomicUsize>);
        impl Hooks for Broken {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                for id in ["minecraft:stone", "minecraft:dirt"] {
                    let block = r.block_key(id);
                    r.block_part(block, PanicsOnDrop(self.0.clone()));
                }
                r.refuse("invalid setup");
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mods = vec![
            ModEntry::new("broken", Box::new(Broken(drops.clone()))),
            ModEntry::new("later", Box::new(Healthy)),
        ];
        crate::testing::with_dispatch(mods.into(), || {
            let registrar = super::super::register_all(mod_vocab::__private::mods(), |_| false);
            assert_eq!(drops.load(Ordering::Relaxed), 2);
            assert!(registrar.parts.blocks.is_empty());
            assert_eq!(
                registrar.refused,
                ["mod broken refused registration: invalid setup"]
            );
            assert_eq!(registrar.biome_regions[0].id, "later:kept");
            assert_eq!(
                mod_vocab::__private::take_hook_panics(),
                [mod_vocab::__private::HookPanicReport {
                    mod_id: "broken",
                    hook: "register",
                }]
            );
        });
    }

    #[test]
    fn ordinary_refusals_are_reported_even_when_register_panics() {
        thread_local! {
            static REPORTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        struct RefusalLog;
        impl log::Log for RefusalLog {
            fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
                meta.level() == log::Level::Warn && meta.target().ends_with("parts::shared")
            }
            fn log(&self, record: &log::Record<'_>) {
                if self.enabled(record.metadata()) {
                    REPORTS.with_borrow_mut(|reports| reports.push(record.args().to_string()));
                }
            }
            fn flush(&self) {}
        }
        struct RefusesThenPanics;
        impl Hooks for RefusesThenPanics {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let _ = r.widen_attribute_bounds("unnamespaced", 0.0, 1.0);
                r.refuse("configuration missing");
                panic!("reject registration after both refusals");
            }
        }
        log::set_logger(&RefusalLog).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        let mods = [ModEntry::new("refusing", Box::new(RefusesThenPanics))];
        let registrar = super::super::register_all(&mods, |_| false);
        assert!(registrar.refused.is_empty());
        assert_eq!(
            REPORTS.take(),
            [
                "mod refusing hook register refused: mod refusing attribute bounds of \"unnamespaced\": InvalidId",
                "mod refusing hook register refused: configuration missing",
            ]
        );
    }
}
