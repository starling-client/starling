use super::super::native_structure_types::book_example;
use super::*;
use sunrise_core::random::LegacyRandom;
use sunrise_world::registry::BlockStateId;
use sunrise_world::worldgen::mod_features::ModFeatureRandom;

/// A world of air with dirt at y 63 (and nothing readable below 0), recording every write.
struct Ground {
    registry: Option<&'static BlockRegistry>,
    writes: Vec<([i32; 3], BlockStateId, i32)>,
    entities: Vec<([i32; 3], Compound)>,
}
impl ModFeatureTarget for Ground {
    fn registry(&self) -> Option<&'static BlockRegistry> {
        self.registry
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockStateId> {
        if pos[1] < 0 {
            return None;
        }
        let written = self.writes.iter().rev().find(|w| w.0 == pos).map(|w| w.1);
        Some(written.unwrap_or(BlockStateId(if pos[1] == 63 { 1 } else { 0 })))
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        self.writes.push((pos, state, flags));
        true
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        Some(tag == "minecraft:dirt" && self.block(pos) == Some(BlockStateId(1)))
    }
    fn min_y(&mut self) -> Option<i32> {
        Some(-64)
    }
    fn max_y(&mut self) -> Option<i32> {
        Some(319)
    }
    fn height(
        &mut self,
        _: sunrise_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> Option<i32> {
        Some(64)
    }
    fn set_block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> bool {
        self.entities.push((pos, nbt));
        true
    }
}

struct Random(LegacyRandom);
impl ModFeatureRandom for Random {
    fn int(&mut self, bound: i32) -> i32 {
        use sunrise_core::random::RandomSource;
        self.0.next_int_bound(bound).unwrap_or(0)
    }
    fn int_unbounded(&mut self) -> i32 {
        use sunrise_core::random::RandomSource;
        self.0.next_int()
    }
    fn long(&mut self) -> i64 {
        use sunrise_core::random::RandomSource;
        self.0.next_long()
    }
    fn float(&mut self) -> f32 {
        use sunrise_core::random::RandomSource;
        self.0.next_float()
    }
    fn double(&mut self) -> f64 {
        use sunrise_core::random::RandomSource;
        self.0.next_double()
    }
    fn boolean(&mut self) -> bool {
        use sunrise_core::random::RandomSource;
        self.0.next_boolean()
    }
    fn gaussian(&mut self) -> f64 {
        use sunrise_core::random::RandomSource;
        self.0.next_gaussian()
    }
}

/// The book's pillar, registered by a native mod and run through the game's adapter on a worker
/// thread (no game or Host there): its config is decoded against its schema at load (a bad one is
/// refused then, naming the field), its states and flags reach the target in engine terms, and
/// its answers come back as given.
#[test]
fn the_books_pillar_places_through_the_adapter_on_a_worker() {
    let registry = registry();
    let types = installed_feature_types();
    assert_eq!(
        types.keys().collect::<Vec<_>>(),
        ["logmod:logged", "mymod:pillar"]
    );
    let adapter = types["mymod:pillar"].clone();
    std::thread::spawn(move || {
        let config = |json: serde_json::Value| adapter.prepare_config(&json);
        let place = |origin: [i32; 3], config: &ModFeatureConfig| {
            let mut ground = Ground {
                registry: Some(registry),
                writes: Vec::new(),
                entities: Vec::new(),
            };
            let result = adapter.place(
                &mut ground,
                &mut Random(LegacyRandom::new(0)),
                origin,
                &**config,
            );
            (result, ground.writes)
        };
        let three = config(serde_json::json!({"type": "mymod:pillar", "height": 3})).unwrap();
        // On dirt: three logs up, axis y (state 2), with Feature.setBlock's flags (3).
        let (result, writes) = place([0, 64, 0], &three);
        assert_eq!(result, Ok(true));
        assert_eq!(
            writes,
            [64, 65, 66].map(|y| ([0, y, 0], BlockStateId(2), 3))
        );
        // Not on dirt: false, nothing written.
        assert_eq!(place([0, 70, 0], &three), (Ok(false), vec![]));
        // The schema's default.
        let default = config(serde_json::json!({"type": "mymod:pillar"})).unwrap();
        assert_eq!(place([0, 64, 0], &default).1.len(), 4);
        // Refused at load, naming the field: out of range, wrong type, undeclared.
        for (json, field) in [
            (
                serde_json::json!({"type": "mymod:pillar", "height": 99}),
                "height",
            ),
            (
                serde_json::json!({"type": "mymod:pillar", "height": "tall"}),
                "height",
            ),
            (
                serde_json::json!({"type": "mymod:pillar", "width": 2}),
                "width",
            ),
        ] {
            let error = config(json).err().unwrap();
            assert!(error.contains(field), "{error}");
        }
    })
    .join()
    .unwrap();
}

/// Block-entity data is taken only when its `id` is the placed block's own block entity, and a
/// world without the game's registry is a named failure, not a silent false.
#[test]
fn block_entity_data_must_fit_its_block_and_a_registry_is_required() {
    let registry = registry();
    let adapter = installed_feature_types()["logmod:logged"].clone();
    std::thread::spawn(move || {
        let config = adapter
            .prepare_config(&serde_json::json!({"type": "logmod:logged"}))
            .unwrap();
        let mut ground = Ground {
            registry: Some(registry),
            writes: Vec::new(),
            entities: Vec::new(),
        };
        // An oak log has no block entity: chest data for it is refused.
        let placed = adapter.place(
            &mut ground,
            &mut Random(LegacyRandom::new(0)),
            [0, 64, 0],
            &*config,
        );
        assert_eq!(placed, Ok(false));
        assert!(ground.entities.is_empty());
        let mut bare = Ground {
            registry: None,
            writes: Vec::new(),
            entities: Vec::new(),
        };
        let placed = adapter.place(
            &mut bare,
            &mut Random(LegacyRandom::new(0)),
            [0, 64, 0],
            &*config,
        );
        assert_eq!(
            placed,
            Err("this world has no block registry for mod features".into())
        );
    })
    .join()
    .unwrap();
}

/// A feature that places an oak log and gives it a chest's block-entity data.
struct Logged;
impl FeatureType for Logged {
    fn place(
        &self,
        context: &mut dyn FeatureContext,
        origin: [i32; 3],
    ) -> Result<bool, mod_api::FeatureError> {
        let log = context.state(BLOCK_LOG.get().copied().unwrap());
        context.set_block(origin, log, BlockUpdateFlags::CLIENTS);
        let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
        Ok(context.block_entity(origin, chest))
    }
}
static BLOCK_LOG: std::sync::OnceLock<BlockKey> = std::sync::OnceLock::new();
struct LoggedMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for LoggedMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let _ = BLOCK_LOG.set(r.block_key("minecraft:oak_log"));
        r.register_feature_type("logged", vec![], Logged);
    }
}

/// The book's mod and `logmod` installed for the calling test's thread (the parts table is the
/// thread's), with a small block registry resolved for the workers.
fn registry() -> &'static BlockRegistry {
    mod_platform::__private::part_tests::install_parts_fixture(&[
        mod_api::ModEntry::new("mymod", Box::new(book_example::features::FeatureMod)),
        mod_api::ModEntry::new("logmod", Box::new(LoggedMod)),
    ])
    .unwrap();
    let registry: &'static BlockRegistry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:air":0,"minecraft:dirt":1,"minecraft:oak_log":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:dirt"],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"y"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"z"}}]]}"#.as_slice())
            .unwrap(),
    ));
    crate::game::api_values::resolve_block_definitions(registry).unwrap();
    registry
}
