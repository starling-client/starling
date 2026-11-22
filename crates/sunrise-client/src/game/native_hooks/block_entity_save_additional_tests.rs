#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{BlockEntityPart, BlockEntitySaveAdditionalEvent, Hooks, LevelHost, ModEntry};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use sunrise_client::{
        chunk::{Chunk, LevelShape},
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, SHAPES_KEY},
        save::SaveDir,
    };
    use sunrise_core::{
        nbt::{Compound, JavaString, Limits, Tag, read_named},
        position::ChunkPos,
    };
    use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;
    use sunrise_world::region::RegionFile;

    thread_local! {
        static SAVED_AT: std::cell::RefCell<Vec<([i32; 3], mod_api::LevelRef)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    struct Counter(Arc<AtomicI32>);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Counter {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Counter {
        fn block_entity_save_additional(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockEntitySaveAdditionalEvent<'_>,
        ) {
            SAVED_AT.with_borrow_mut(|seen| seen.push((event.pos, event.level)));
            let serial = self.0.fetch_add(1, Ordering::Relaxed);
            match event.tag.get(&"count".into()) {
                Some(Tag::Int(-3)) => {
                    event.tag.insert(
                        "values".into(),
                        Tag::List(vec![Tag::String(JavaString(vec![0; 32_768]))]),
                    );
                }
                Some(Tag::Int(-2)) => {
                    event.tag.insert(JavaString(vec![0; 32_768]), Tag::Int(-99));
                }
                Some(Tag::Int(-1)) => {
                    event.tag.insert("invalid".into(), Tag::End);
                }
                Some(Tag::Int(0)) => {
                    event.tag.remove(&"count".into());
                }
                Some(Tag::Int(7)) => {
                    event.tag.insert("count".into(), Tag::Int(serial));
                }
                other => panic!("unexpected payload: {other:?}"),
            }
            assert!(!event.tag.contains_key(&ATTACHMENTS_KEY.into()));
            event
                .tag
                .insert("id".into(), Tag::String("test:wrong".into()));
            event.tag.insert(SHAPES_KEY.into(), Tag::Byte(1));
            event.tag.insert(ATTACHMENTS_KEY.into(), Tag::Byte(1));
        }
    }
    struct Mod(Arc<AtomicI32>);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let counter = registrar.block_entity_type("counter").build();
            registrar
                .block_entity_part(&counter, Counter(self.0.clone()))
                .unwrap();
            registrar.block_entity_type("silent").build();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn structure_capture_refuses_lossy_listener_keys_and_strings() {
        let calls = Arc::new(AtomicI32::new(0));
        let mods = vec![ModEntry::new("test", Box::new(Mod(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                mod_api::__private::resolve_block_entity_types(
                    ["test:counter", "test:silent"],
                    |_| Some(()),
                )
                .unwrap();
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                for (x, selector) in [(5, -2), (6, -3)] {
                    game.world.set(x, 64, 1, dirt).unwrap();
                    game.world.block_entities.insert(
                        (x, 64, 1),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "test:counter".into(),
                            ModBlockEntityFields {
                                nbt: Compound::from([
                                    ("".into(), Tag::Int(123)),
                                    ("count".into(), Tag::Int(selector)),
                                    ("values".into(), Tag::List(vec![Tag::String("keep".into())])),
                                ]),
                                ..Default::default()
                            },
                        ))),
                    );
                }
                assert!(game.save_structure_area(
                    "test:payload",
                    [5, 64, 1],
                    [2, 1, 1],
                    true,
                    "",
                    true,
                    false
                ));
                let bytes =
                    std::fs::read(scratch.path().join("generated/test/structures/payload.nbt"))
                        .unwrap();
                let limits = Limits::templates();
                let raw = sunrise_world::compression::Compression::Gzip
                    .decode(&bytes, limits.max_bytes)
                    .unwrap();
                let (saved, _) = read_named(&raw, limits).unwrap();
                let Some(Tag::List(blocks)) = saved.tag.get("blocks") else {
                    panic!("structure blocks missing")
                };
                assert_eq!(blocks.len(), 2);
                for block in blocks {
                    let nbt = block.get("nbt").unwrap();
                    assert_eq!(nbt.get(""), Some(&Tag::Int(123)));
                    assert_eq!(
                        nbt.get("values"),
                        Some(&Tag::List(vec![Tag::String("keep".into())]))
                    );
                }
                assert_eq!(calls.load(Ordering::Relaxed), 2);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn command_and_unload_capture_the_live_payload_without_mutating_storage() {
        let count = Arc::new(AtomicI32::new(11));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(Mod(count.clone())))].into(),
            || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let pos = (1, 64, 1);
                    let silent = (2, 64, 1);
                    let invalid = (3, 64, 1);
                    let erased = (4, 64, 1);
                    for (pos, kind, saved) in [
                        (pos, "test:counter", 7),
                        (silent, "test:silent", 7),
                        (invalid, "test:counter", -1),
                        (erased, "test:counter", 0),
                    ] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:dirt")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                kind.into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([
                                        ("count".into(), Tag::Int(saved)),
                                        (ATTACHMENTS_KEY.into(), Tag::Byte(7)),
                                    ]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    }
                    let tag = game.block_entity_nbt([1, 64, 1]).unwrap();
                    assert_eq!(tag.get("count"), Some(&Tag::Int(11)));
                    assert_eq!(tag.get("id"), Some(&Tag::String("test:counter".into())));
                    assert_eq!(tag.get(ATTACHMENTS_KEY), Some(&Tag::Byte(7)));
                    assert_eq!(tag.get(SHAPES_KEY), None);

                    // Ctrl-pick must not copy the removed field in the base clone item.
                    let picked = game.picked_stack([4, 64, 1], true).unwrap();
                    assert!(
                        picked
                            .data()
                            .and_then(|d| d.other("minecraft:block_entity_data"))
                            .is_none_or(|tag| tag.get("count").is_none())
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 13);

                    let detached = game.world.chunks.remove(&(0, 0)).unwrap();
                    let capture = game.with_entities([&detached]);
                    let fields = |entity: &BlockEntity| {
                        let BlockEntity::Mod(entity) = entity else {
                            panic!("expected a mod block entity")
                        };
                        entity.nbt.get(&"count".into()).cloned()
                    };
                    let snapshots = &capture[0].block_entities;
                    let stored = fields(&snapshots.iter().find(|(p, _)| *p == pos).unwrap().1);
                    assert_eq!(stored, Some(Tag::Int(7)));
                    assert_eq!(
                        fields(&snapshots.iter().find(|(p, _)| *p == silent).unwrap().1),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(
                        fields(game.world.block_entities.get(&pos).unwrap()),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 16);
                    // The unload capture names the entity's place after its chunk left the world.
                    let level = game.level("minecraft:overworld").unwrap();
                    SAVED_AT.with_borrow(|seen| {
                        assert!(seen.len() >= 3);
                        assert!(seen.contains(&([1, 64, 1], level)));
                        assert!(seen.iter().all(|(_, at)| *at == level));
                    });

                    // A malformed individual payload is refused at the callback seam.
                    // Other entities' valid edits still reach the actual region record.
                    let scratch = tempfile::tempdir().unwrap();
                    let save = SaveDir::new(
                        scratch.path().to_owned(),
                        LevelShape::OVERWORLD,
                        Some(Arc::new(Default::default())),
                    );
                    save.save_later(&capture, game.pack.assets.registry, game.res.biomes())
                        .unwrap();
                    save.flush().unwrap();
                    let saved = RegionFile::open(scratch.path().join("region/r.0.0.mca"))
                        .unwrap()
                        .read(ChunkPos::new(0, 0))
                        .unwrap()
                        .unwrap()
                        .nbt(Limits::saved())
                        .unwrap();
                    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
                        panic!("expected saved block entities");
                    };
                    let at = |x| {
                        entities
                            .iter()
                            .find(|tag| tag.get("x") == Some(&Tag::Int(x)))
                            .unwrap()
                    };
                    assert!(matches!(at(1).get("count"), Some(Tag::Int(13..=15))));
                    assert_eq!(at(3).get("count"), Some(&Tag::Int(-1)));
                    assert_eq!(at(3).get("invalid"), None);
                    assert_eq!(at(4).get("count"), None);
                })
            },
        );
    }

    struct SaveItemsPart;
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for SaveItemsPart {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for SaveItemsPart {
        fn block_entity_ticks(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::BlockEntityTicksEvent<'_>,
        ) {
            event.ticks = true;
        }

        fn block_entity_save_additional(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockEntitySaveAdditionalEvent<'_>,
        ) {
            let key = "Items".into();
            if event.pos[0] <= 2 {
                assert_eq!(event.tag.get(&key), Some(&saved_stack("minecraft:stone")));
            } else {
                assert!(
                    !event.tag.contains_key(&key),
                    "modeled slots are not listener input"
                );
            }
            if event.pos[0] == 2 {
                event.tag.remove(&key);
            } else {
                event.tag.insert(key, saved_stack("minecraft:dirt"));
            }
        }
    }

    fn saved_stack(id: &str) -> Tag {
        Tag::List(vec![Tag::Compound(Compound::from([
            ("Slot".into(), Tag::Byte(0)),
            ("id".into(), Tag::String(id.into())),
            ("count".into(), Tag::Int(1)),
        ]))])
    }

    struct SaveItemsMod;
    #[mod_api::__subscriptions]
    impl Hooks for SaveItemsMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let holder = r
                .block_entity_type("holder")
                .blocks(&["save_items:slotless", "save_items:container"])
                .build();
            r.block_entity_part(&holder, SaveItemsPart).unwrap();
            let container = r.block_key("save_items:container");
            r.block_container(
                &container,
                mod_api::ModContainer {
                    slots: 1,
                    ..Default::default()
                },
            );
        }

        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            for id in ["save_items:slotless", "save_items:container"] {
                event
                    .register(mod_api::Block::new(id, "minecraft:stone").with_item(false))
                    .unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn save_edits_slotless_items_but_keeps_declared_slots_after_a_skipped_tick() {
        use crate::game::native_entity_refs::tests::{fixture_root, run_fresh_cases, with_game_at};
        const NAME: &str = "game::native_hooks::block_entity_save_additional_tests::tests::save_edits_slotless_items_but_keeps_declared_slots_after_a_skipped_tick";
        if run_fresh_cases(NAME, "SUNRISE_SAVE_ITEMS_CHILD", &["save"]) {
            return;
        }
        let mods: mod_api::Mods = vec![ModEntry::new("save_items", Box::new(SaveItemsMod))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            let root =
                crate::game::mods::content_root(&fixture_root(), &Default::default()).unwrap();
            with_game_at(root.prepared().to_path_buf(), |game| {
                game.options.dedicated = Some(Vec::new());
                let registry = game.pack.assets.registry;
                let mut chunk = Chunk::empty(0, 0);
                for (x, block) in [(1, "slotless"), (2, "slotless"), (3, "container")] {
                    let state = registry
                        .default_state(&format!("save_items:{block}"))
                        .unwrap();
                    chunk.set(x, 64, 1, state.0);
                    chunk.block_entities.push((
                        (x, 64, 1),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "save_items:holder".into(),
                            ModBlockEntityFields {
                                nbt: Compound::from([(
                                    "Items".into(),
                                    saved_stack("minecraft:stone"),
                                )]),
                                ..Default::default()
                            },
                        ))),
                    ));
                }
                game.save
                    .save(&[Arc::new(chunk)], registry, game.res.biomes())
                    .unwrap();
                let mut chunk = game.save.load(0, 0, registry, game.res.biomes()).unwrap();
                game.world.install_chunk_block_entities(&mut chunk);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                assert!(game.streaming.set_forced(0, 0, true));
                game.update_chunk_tickets();
                game.tick_block_entities().unwrap();
                let stone = saved_stack("minecraft:stone");
                let expected = [
                    Some(saved_stack("minecraft:dirt")),
                    None,
                    Some(stone.clone()),
                ];
                let items_key = "Items".into();
                for (x, items) in (1..=3).zip(&expected) {
                    assert_eq!(
                        game.world
                            .block_entities
                            .ticking(game.world.block(x, 64, 1).0),
                        sunrise_client::block_entities::Ticking::Yes,
                    );
                    let Some(BlockEntity::Mod(row)) = game.world.block_entities.get(&(x, 64, 1))
                    else {
                        panic!("missing resident holder at {x}");
                    };
                    assert!(row.numeric_id().is_some());
                    assert!(row.tick_kind().is_none());
                    if x == 3 {
                        assert_eq!(row.nbt.get(&items_key), None);
                        let [Some(stack)] = row.items.as_slice() else {
                            panic!("expected one modeled stack in slot 0 at {x}");
                        };
                        assert_eq!((stack.item.as_ref(), stack.count), ("minecraft:stone", 1));
                    } else {
                        assert_eq!(row.nbt.get(&items_key), Some(&stone));
                        assert!(row.items.is_empty());
                    }
                    let captured = game.block_entity_nbt([x, 64, 1]).unwrap();
                    assert_eq!(captured.get("Items"), items.as_ref());
                }

                let capture = game.with_entities([&game.world.chunks[&(0, 0)]]);
                game.save
                    .save_later(&capture, registry, game.res.biomes())
                    .unwrap();
                game.save.flush().unwrap();
                let reloaded = game.save.load(0, 0, registry, game.res.biomes()).unwrap();
                for (x, items) in (1..=3).zip(&expected) {
                    let (_, BlockEntity::Mod(row)) = reloaded
                        .block_entities
                        .iter()
                        .find(|(pos, _)| *pos == (x, 64, 1))
                        .unwrap()
                    else {
                        panic!("missing saved holder at {x}");
                    };
                    if x == 3 {
                        assert_eq!(row.nbt.get(&items_key), None);
                        let [Some(stack)] = row.items.as_slice() else {
                            panic!("expected one saved stack in slot 0 at {x}");
                        };
                        assert_eq!((stack.item.as_ref(), stack.count), ("minecraft:stone", 1));
                    } else {
                        assert_eq!(row.nbt.get(&items_key), items.as_ref());
                        assert!(row.items.is_empty());
                    }
                }
            });
        });
    }

    struct BreakObserver(Arc<Mutex<Vec<(&'static str, Option<Tag>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for BreakObserver {
        fn player_destroyed_block(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerDestroyedBlockEvent,
        ) {
            let stored = event
                .block_entity
                .as_ref()
                .and_then(|tag| tag.get(&"count".into()))
                .cloned();
            self.0.lock().unwrap().push(("removed", stored));
        }
        fn block_drops(
            &self,
            _: &dyn LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            assert!(
                event.block_entity.is_none(),
                "a player break has already removed the block entity from the world"
            );
            let stored = event
                .block_entity_data
                .and_then(|tag| tag.get(&"count".into()).cloned());
            self.0.lock().unwrap().push(("drops", stored));
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn break_only_serializes_admitted_loot_not_removal_notices() {
        use sunrise_client::{command::args::GameMode, items::ItemStack, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;

        for with_observers in [false, true] {
            let count = Arc::new(AtomicI32::new(11));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mut mods = vec![ModEntry::new("test", Box::new(Mod(count.clone())))];
            if with_observers {
                mods.push(ModEntry::new(
                    "test_observer",
                    Box::new(BreakObserver(seen.clone())),
                ));
            }
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
                    game.lan_acting_uuid = Some(123);
                    game.lan_acting_entity = Some(20);
                    game.lan_acting_permission = Some(0);
                    let place = |game: &mut crate::game::Game<'_>, name| {
                        let pos = (7, 64, 4);
                        let state = game.pack.assets.registry.default_state(name).unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:counter".into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([("count".into(), Tag::Int(7))]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    };
                    for (mode, name, tool, serials) in [
                        (GameMode::Survival, "minecraft:dirt", None, 12),
                        (GameMode::Survival, "minecraft:diamond_ore", None, 12),
                        (
                            GameMode::Creative,
                            "minecraft:dirt",
                            Some(ItemStack::new("minecraft:stick", 1)),
                            12,
                        ),
                    ] {
                        game.set_mode(mode);
                        game.inventory[game.selected] = tool;
                        place(game, name);
                        assert!(game.break_block_as_player([7, 64, 4]).unwrap());
                        assert_eq!(count.load(Ordering::Relaxed), serials, "{mode:?}, {name}");
                    }
                    let seen = seen.lock().unwrap();
                    if with_observers {
                        assert_eq!(
                            *seen,
                            [
                                ("removed", Some(Tag::Int(7))),
                                ("drops", Some(Tag::Int(7))),
                                ("removed", Some(Tag::Int(7))),
                                ("removed", Some(Tag::Int(7))),
                            ]
                        );
                    } else {
                        assert!(seen.is_empty());
                    }
                })
            });
        }
    }
}
