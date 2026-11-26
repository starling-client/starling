//! Native behavioral coverage of the explicit default-provider domain cutover.
#[cfg(test)]
mod tests {
    use crate::{
        ENERGY, FLUIDS, Hooks, Host, ITEM_FLUIDS, ITEM_ITEMS, ITEMS, ModEntry, Registrar,
        RegistrationContext, STORAGE,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    };

    struct Defaults {
        slots: Arc<AtomicU32>,
        calls: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Defaults {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let machine = r.block_key("minecraft:test_machine");
            let backpack = r.item_key("defaults:backpack");
            let bucket = r.item_key("minecraft:water_bucket");
            let later = r.block_key("later:machine");
            assert!(!r.is_block_capability_registered(&ITEMS, &later));
            assert!(!r.is_block_capability_registered(&ITEMS, &machine));
            assert!(!r.is_block_capability_registered(&STORAGE, &machine));
            assert!(!r.is_block_capability_registered(&FLUIDS, &machine));
            assert!(!r.is_block_capability_registered(&ENERGY, &machine));
            assert!(!r.is_item_capability_registered(&ITEM_ITEMS, &backpack));
            assert!(r.is_item_capability_registered(&ITEM_FLUIDS, &bucket));
            // A registered provider can currently answer no slots/tanks/energy. Its type
            // domain is still registered, so a fallback must not replace it.
            r.block_container(&machine, Default::default());
            r.block_fluids(&machine, Default::default());
            r.block_energy(&machine, Default::default());
            assert!(r.is_block_capability_registered(&ITEMS, &machine));
            assert!(r.is_block_capability_registered(&STORAGE, &machine));
            assert!(r.is_block_capability_registered(&FLUIDS, &machine));
            assert!(r.is_block_capability_registered(&ENERGY, &machine));
            let slots = self.slots.clone();
            let calls = self.calls.clone();
            r.item_container(&backpack, move |_: &dyn Host| {
                calls.fetch_add(1, Ordering::SeqCst);
                let current = slots.load(Ordering::SeqCst);
                (current > 0).then_some(current)
            });
            assert!(r.is_item_capability_registered(&ITEM_ITEMS, &backpack));
            assert_eq!(self.calls.load(Ordering::SeqCst), 0);
        }
    }

    struct LaterDefaults;
    #[mod_api::__subscriptions]
    impl Hooks for LaterDefaults {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let earlier = r.block_key("minecraft:test_machine");
            let later = r.block_key("later:machine");
            assert!(r.is_block_capability_registered(&ITEMS, &earlier));
            assert!(!r.is_block_capability_registered(&ITEMS, &later));
            let replacement = crate::ModContainer {
                slots: 9,
                ..Default::default()
            };
            r.block_container(&earlier, replacement.clone());
            r.block_container(&later, replacement);
            assert!(r.is_block_capability_registered(&ITEMS, &later));
        }
    }

    #[test]
    fn native_default_domains_do_not_evaluate_dynamic_item_providers() {
        let slots = Arc::new(AtomicU32::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        crate::__private::part_tests::install_parts_fixture(&[
            ModEntry::new(
                "defaults",
                Box::new(Defaults {
                    slots: slots.clone(),
                    calls: calls.clone(),
                }),
            ),
            ModEntry::new("later", Box::new(LaterDefaults)),
        ])
        .unwrap();
        let host = crate::testing::FakeHost::new();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            mod_vocab::__private::parts_installation::item_container_slots(
                &host,
                "defaults:backpack"
            ),
            None
        );
        slots.store(7, Ordering::SeqCst);
        assert_eq!(
            mod_vocab::__private::parts_installation::item_container_slots(
                &host,
                "defaults:backpack"
            ),
            Some(7)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            mod_vocab::__private::parts_installation::block_container("minecraft:test_machine")
                .unwrap()
                .slots,
            0
        );
        assert_eq!(
            mod_vocab::__private::parts_installation::block_container("later:machine")
                .unwrap()
                .slots,
            9
        );
        assert!(
            mod_vocab::__private::parts_installation::block_fluids("minecraft:test_machine")
                .is_some()
        );
        assert!(
            mod_vocab::__private::parts_installation::block_energy("minecraft:test_machine")
                .is_some()
        );
    }

    #[test]
    fn native_block_container_refuses_slots_it_cannot_save() {
        struct Container {
            block: &'static str,
            slots: usize,
        }

        #[mod_api::__subscriptions]
        impl Hooks for Container {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let block = r.block_key(self.block);
                r.block_container(
                    &block,
                    crate::ModContainer {
                        slots: self.slots,
                        ..Default::default()
                    },
                );
            }
        }

        let max_saved_slots = 256;
        let oversized_slots = max_saved_slots + 1;
        let registrar = super::super::register_all(
            &[
                ModEntry::new(
                    "fitting",
                    Box::new(Container {
                        block: "fitting:machine",
                        slots: max_saved_slots,
                    }),
                ),
                ModEntry::new(
                    "oversized",
                    Box::new(Container {
                        block: "oversized:machine",
                        slots: oversized_slots,
                    }),
                ),
            ],
            |_| false,
        );
        assert_eq!(
            registrar.parts.default_block_containers["fitting:machine"][0]
                .1
                .slots,
            max_saved_slots
        );
        assert!(
            !registrar
                .parts
                .default_block_containers
                .contains_key("oversized:machine")
        );
        let [refusal] = registrar.refused.as_slice() else {
            panic!("expected one refusal, got {:?}", registrar.refused);
        };
        let oversized = oversized_slots.to_string();
        let limit = max_saved_slots.to_string();
        for field in ["oversized:machine", oversized.as_str(), limit.as_str()] {
            assert!(refusal.contains(field), "{refusal}");
        }
    }

    #[test]
    fn requested_entity_type_slots_resolve_without_renumbering_and_refuse_unknown_ids() {
        let mut registrar = super::super::PartRegistrar::new("requester");
        let registrar_api = &mut registrar as &mut dyn Registrar;
        let zombie = registrar_api.entity_type_key("minecraft:zombie");
        let pig = registrar_api.entity_type_key("minecraft:pig");
        assert_ne!(zombie, pig);
        super::super::install_registrar(registrar, &crate::__private::part_tests::rule_schema())
            .unwrap();
        mod_vocab::__private::resolve_entity_types(["minecraft:pig", "minecraft:zombie"], |_| None)
            .unwrap();
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(zombie).as_deref(),
            Some("minecraft:zombie")
        );
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(pig).as_deref(),
            Some("minecraft:pig")
        );
        assert_eq!(mod_vocab::__private::entity_type_keys(), [pig, zombie]);
        let errors =
            mod_vocab::__private::resolve_entity_types(["minecraft:pig"], |_| None).unwrap_err();
        assert_eq!(
            errors,
            ["mod requester requested unknown entity type minecraft:zombie"]
        );
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(zombie).as_deref(),
            Some("minecraft:zombie")
        );
    }

    #[test]
    fn panicking_dynamic_item_container_provider_is_isolated_to_its_registration() {
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

                let mut registrar = super::super::PartRegistrar::new("faulty");
                let backpack = (&mut registrar as &mut dyn Registrar).item_key("test:panic_bag");
                let pouch = (&mut registrar as &mut dyn Registrar).item_key("test:sibling_bag");
                {
                    let calls = Arc::clone(&bad_calls);
                    (&mut registrar as &mut dyn Registrar).item_container(
                        &backpack,
                        move |_: &dyn Host| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            panic!("boom in dynamic item container provider");
                        },
                    );
                }
                {
                    let calls = Arc::clone(&sibling_calls);
                    (&mut registrar as &mut dyn Registrar).item_container(
                        &pouch,
                        move |_: &dyn Host| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            Some(12)
                        },
                    );
                }

                registrar.mod_id = "healthy";
                registrar.mod_index = 1;
                {
                    let calls = Arc::clone(&none_calls);
                    (&mut registrar as &mut dyn Registrar).item_container(
                        &backpack,
                        move |_: &dyn Host| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            None
                        },
                    );
                }
                {
                    let calls = Arc::clone(&winner_calls);
                    (&mut registrar as &mut dyn Registrar).item_container(
                        &backpack,
                        move |_: &dyn Host| {
                            calls.fetch_add(1, Ordering::Relaxed);
                            Some(9)
                        },
                    );
                }

                assert!(registrar.refused.is_empty(), "{:?}", registrar.refused);
                super::super::install_registrar(
                    registrar,
                    &crate::__private::part_tests::rule_schema(),
                )
                .unwrap();

                let host = crate::testing::FakeHost::new();
                for visit in [1, 2] {
                    assert_eq!(
                        mod_vocab::__private::parts_installation::item_container_slots(
                            &host,
                            "test:panic_bag",
                        ),
                        Some(9)
                    );
                    assert_eq!(bad_calls.load(Ordering::Relaxed), 1);
                    assert_eq!(none_calls.load(Ordering::Relaxed), visit);
                    assert_eq!(winner_calls.load(Ordering::Relaxed), visit);
                }

                assert_eq!(
                    mod_vocab::__private::parts_installation::item_container_slots(
                        &host,
                        "test:sibling_bag",
                    ),
                    Some(12)
                );
                assert_eq!(sibling_calls.load(Ordering::Relaxed), 1);
            },
        );
    }

    #[test]
    fn block_container_refuses_more_slots_than_vanilla_saves() {
        let mut registrar = super::super::PartRegistrar::new("overflow");
        let rejected = (&mut registrar as &mut dyn Registrar).block_key("overflow:too_many");
        (&mut registrar as &mut dyn Registrar).block_container(
            &rejected,
            crate::ModContainer {
                slots: 257,
                ..Default::default()
            },
        );
        assert_eq!(registrar.refused.len(), 1);
        assert!(
            !registrar
                .parts
                .default_block_containers
                .contains_key("overflow:too_many")
        );
        assert!(
            !(&mut registrar as &mut dyn Registrar)
                .is_block_capability_registered(&ITEMS, &rejected)
        );

        let accepted = (&mut registrar as &mut dyn Registrar).block_key("overflow:max_saved");
        (&mut registrar as &mut dyn Registrar).block_container(
            &accepted,
            crate::ModContainer {
                slots: 256,
                ..Default::default()
            },
        );
        assert_eq!(registrar.refused.len(), 1);
        assert!(
            (&mut registrar as &mut dyn Registrar)
                .is_block_capability_registered(&ITEMS, &accepted)
        );
    }

    #[test]
    fn stock_storage_refuses_malformed_shapes_and_negative_energy() {
        let mut registrar = super::super::PartRegistrar::new("bad_storage");

        let mut bad_face: [Vec<usize>; 6] = Default::default();
        bad_face[0] = vec![2];
        let mut multi_bad_face: [Vec<usize>; 6] = Default::default();
        multi_bad_face[0] = vec![1];
        let bad_containers = [
            (
                "bad_storage:bad_face",
                crate::ModContainer {
                    slots: 2,
                    faces: bad_face,
                    ..Default::default()
                },
            ),
            (
                "bad_storage:bad_input",
                crate::ModContainer {
                    slots: 2,
                    inputs: vec![2],
                    ..Default::default()
                },
            ),
            (
                "bad_storage:bad_output",
                crate::ModContainer {
                    slots: 2,
                    outputs: vec![2],
                    ..Default::default()
                },
            ),
            (
                "bad_storage:bad_rules",
                crate::ModContainer {
                    slots: 1,
                    insert_rules: vec![
                        crate::ModContainerInsertRule::Any,
                        crate::ModContainerInsertRule::OnlyWhenEmpty,
                    ],
                    ..Default::default()
                },
            ),
            (
                "bad_storage:multi_bad",
                crate::ModContainer {
                    slots: 1,
                    faces: multi_bad_face,
                    inputs: vec![1],
                    outputs: vec![1],
                    insert_rules: vec![
                        crate::ModContainerInsertRule::Any,
                        crate::ModContainerInsertRule::Any,
                    ],
                    menu: None,
                },
            ),
        ];
        for (target, container) in bad_containers {
            let before = registrar.refused.len();
            let key = (&mut registrar as &mut dyn Registrar).block_key(target);
            (&mut registrar as &mut dyn Registrar).block_container(&key, container);
            assert_eq!(registrar.refused.len(), before + 1, "{target}");
            let msg = registrar.refused.last().unwrap();
            assert!(msg.contains("bad_storage") && msg.contains(target), "{msg}");
            assert!(
                !registrar
                    .parts
                    .default_block_containers
                    .contains_key(target)
            );
            assert!(
                !(&mut registrar as &mut dyn Registrar)
                    .is_block_capability_registered(&ITEMS, &key)
            );
            assert!(
                !(&mut registrar as &mut dyn Registrar)
                    .is_block_capability_registered(&STORAGE, &key)
            );
        }

        let mut bad_fluid_face = crate::ModFluidTanks::default();
        bad_fluid_face.tanks = vec![1000];
        bad_fluid_face.faces[0] = vec![1];
        let mut bad_fluid_input = crate::ModFluidTanks::default();
        bad_fluid_input.tanks = vec![1000];
        bad_fluid_input.inputs = vec![1];
        let mut bad_fluid_output = crate::ModFluidTanks::default();
        bad_fluid_output.tanks = vec![1000];
        bad_fluid_output.outputs = vec![1];
        for (target, fluids) in [
            ("bad_storage:fluid_face", bad_fluid_face),
            ("bad_storage:fluid_input", bad_fluid_input),
            ("bad_storage:fluid_output", bad_fluid_output),
        ] {
            let before = registrar.refused.len();
            let key = (&mut registrar as &mut dyn Registrar).block_key(target);
            (&mut registrar as &mut dyn Registrar).block_fluids(&key, fluids);
            assert_eq!(registrar.refused.len(), before + 1, "{target}");
            let msg = registrar.refused.last().unwrap();
            assert!(msg.contains("bad_storage") && msg.contains(target), "{msg}");
            assert!(!registrar.parts.default_block_fluids.contains_key(target));
            assert!(
                !(&mut registrar as &mut dyn Registrar)
                    .is_block_capability_registered(&FLUIDS, &key)
            );
        }

        for (target, energy) in [
            (
                "bad_storage:neg_cap",
                crate::ModEnergyBuffer {
                    capacity: -1,
                    max_insert: 0,
                    max_extract: 0,
                    insert_faces: [true; 6],
                    extract_faces: [true; 6],
                },
            ),
            (
                "bad_storage:neg_insert",
                crate::ModEnergyBuffer {
                    capacity: 0,
                    max_insert: -1,
                    max_extract: 0,
                    insert_faces: [true; 6],
                    extract_faces: [true; 6],
                },
            ),
            (
                "bad_storage:neg_extract",
                crate::ModEnergyBuffer {
                    capacity: 0,
                    max_insert: 0,
                    max_extract: -1,
                    insert_faces: [true; 6],
                    extract_faces: [true; 6],
                },
            ),
        ] {
            let before = registrar.refused.len();
            let key = (&mut registrar as &mut dyn Registrar).block_key(target);
            (&mut registrar as &mut dyn Registrar).block_energy(&key, energy);
            assert_eq!(registrar.refused.len(), before + 1, "{target}");
            let msg = registrar.refused.last().unwrap();
            assert!(msg.contains("bad_storage") && msg.contains(target), "{msg}");
            assert!(!registrar.parts.default_block_energy.contains_key(target));
            assert!(
                !(&mut registrar as &mut dyn Registrar)
                    .is_block_capability_registered(&ENERGY, &key)
            );
        }

        let refused_before_valid = registrar.refused.len();
        let vault = (&mut registrar as &mut dyn Registrar).block_key("bad_storage:valid_vault");
        let mut vault_faces: [Vec<usize>; 6] = Default::default();
        vault_faces[1] = vec![255];
        (&mut registrar as &mut dyn Registrar).block_container(
            &vault,
            crate::ModContainer {
                slots: 256,
                faces: vault_faces,
                inputs: vec![255],
                outputs: vec![255],
                insert_rules: Vec::new(),
                menu: None,
            },
        );
        let tank = (&mut registrar as &mut dyn Registrar).block_key("bad_storage:valid_tank");
        let mut fluids = crate::ModFluidTanks::default();
        fluids.tanks = vec![1000; 17];
        fluids.tanks[0] = 0;
        fluids.tanks[16] = 1_000_001;
        fluids.faces[1] = vec![0, 16];
        fluids.inputs = vec![0, 16];
        fluids.outputs = vec![0, 16];
        (&mut registrar as &mut dyn Registrar).block_fluids(&tank, fluids);
        let battery = (&mut registrar as &mut dyn Registrar).block_key("bad_storage:valid_battery");
        (&mut registrar as &mut dyn Registrar).block_energy(
            &battery,
            crate::ModEnergyBuffer {
                capacity: 0,
                max_insert: 500,
                max_extract: 500,
                insert_faces: [false; 6],
                extract_faces: [true; 6],
            },
        );
        assert_eq!(registrar.refused.len(), refused_before_valid);
        assert!(
            (&mut registrar as &mut dyn Registrar).is_block_capability_registered(&ITEMS, &vault)
        );
        assert!(
            (&mut registrar as &mut dyn Registrar).is_block_capability_registered(&FLUIDS, &tank)
        );
        assert!(
            (&mut registrar as &mut dyn Registrar)
                .is_block_capability_registered(&ENERGY, &battery)
        );
    }
}
