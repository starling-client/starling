//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    /// Attaches each declared type's parts to its blocks. A type is found by id whichever mod
    /// declared it, but another mod's type requires a surviving dependency on its owner.
    pub(crate) fn expand_block_entity_parts(&mut self) {
        let packages = crate::load_state::current_packages();
        let dependencies = crate::load_state::effective_dependencies();
        let mut ids: Vec<_> = self.parts.block_entity_kinds.keys().collect();
        ids.sort_unstable();
        for id in ids {
            let parts = &self.parts.block_entity_kinds[id];
            let Some(&(owner, ref kind)) = self
                .block_entity_types
                .iter()
                .find(|(_, kind)| kind.id == **id)
            else {
                let namespace = id.split_once(':').map_or("", |(namespace, _)| namespace);
                // The installed package owning the namespace; an absent one is known only by
                // its id, which an optional dependency is matched against.
                let package = packages
                    .iter()
                    .find(|package| package.namespaces().any(|owned| owned == namespace));
                let dependency_id = package.map_or(namespace, |package| package.id());
                let absent = package.is_none();
                for part in parts {
                    let dependency = dependencies.kind(part.mod_id, dependency_id);
                    match dependency {
                        Some(crate::DependencyKind::Optional) if absent => {
                            log::debug!(
                                "mod {} skipped {} on block entity type {id}: optional dependency {dependency_id} is absent",
                                part.mod_id,
                                part.type_name
                            );
                            // Its type key asked for a type that, with the dependency absent,
                            // nobody declares: the request is not a load error, unless this mod
                            // also keyed a capability provider on the type, which still needs it.
                            let answers_capability = self.parts.capabilities.values().any(|by_type| {
                                by_type.get(&**id).is_some_and(|providers| {
                                    providers.iter().any(|provider| provider.mod_id == part.mod_id)
                                })
                            });
                            if !answers_capability {
                                self.registry_keys
                                    .borrow_mut()
                                    .block_entity_types
                                    .withdraw(part.mod_id, id);
                            }
                        }
                        Some(crate::DependencyKind::Required) => self.refused.push(format!(
                            "mod {} attached {} to block entity type {id}, which its required dependency {dependency_id} does not declare",
                            part.mod_id, part.type_name
                        )),
                        _ => self.refused.push(format!(
                            "mod {} attached {} to undeclared native block entity type {id}",
                            part.mod_id, part.type_name
                        )),
                    }
                }
                continue;
            };
            let allowed: Vec<_> = parts
                .iter()
                .filter(|part| {
                    let depends = dependencies.permits(part.mod_id, owner);
                    if !depends {
                        self.refused.push(format!(
                            "mod {} attached {} to block entity type {id}, declared by package {owner}; declare {owner} as a dependency",
                            part.mod_id, part.type_name
                        ));
                    }
                    depends
                })
                .collect();
            for block in &kind.blocks {
                for part in allowed.iter().copied() {
                    let block_part: Arc<dyn BlockPart> = part.part.clone();
                    if insert_part(
                        &mut self.parts.blocks,
                        block,
                        part.with_part(block_part),
                        &mut self.refused,
                    ) {
                        insert_part(
                            &mut self.parts.block_entities,
                            block,
                            part.clone(),
                            &mut self.refused,
                        );
                    }
                }
            }
        }
        for parts in self.parts.blocks.values_mut() {
            parts.sort_by_key(|part| (part.mod_index, part.order));
        }
        for parts in self.parts.block_entities.values_mut() {
            parts.sort_by_key(|part| (part.mod_index, part.order));
        }
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    #[test]
    pub(crate) fn block_entity_slots_resolve_forward_declarations_and_refuse_unknown() {
        struct Request(Arc<std::sync::Mutex<Vec<BlockEntityTypeKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Request {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let later = r.block_entity_type_key("owner:press");
                assert_eq!(later, r.block_entity_type_key("owner:press"));
                assert_eq!(
                    mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                        mod_vocab::__private::parts_installation::block_entity_type_key_slot(later)
                    ),
                    Ok(later)
                );
                assert!(
                    mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                        mod_vocab::__private::parts_installation::block_entity_type_key_slot(later)
                            + 1
                    )
                    .is_err()
                );
                self.0.lock().unwrap().push(later);
            }
        }
        struct Declare(Arc<std::sync::Mutex<Vec<BlockEntityTypeKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Declare {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let declared = r.block_entity_type("press").build();
                let chest = r.block_entity_type_key("chest");
                self.0.lock().unwrap().extend([declared, chest]);
            }
        }
        let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
        install_parts(
            &[
                ModEntry::new("first", Box::new(Request(keys.clone()))),
                ModEntry::new("owner", Box::new(Declare(keys.clone()))),
            ],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let types = |names: &[&str]| names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            {
                let types = &types(&["minecraft:chest"]);
                mod_vocab::__private::resolve_block_entity_types(
                    types.iter().map(String::as_str),
                    |id| types.iter().position(|name| name == id),
                )
            }
            .unwrap_err(),
            [
                "mod first requested unknown block entity type owner:press",
                "mod owner requested unknown block entity type owner:press"
            ]
        );
        {
            let types = &types(&["minecraft:chest", "owner:press", "minecraft:furnace"]);
            mod_vocab::__private::resolve_block_entity_types(
                types.iter().map(String::as_str),
                |id| types.iter().position(|name| name == id),
            )
        }
        .unwrap();
        let keys = keys.lock().unwrap();
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[0], keys[2]);
        assert_eq!(
            mod_vocab::__private::block_entity_type_key("owner:press"),
            Some(keys[0])
        );
        assert_eq!(
            mod_vocab::__private::block_entity_type_name(keys[2]),
            Some("minecraft:chest")
        );
        let furnace = mod_vocab::__private::block_entity_type_key("minecraft:furnace").unwrap();
        assert_ne!(furnace, keys[2]);
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                mod_vocab::__private::parts_installation::block_entity_type_key_slot(furnace)
            ),
            Ok(furnace)
        );
        assert!(
            mod_vocab::__private::parts_installation::validate_block_entity_type_key(u32::MAX)
                .is_err()
        );
        struct Table;
        impl mod_api::prelude::KeysBackend for Table {
            fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
                mod_vocab::__private::block_entity_type_name(key)
            }
        }
        impl Backend for Table {}
        mod_api::impl_backend_areas!(Table, except(keys));
        impl Host for Table {
            fn backend(&self) -> Option<BackendRef<'_>> {
                Some(mod_vocab::__private::parts_installation::backend_ref(self))
            }
        }
        mod_api::impl_host_areas!(Table);
        assert_eq!(keys[0].id(&Table), Some("owner:press"));
    }

    #[test]
    pub(crate) fn invalid_block_entity_type_paths_refuse_registration() {
        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                for path in ["", "other:press", "Uppercase", "has space"] {
                    r.block_entity_type(path).build();
                }
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().all(|error| error.contains("mod test")
            && (error.contains("invalid identifier") || error.contains("unowned namespace"))));
    }

    /// The block entity of type `doubler:press_entity`, held by block `test:press`.
    static PRESS_TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    pub(crate) struct Press;
    #[mod_api::sunrise_part]
    impl BlockPart for Press {
        fn analog_output(&self, _: &mut dyn LevelRandomHost, event: &mut AnalogOutputEvent) {
            event.output += 40;
        }
    }
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Press {
        fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
            event.ticks = true;
        }
        fn block_entity_tick(&self, _: &mut BlockEntityTickEvent<'_>) {
            PRESS_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        fn block_entity_view_distance(
            &self,
            _: &dyn ClientHost,
            event: &mut BlockEntityViewDistanceEvent<'_>,
        ) {
            event.distance = 128.0;
        }
    }
    /// The gold a `doubler:hoard` block entity holds, kept beside it.
    pub const HOARD: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "hoard",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    /// How many times a `doubler:hoard` was asked about its removal.
    pub static HOARD_ASKED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    /// The block entity of type `doubler:hoard`, held by block `test:hoard`: when it is
    /// removed it gives back the gold its `HOARD` counts.
    pub(crate) struct Hoard(pub(crate) BlockKey);
    #[mod_api::sunrise_part]
    impl BlockPart for Hoard {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Hoard {
        /// With data `[1]`, it replaces its own block with granite.
        fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
            if let Some((level, pos)) = event
                .subject
                .with_view(|entity| {
                    (entity.data_slots() == [1]).then(|| (entity.level(), entity.position()))
                })
                .flatten()
            {
                let host = event.subject.host();
                let state = BlockState::of(&host.block_states(), self.0, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, pos, state, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
            }
        }
        fn pre_remove_side_effects(
            &self,
            level: &mut dyn LevelRandomHost,
            event: &mut PreRemoveSideEffectsEvent<'_>,
        ) {
            HOARD_ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let BlockPos { x, y, z } = event.pos;
            if let Ok(Some(count @ 1..)) = HOARD.get_existing(level, Holder::BlockEntity([x, y, z]))
            {
                event.drops.push(Some(crate::parts::items::tests::stack(
                    "minecraft:gold_ingot",
                    count,
                    0,
                )));
            }
        }
    }

    #[test]
    pub(crate) fn a_vanilla_type_refuses_a_native_payload_part_at_registration() {
        let mut r = super::PartRegistrar::new("test");
        let kind = r.block_entity_type_key("minecraft:chest");
        assert_eq!(
            (&mut r as &mut dyn Registrar).block_entity_part(&kind, Press),
            Err(BlockEntityPartError::VanillaType { kind }),
        );
        assert!(r.parts.block_entity_kinds.is_empty());
        assert!(r.parts.blocks.is_empty());
        assert!(r.parts.block_entities.is_empty());
    }

    #[test]
    pub(crate) fn a_duplicate_native_part_returns_its_typed_error_and_keeps_the_first() {
        let mut r = super::PartRegistrar::new("test");
        let kind = (&mut r as &mut dyn Registrar)
            .block_entity_type("press")
            .blocks(&["test:press"])
            .build();
        (&mut r as &mut dyn Registrar)
            .block_entity_part(&kind, Press)
            .unwrap();
        assert_eq!(
            (&mut r as &mut dyn Registrar).block_entity_part(&kind, Press),
            Err(BlockEntityPartError::Duplicate {
                kind,
                part: type_name::<Press>()
            }),
        );
        assert_eq!(r.parts.block_entity_kinds["test:press"].len(), 1);
    }

    /// A part with no hooks: it only shows who attached it, and in what order.
    #[cfg(test)]
    macro_rules! quiet_part {
        ($name:ident) => {
            struct $name;
            #[mod_api::sunrise_part]
            impl BlockPart for $name {}
            #[mod_api::sunrise_part]
            impl BlockEntityPart for $name {}
        };
    }
    #[cfg(test)]
    quiet_part!(FromB);
    #[cfg(test)]
    quiet_part!(FromC);

    #[cfg(test)]
    struct Owner;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl Hooks for Owner {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.block_entity_type("press")
                .blocks(&["owner:block"])
                .build();
        }
    }
    #[cfg(test)]
    struct Attaches<P: BlockEntityPart + Clone>(P);
    #[cfg(test)]
    impl<P: BlockEntityPart + Clone> Hooks for Attaches<P> {
        fn subscribed(&self) -> &'static [Option<HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let press = r.block_entity_type_key("owner:press");
            r.block_entity_part(&press, self.0.clone()).unwrap();
        }
    }
    #[cfg(test)]
    macro_rules! attach_part {
        ($name:ident) => {
            impl Clone for $name {
                fn clone(&self) -> Self {
                    Self
                }
            }
        };
    }
    #[cfg(test)]
    attach_part!(FromB);
    #[cfg(test)]
    attach_part!(FromC);

    #[cfg(test)]
    fn package(id: &str, side: Side, depends_on: &[&str]) -> PackageInfo {
        package_of(id, side, depends_on, DependencyKind::Required)
    }

    #[cfg(test)]
    fn package_of(id: &str, side: Side, depends_on: &[&str], kind: DependencyKind) -> PackageInfo {
        PackageInfo::new(
            id.into(),
            "1".into(),
            PackageKind::Code,
            side,
            vec![id.into()],
            depends_on
                .iter()
                .map(|owner| {
                    PackageDependency::new(
                        (*owner).into(),
                        "*".into(),
                        kind,
                        DependencyOrder::After,
                        Side::Both,
                        None,
                        None,
                    )
                })
                .collect(),
        )
    }

    #[cfg(test)]
    fn attached_to_owner_block(mods: &[ModEntry]) -> (Vec<&'static str>, Vec<String>) {
        let registrar = crate::parts::shared::register_all(mods, |_| false);
        let attached = registrar
            .parts
            .block_entities
            .get("owner:block")
            .map(|parts| parts.iter().map(|part| part.mod_id).collect())
            .unwrap_or_default();
        (attached, registrar.refused)
    }

    #[test]
    pub(crate) fn a_part_attaches_to_a_type_its_dependency_declared() {
        let mods = [
            ModEntry::new("owner", Box::new(Owner)),
            ModEntry::new("b", Box::new(Attaches(FromB))),
        ];
        crate::load_state::with_test_package_inventory(
            vec![
                package("owner", Side::Both, &[]),
                package("b", Side::Both, &["owner"]),
            ],
            || {
                let (attached, refused) = attached_to_owner_block(&mods);
                assert_eq!(refused, Vec::<String>::new());
                assert_eq!(attached, ["b"]);
                crate::load_state::with_test_dependencies(Default::default(), || {
                    let (attached, refused) = attached_to_owner_block(&mods);
                    assert!(attached.is_empty());
                    assert_eq!(
                        refused,
                        [format!(
                            "mod b attached {} to block entity type owner:press, declared by package owner; declare owner as a dependency",
                            type_name::<FromB>()
                        )]
                    );
                });
            },
        );
    }

    #[test]
    pub(crate) fn a_type_of_an_undeclared_package_is_refused_with_the_dependency_to_add() {
        let mods = [
            ModEntry::new("owner", Box::new(Owner)),
            ModEntry::new("b", Box::new(Attaches(FromB))),
        ];
        // `owner` is installed, but `b` does not list it.
        crate::load_state::with_test_package_inventory(
            vec![
                package("owner", Side::Both, &[]),
                package("b", Side::Both, &[]),
            ],
            || {
                let (attached, refused) = attached_to_owner_block(&mods);
                assert!(attached.is_empty());
                assert_eq!(
                    refused,
                    [format!(
                        "mod b attached {} to block entity type owner:press, declared by package owner; declare owner as a dependency",
                        type_name::<FromB>()
                    )]
                );
            },
        );
    }

    #[test]
    pub(crate) fn a_type_declared_nowhere_is_refused_by_id_and_mod() {
        let mods = [ModEntry::new("b", Box::new(Attaches(FromB)))];
        crate::load_state::with_test_package_inventory(
            vec![package("b", Side::Both, &["owner"])],
            || {
                let (attached, refused) = attached_to_owner_block(&mods);
                assert!(attached.is_empty());
                assert_eq!(
                    refused,
                    [format!(
                        "mod b attached {} to block entity type owner:press, which its required dependency owner does not declare",
                        type_name::<FromB>()
                    )]
                );
            },
        );
        // No dependency on it at all: today's refusal.
        let mods = [ModEntry::new("b", Box::new(Attaches(FromB)))];
        crate::load_state::with_test_package_inventory(vec![package("b", Side::Both, &[])], || {
            let (_, refused) = attached_to_owner_block(&mods);
            assert_eq!(
                refused,
                [format!(
                    "mod b attached {} to undeclared native block entity type owner:press",
                    type_name::<FromB>()
                )]
            );
        });
    }

    #[test]
    pub(crate) fn an_absent_optional_dependency_loads_and_an_unknown_required_key_is_still_refused()
    {
        struct RequestsOnly;
        impl Hooks for RequestsOnly {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.block_entity_type_key("owner:press");
            }
        }
        let resolve = || {
            let known = ["minecraft:chest"];
            mod_vocab::__private::resolve_block_entity_types(known, |id| {
                known.iter().position(|name| *name == id)
            })
        };
        let optional = || {
            vec![package_of(
                "b",
                Side::Both,
                &["owner"],
                DependencyKind::Optional,
            )]
        };
        crate::load_state::with_test_package_inventory(optional(), || {
            let mods = [ModEntry::new("b", Box::new(Attaches(FromB)))];
            crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
                .unwrap();
            assert_eq!(resolve(), Ok(()));
        });
        // A capability provider keyed on the same absent type still needs it.
        struct AnswersToo;
        #[mod_api::__subscriptions]
        impl Hooks for AnswersToo {
            fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
                Attaches(FromB).register(r, cx);
                let press = r.block_entity_type_key("owner:press");
                r.block_entity_capability(
                    &crate::parts::capabilities::tests::CHEMICALS,
                    &press,
                    |_, at, _, _, offer| {
                        offer.offer(|| Box::new(crate::parts::capabilities::tests::ChemTank(at)))
                    },
                );
            }
        }
        crate::load_state::with_test_package_inventory(optional(), || {
            let mods = [ModEntry::new("b", Box::new(AnswersToo))];
            crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
                .unwrap();
            assert_eq!(
                resolve(),
                Err(vec![
                    "mod b requested unknown block entity type owner:press".to_owned()
                ])
            );
        });
        let mut packages = optional();
        packages.push(package("c", Side::Both, &[]));
        crate::load_state::with_test_package_inventory(packages, || {
            let mods = [
                ModEntry::new("b", Box::new(Attaches(FromB))),
                ModEntry::new("c", Box::new(RequestsOnly)),
            ];
            crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
                .unwrap();
            assert_eq!(
                resolve(),
                Err(vec![
                    "mod c requested unknown block entity type owner:press".to_owned()
                ])
            );
        });
    }

    #[test]
    pub(crate) fn a_required_dependency_owning_the_namespace_under_another_id_is_named_by_its_id() {
        let mods = [ModEntry::new("b", Box::new(Attaches(FromB)))];
        let lib = PackageInfo::new(
            "owner-lib".into(),
            "1".into(),
            PackageKind::Code,
            Side::Both,
            vec!["owner-lib".into(), "owner".into()],
            vec![],
        );
        crate::load_state::with_test_package_inventory(
            vec![lib, package("b", Side::Both, &["owner-lib"])],
            || {
                let (attached, refused) = attached_to_owner_block(&mods);
                assert!(attached.is_empty());
                assert_eq!(
                    refused,
                    [format!(
                        "mod b attached {} to block entity type owner:press, which its required dependency owner-lib does not declare",
                        type_name::<FromB>()
                    )]
                );
            },
        );
    }

    #[test]
    pub(crate) fn an_optional_dependencys_type_attaches_when_present_and_is_skipped_when_absent() {
        let optional = |installed: &[&str]| {
            let mut packages = vec![package_of(
                "b",
                Side::Both,
                &["owner"],
                DependencyKind::Optional,
            )];
            packages.extend(installed.iter().map(|id| package(id, Side::Both, &[])));
            packages
        };
        crate::load_state::with_test_package_inventory(optional(&["owner"]), || {
            let mods = [
                ModEntry::new("owner", Box::new(Owner)),
                ModEntry::new("b", Box::new(Attaches(FromB))),
            ];
            let (attached, refused) = attached_to_owner_block(&mods);
            assert_eq!(refused, Vec::<String>::new());
            assert_eq!(attached, ["b"]);
        });
        crate::load_state::with_test_package_inventory(optional(&[]), || {
            let mods = [ModEntry::new("b", Box::new(Attaches(FromB)))];
            let (attached, refused) = attached_to_owner_block(&mods);
            assert_eq!(refused, Vec::<String>::new());
            assert!(attached.is_empty());
        });
    }

    #[test]
    pub(crate) fn parts_of_two_packages_attach_in_dispatch_order_on_every_load() {
        let inventory = || {
            vec![
                package("owner", Side::Both, &[]),
                package("c", Side::Both, &["owner"]),
                package("b", Side::Both, &["owner"]),
            ]
        };
        crate::load_state::with_test_package_inventory(inventory(), || {
            for _load in 0..16 {
                let mods = [
                    ModEntry::new("owner", Box::new(Owner)),
                    ModEntry::new("c", Box::new(Attaches(FromC))),
                    ModEntry::new("b", Box::new(Attaches(FromB))),
                ];
                let (attached, refused) = attached_to_owner_block(&mods);
                assert_eq!(refused, Vec::<String>::new());
                assert_eq!(attached, ["c", "b"]);
            }
        });
    }

    #[cfg(test)]
    static OWNERS_PROGRESS: AttachmentType<u32> = AttachmentType {
        namespace: "owner",
        name: "progress",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    #[cfg(test)]
    static B_PROGRESS: AttachmentType<u32> = AttachmentType {
        namespace: "b",
        name: "progress",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    #[cfg(test)]
    struct Keeps<const OWNERS: bool>;
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockPart for Keeps<true> {}
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Keeps<true> {}
    #[cfg(test)]
    impl BlockEntityPartState for Keeps<true> {
        type Data = u32;
        const DATA: &'static AttachmentType<u32> = &OWNERS_PROGRESS;
    }
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockPart for Keeps<false> {}
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Keeps<false> {}
    #[cfg(test)]
    impl BlockEntityPartState for Keeps<false> {
        type Data = u32;
        const DATA: &'static AttachmentType<u32> = &B_PROGRESS;
    }
    #[cfg(test)]
    struct KeepsState<const OWNERS: bool>;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl<const OWNERS: bool> Hooks for KeepsState<OWNERS>
    where
        Keeps<OWNERS>: BlockEntityPartState + 'static,
    {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_attachment_type("b:progress", false).unwrap();
            let press = r.block_entity_type_key("owner:press");
            let _ = r.block_entity_part_with_state(&press, Keeps::<OWNERS>);
        }
    }

    #[test]
    pub(crate) fn a_foreign_parts_state_is_declared_and_keyed_by_its_own_package() {
        struct OwnerWithState;
        #[mod_api::__subscriptions]
        impl Hooks for OwnerWithState {
            fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
                Owner.register(r, cx);
            }
        }
        struct OwnerDeclaresKey;
        #[mod_api::__subscriptions]
        impl Hooks for OwnerDeclaresKey {
            fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
                r.register_attachment_type("owner:progress", false).unwrap();
                Owner.register(r, cx);
            }
        }
        let packages = || {
            vec![
                package("owner", Side::Both, &[]),
                package("b", Side::Both, &["owner"]),
            ]
        };
        // The part's own key: declared by `b` in `b`'s namespace, saved under `b`.
        crate::load_state::with_test_package_inventory(packages(), || {
            let mods = [
                ModEntry::new("owner", Box::new(OwnerWithState)),
                ModEntry::new("b", Box::new(KeepsState::<false>)),
            ];
            let registrar = crate::parts::shared::register_all(&mods, |_| false);
            assert_eq!(registrar.refused, Vec::<String>::new());
            assert_eq!(
                registrar.attachment_uses,
                [("b".to_owned(), "b".to_owned(), "progress".to_owned())]
            );
        });
        // The owner's key is only usable when the owner declared it (its public attachment);
        // a part cannot invent a key under the owner's namespace.
        crate::load_state::with_test_package_inventory(packages(), || {
            let mods = [
                ModEntry::new("owner", Box::new(OwnerWithState)),
                ModEntry::new("b", Box::new(KeepsState::<true>)),
            ];
            let errors =
                crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| {
                    false
                })
                .unwrap_err();
            assert_eq!(
                errors,
                ["package b attachment owner:progress: undeclared attachment use"]
            );
        });
        crate::load_state::with_test_package_inventory(packages(), || {
            let mods = [
                ModEntry::new("owner", Box::new(OwnerDeclaresKey)),
                ModEntry::new("b", Box::new(KeepsState::<true>)),
            ];
            let registrar = crate::parts::shared::register_all(&mods, |_| false);
            assert_eq!(
                registrar.attachment_uses,
                [("b".to_owned(), "owner".to_owned(), "progress".to_owned())],
                "a declared key is the owner's public attachment"
            );
        });
    }

    #[test]
    pub(crate) fn a_server_package_with_a_client_part_on_a_dependencys_type_is_refused() {
        struct ClientHooked;
        #[mod_api::sunrise_part]
        impl BlockPart for ClientHooked {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for ClientHooked {
            fn block_entity_view_distance(
                &self,
                _: &dyn ClientHost,
                event: &mut BlockEntityViewDistanceEvent<'_>,
            ) {
                event.distance = 128.0;
            }
        }
        impl Clone for ClientHooked {
            fn clone(&self) -> Self {
                Self
            }
        }
        crate::load_state::with_test_package_inventory(
            vec![
                package("owner", Side::Both, &[]),
                package("b", Side::Server, &["owner"]),
            ],
            || {
                let mods = [
                    ModEntry::new("owner", Box::new(Owner)),
                    ModEntry::new("b", Box::new(Attaches(ClientHooked))),
                ];
                crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| {
                    false
                })
                .unwrap();
                assert_eq!(
                    crate::__private::client_requirement("b"),
                    Some(format!(
                        "attaches the part {}, which listens to the client hook block_entity_view_distance",
                        type_name::<ClientHooked>()
                    ))
                );
                assert_eq!(crate::__private::client_requirement("owner"), None);
            },
        );
    }

    #[test]
    pub(crate) fn a_block_entity_part_answers_by_block_and_by_entity_type() {
        game_thread();
        let ticks = |block| {
            let mut ticks = false;
            dispatch_block_entity_ticks(&NoHost, block, &mut ticks);
            ticks
        };
        // The ticker is asked by block: the type id is not a block.
        assert!(ticks("test:press"));
        assert!(!ticks("doubler:press_entity"));
        mod_vocab::__private::resolve_block_entity_types(
            [
                "doubler:press_entity",
                "doubler:hoard",
                "test:pipe",
                "test:greedy",
                "test:item_overstates",
                "test:press",
            ],
            |_| Some(()),
        )
        .unwrap();
        struct Unused;
        impl BlockEntityAccess for Unused {
            fn with_view(&mut self, _: &mut dyn FnMut(BlockEntityView<'_>)) {
                unreachable!()
            }
            fn host(&mut self) -> &mut dyn ServerHost {
                unreachable!()
            }
            fn transaction(
                &mut self,
                _: &mut dyn FnMut(&mut dyn BlockEntityAccess, &Transaction) -> bool,
            ) {
                unreachable!()
            }
        }
        let tick = |kind: &str| {
            let kind = mod_vocab::__private::block_entity_type_key(kind).unwrap();
            dispatch_block_entity_tick(kind, BlockEntityScope::new(&mut Unused));
            PRESS_TICKS.swap(0, std::sync::atomic::Ordering::Relaxed)
        };
        // The tick is asked by the entity's type, which differs from its block's id.
        assert_eq!(tick("doubler:press_entity"), 1);
        assert_eq!(tick("test:press"), 0);
        // The view distance is asked by block, as the ticker is.
        let distance = |block| {
            let mut distance = 64.0;
            dispatch_block_entity_view_distance(&NoHost, block, &mut distance);
            distance
        };
        assert_eq!(distance("test:press"), 128.0);
        assert_eq!(distance("doubler:press_entity"), 64.0);
        assert_eq!(distance("test:other"), 64.0);
    }
}

impl BlockEntitiesRegistrar for PartRegistrar {
    fn set_block_entity_transform(
        &mut self,
        key: crate::BlockEntityTypeKey,
        step: Arc<dyn crate::BlockEntityTransform>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .block_entity_types
            .id(mod_vocab::__private::parts_installation::block_entity_type_key_slot(key))
            .map(str::to_owned);
        if let Some(id) = id.filter(|id| {
            self.block_entity_types
                .iter()
                .any(|(_, decl)| decl.id == *id)
                && self.owned_declaration_id(id).is_ok()
        }) {
            self.block_entity_transforms.insert(id, step);
        } else {
            self.refused
                .push("transform requires an owned block entity declaration".into());
        }
    }

    fn block_entity_type_key(&mut self, id: &str) -> crate::BlockEntityTypeKey {
        match self
            .registry_keys
            .borrow_mut()
            .block_entity_types
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::block_entity_type_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            }
        }
    }

    fn declare_block_entity_type(
        &mut self,
        path: &str,
        blocks: Vec<String>,
    ) -> crate::BlockEntityTypeKey {
        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX);
            }
        };
        let key = self.block_entity_type_key(&id);
        let blocks = blocks
            .iter()
            .map(|block| mod_vocab::__private::qualified(self.mod_id, block))
            .collect();
        self.block_entity_types.push((
            self.mod_id,
            mod_vocab::__private::parts_installation::block_entity_declaration(id, blocks),
        ));
        key
    }

    fn attach_block_entity_part(
        &mut self,
        key: &crate::BlockEntityTypeKey,
        kind: &'static str,
        part: Arc<dyn BlockEntityPart>,
    ) -> Result<(), crate::BlockEntityPartError> {
        let id = self
            .registry_keys
            .borrow()
            .block_entity_types
            .id(mod_vocab::__private::parts_installation::block_entity_type_key_slot(*key))
            .map(str::to_owned)
            .unwrap_or_default();
        if id.starts_with("minecraft:") {
            return Err(crate::BlockEntityPartError::VanillaType { kind: *key });
        }
        if self.attach_known(|p| &mut p.block_entity_kinds, kind, &id, part) {
            Ok(())
        } else {
            Err(crate::BlockEntityPartError::Duplicate {
                kind: *key,
                part: kind,
            })
        }
    }

    fn declare_block_entity_part_state(
        &mut self,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::BlockEntityStateError> {
        self.check_part_state(declaration).map_err(|reason| {
            let error = declaration.refused(part, reason);
            self.refused.push(format!("{}: {error}", self.mod_id));
            error
        })
    }

    fn provide_block_entity_capability(
        &mut self,
        name: &'static str,
        kind: &crate::BlockEntityTypeKey,
        before_game: bool,
        provider: BlockProviderOf,
    ) {
        self.capability(
            |p| &mut p.capabilities,
            Some(|p| &p.block_capabilities),
            name,
            ProviderTarget::BlockEntity(kind),
            before_game,
            {
                let handler =
                    mod_vocab::__private::parts_installation::block_provider_handler(&provider);
                (provider, handler)
            },
        );
    }
}

impl PartRegistrar {
    /// Checks a Part state declaration of either holder and records its key: see
    /// `PartStateDeclaration::check` or a key used with another Rust type name.
    /// Owner declarations are checked after all packages have registered.
    pub(crate) fn check_part_state(
        &mut self,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::PartStateRefusal> {
        let key = (declaration.namespace, declaration.name);
        declaration
            .check()
            .and_then(|()| match self.part_states.get(&key) {
                Some(&declared) if declared != declaration.data_name => {
                    Err(crate::PartStateRefusal::Incompatible { declared })
                }
                _ => Ok(()),
            })?;
        self.attachment_uses
            .push((self.mod_id.into(), key.0.into(), key.1.into()));
        self.part_states.insert(key, declaration.data_name);
        Ok(())
    }
}
