//! Mod feature types through the engine: decoding, the caller's random, the placement chain's
//! order, nesting, staging, faults, the custom budget and block entities.
use super::*;
use crate::worldgen::anchor::WorldGenerationContext;
use crate::worldgen::feature::{FeatureBlocks, FeatureWorld};
use crate::worldgen::feature_graph::{FeatureDefinition, PlacedFeatureDefinition};
use crate::worldgen::placement::{PlacementContext, PlacementModifier, PlacementQueryError};
use crate::worldgen::value_provider::IntProvider;
use std::sync::Mutex;
use sunrise_core::nbt::Tag;
use sunrise_core::random::{LegacyRandom, WorldgenRandom};

/// A world 64 blocks wide and 16 tall (y 0..16): reads inside it, writes inside its write zone
/// (x and z 0..48), every write and block entity kept in order. State 1 is in
/// `minecraft:dirt`; so is anything at y 0.
#[derive(Default)]
struct World {
    writes: Vec<([i32; 3], BlockStateId, i32)>,
    entities: Vec<([i32; 3], Compound)>,
}
impl FeatureWorld for World {
    type Error = PlacementQueryError;
    fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        if !(0..16).contains(&p[1]) || !(0..64).contains(&p[0]) || !(0..64).contains(&p[2]) {
            return Err(PlacementQueryError::Unavailable);
        }
        Ok(self
            .writes
            .iter()
            .rev()
            .find(|w| w.0 == p)
            .map_or(BlockStateId(0), |w| w.1))
    }
    fn set_block(&mut self, p: [i32; 3], s: BlockStateId, flags: i32) -> Result<bool, Self::Error> {
        self.block(p)?;
        self.writes.push((p, s, flags));
        Ok(true)
    }
}
impl PlacementContext for World {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 16, 8)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(15)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Ok(4)
    }
    fn ensure_can_write(&mut self, p: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok((0..48).contains(&p[0]) && (0..48).contains(&p[2]))
    }
    fn state_matches_tag(
        &mut self,
        state: BlockStateId,
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        Ok(tag == "minecraft:dirt" && state == BlockStateId(1))
    }
    fn block_matches_tag(&mut self, p: [i32; 3], tag: &str) -> Result<bool, PlacementQueryError> {
        let state = self.block(p)?;
        Ok(tag == "minecraft:dirt" && (state == BlockStateId(1) || p[1] == 0))
    }
    fn set_block_entity(&mut self, p: [i32; 3], nbt: Compound) -> Result<(), PlacementQueryError> {
        self.entities.push((p, nbt));
        Ok(())
    }
}
fn blocks() -> FeatureBlocks {
    FeatureBlocks {
        air: BlockStateId(0),
        stone: BlockStateId(1),
        cobblestone: BlockStateId(2),
        end_stone: BlockStateId(3),
        obsidian: BlockStateId(4),
    }
}

/// What a test type does when placed:
/// - `Marks` draws `nextInt(1)`, records the origin and the draw, and writes state 1 there;
/// - `Column` writes its config's height of state 1 upward, then answers false;
/// - `Fails` writes once, then fails; `Panics` writes once, then panics;
/// - `Flood` reads the origin 70,000 times;
/// - `Refuses` writes once, then writes outside the world's write zone;
/// - `Fill` writes 8,000 blocks with block entities, then reads 8,000 positions it never wrote;
/// - `SecondFails` and `SecondFloods` write once and answer `Ok`, as `Marks` does, but their
///   second call (of a run's log) fails, or floods;
/// - `Chest` writes state 5 at the origin with block-entity data there, and tries block-entity
///   data next to it (not placed by it) and far outside the world.
#[derive(Clone, Copy)]
enum Kind {
    Marks,
    Column,
    Fails,
    Panics,
    Flood,
    Chest,
    SecondFails,
    SecondFloods,
    Fill,
    Refuses,
}
type Log = Arc<Mutex<Vec<([i32; 3], i32)>>>;
struct Test(Kind, Log);
impl ModFeatureType for Test {
    fn prepare_config(&self, config: &serde_json::Value) -> Result<ModFeatureConfig, String> {
        match config.get("height") {
            None => Ok(Arc::new(1_i32)),
            Some(serde_json::Value::Number(n))
                if n.as_i64().is_some_and(|n| (1..=8).contains(&n)) =>
            {
                Ok(Arc::new(n.as_i64().unwrap_or(1) as i32))
            }
            Some(_) => Err("height must be 1 to 8".into()),
        }
    }
    fn place(
        &self,
        target: &mut dyn ModFeatureTarget,
        random: &mut dyn ModFeatureRandom,
        origin: [i32; 3],
        config: &(dyn Any + Send + Sync),
    ) -> Result<bool, String> {
        let [x, y, z] = origin;
        match self.0 {
            Kind::Marks => {
                let draw = random.int(1);
                self.1.lock().unwrap().push((origin, draw));
                Ok(target.set_block(origin, BlockStateId(1), 3))
            }
            Kind::Column => {
                let height = *config.downcast_ref::<i32>().ok_or("config")?;
                for dy in 0..height {
                    target.set_block([x, y + dy, z], BlockStateId(1), 2);
                }
                Ok(false)
            }
            Kind::Fails => {
                target.set_block(origin, BlockStateId(2), 3);
                Err("no room".into())
            }
            Kind::Panics => {
                target.set_block(origin, BlockStateId(2), 3);
                panic!("test panic")
            }
            Kind::Flood => {
                target.set_block(origin, BlockStateId(2), 3);
                for _ in 0..70_000 {
                    target.block(origin);
                }
                Ok(true)
            }
            Kind::Refuses => {
                target.set_block(origin, BlockStateId(1), 3);
                target.set_block([x + 100, y, z], BlockStateId(1), 3);
                Ok(true)
            }
            Kind::Fill => {
                let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
                let at = |i: i32| [i % 48, 1 + i / 2304, (i / 48) % 48];
                for i in 0..8_000 {
                    target.set_block(at(i), BlockStateId(5), 3);
                    target.set_block_entity(at(i), chest.clone());
                }
                let missed = (8_000..16_000)
                    .filter(|&i| target.block(at(i)) == Some(BlockStateId(0)))
                    .count();
                self.1.lock().unwrap().push((origin, missed as i32));
                Ok(true)
            }
            Kind::SecondFails | Kind::SecondFloods => {
                let calls = {
                    let mut log = self.1.lock().unwrap();
                    log.push((origin, 0));
                    log.len()
                };
                let placed = target.set_block(origin, BlockStateId(5), 3);
                let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
                target.set_block_entity(origin, chest);
                if calls == 2 {
                    match self.0 {
                        Kind::SecondFails => return Err("second call".into()),
                        _ => {
                            for _ in 0..70_000 {
                                target.block(origin);
                            }
                        }
                    }
                }
                Ok(placed)
            }
            Kind::Chest => {
                let nbt = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
                let placed = target.set_block(origin, BlockStateId(5), 3);
                let own = target.set_block_entity(origin, nbt.clone());
                let beside = target.set_block_entity([x + 1, y, z], nbt.clone());
                let far = target.set_block_entity([i32::MAX, i32::MIN, i32::MAX], nbt);
                self.1.lock().unwrap().push((
                    origin,
                    i32::from(placed) * 4 + i32::from(own) * 2 + i32::from(beside || far),
                ));
                Ok(placed)
            }
        }
    }
}
fn types(log: &Log) -> ModFeatureTypes {
    [
        ("test:marks", Kind::Marks),
        ("test:column", Kind::Column),
        ("test:fails", Kind::Fails),
        ("test:panics", Kind::Panics),
        ("test:flood", Kind::Flood),
        ("test:chest", Kind::Chest),
        ("test:fill", Kind::Fill),
        ("test:refuses", Kind::Refuses),
        ("test:second_fails", Kind::SecondFails),
        ("test:second_floods", Kind::SecondFloods),
    ]
    .into_iter()
    .map(|(id, kind)| {
        let ty: Arc<dyn ModFeatureType> = Arc::new(Test(kind, log.clone()));
        (id.to_owned(), ty)
    })
    .collect()
}
fn feature(json: &str) -> FeatureDefinition {
    FeatureDefinition::from_json(json).unwrap()
}
fn place(feature: &FeatureDefinition, world: &mut World, origin: [i32; 3]) -> Result<bool, String> {
    feature
        .place(world, &mut LegacyRandom::new(1), origin, blocks())
        .map_err(|error| error.to_string())
}

/// A registered type's id decodes to the mod's feature, with its config decoded once; a config
/// its type refuses refuses the definition at load, naming the type; without the type the id is
/// the unknown type it was before.
#[test]
fn a_registered_type_decodes_and_a_bad_config_is_refused_at_load() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let FeatureDefinition::Mod(column) = feature(r#"{"type":"test:column","height":3}"#) else {
            panic!("not a mod feature")
        };
        assert_eq!(column.id, "test:column");
        assert_eq!(column.config.downcast_ref::<i32>(), Some(&3));
        let error = FeatureDefinition::from_json(r#"{"type":"test:column","height":99}"#)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("test:column") && error.contains("1 to 8"),
            "{error}"
        );
        assert!(FeatureDefinition::from_json(r#"{"type":"test:missing"}"#).is_err());
    });
    assert!(FeatureDefinition::from_json(r#"{"type":"test:column"}"#).is_err());
}

/// The feature draws from the caller's own random: its `nextInt(1)` is 0 and spends one
/// draw, so the caller's next draws are the jar's (`oracle_tests`: 13, 3, then
/// -7261648964369397258 from seed 0).
#[test]
fn the_feature_draws_from_the_callers_stream() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let marks = feature(r#"{"type":"test:marks"}"#);
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        let mut world = World::default();
        assert!(
            marks
                .place(&mut world, &mut random, [0, 4, 0], blocks())
                .unwrap()
        );
        assert_eq!(*log.lock().unwrap(), [([0, 4, 0], 0)]);
        assert_eq!(world.writes, [([0, 4, 0], BlockStateId(1), 3)]);
        assert_eq!(random.next_int_bound(16).unwrap(), 13);
        assert_eq!(random.next_int_bound(16).unwrap(), 3);
        assert_eq!(random.next_long(), -7261648964369397258);
    });
}

/// Through `count 2` then `in_square`, depth first as `FeaturePlacer.place` runs them: the
/// feature runs at the first position before `in_square` draws for the second. The expected
/// values are `java.util.Random`'s from seed 42 (the LCG vanilla's `LegacyRandomSource` is),
/// worked out apart from Sunrise: in_square draws 11, 0, the feature's `nextInt(1)` is 0,
/// in_square draws 0, 4, the feature 0, and the next `nextLong` is 5111195811822994797.
#[test]
fn modifiers_feed_the_feature_in_vanilla_order() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let placed = PlacedFeatureDefinition {
            feature: Box::new(feature(r#"{"type":"test:marks"}"#)),
            placement: vec![
                PlacementModifier::Count(IntProvider::Constant(2)),
                PlacementModifier::InSquare,
            ],
        };
        let mut random = WorldgenRandom::new(LegacyRandom::new(42));
        let mut world = World::default();
        assert!(
            placed
                .place(&mut world, &mut random, [16, 4, 32], blocks())
                .unwrap()
        );
        assert_eq!(*log.lock().unwrap(), [([27, 4, 32], 0), ([16, 4, 36], 0)]);
        assert_eq!(random.next_long(), 5111195811822994797);
    });
}

/// A position given twice is placed twice: the second call finds its own block already there,
/// so its write answers false, as `Level.setBlock` does for an unchanged state.
#[test]
fn a_duplicate_final_position_is_placed_twice() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let placed = PlacedFeatureDefinition {
            feature: Box::new(feature(r#"{"type":"test:marks"}"#)),
            placement: vec![PlacementModifier::Count(IntProvider::Constant(2))],
        };
        let mut world = World::default();
        // The first call placed, so the placement did.
        assert!(
            placed
                .place(&mut world, &mut LegacyRandom::new(1), [3, 4, 3], blocks())
                .unwrap()
        );
        assert_eq!(*log.lock().unwrap(), [([3, 4, 3], 0), ([3, 4, 3], 0)]);
        assert_eq!(world.writes, [([3, 4, 3], BlockStateId(1), 3)]);
    });
}

/// A nested feature (a random selector's default) reaches the mod's type, and `Ok(false)` keeps
/// what it wrote, as vanilla does.
#[test]
fn a_nested_feature_reaches_the_type_and_false_keeps_its_writes() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let selector = feature(
            r#"{"type":"minecraft:random_selector","features":[],
                "default":{"feature":{"type":"test:column","height":4},"placement":[]}}"#,
        );
        let mut world = World::default();
        assert_eq!(place(&selector, &mut world, [3, 12, 3]), Ok(false));
        assert_eq!(
            world.writes.iter().map(|w| w.0).collect::<Vec<_>>(),
            [[3, 12, 3], [3, 13, 3], [3, 14, 3], [3, 15, 3]]
        );
    });
}

/// A failure or a panic after a write discards that call's writes and disables the type for
/// the session: its next placement answers `Ok(false)` without calling it, and its root
/// carries on. Another type is not disabled with it, and a fresh load of the mod set enables it
/// again.
#[test]
fn faults_discard_their_writes_and_disable_the_type_until_the_mods_load_again() {
    let log = Log::default();
    let loaded = || types(&log);
    with_test_feature_types(&loaded(), || {
        let fails = feature(r#"{"type":"test:fails"}"#);
        let mut world = World::default();
        assert_eq!(place(&fails, &mut world, [0, 1, 0]), Ok(false));
        assert!(world.writes.is_empty());
        assert!(mod_feature_type("test:fails").unwrap().disabled());
        // Disabled: not called again, so nothing is written, and no error either.
        assert_eq!(place(&fails, &mut world, [0, 1, 0]), Ok(false));
        assert!(world.writes.is_empty());
        let nested = feature(
            r#"{"type":"minecraft:random_selector","features":[],
                "default":{"feature":{"type":"test:panics"},"placement":[]}}"#,
        );
        assert_eq!(place(&nested, &mut world, [0, 1, 0]), Ok(false));
        assert!(world.writes.is_empty());
        assert!(mod_feature_type("test:panics").unwrap().disabled());
        // Another type is not disabled with it.
        assert_eq!(
            place(&feature(r#"{"type":"test:marks"}"#), &mut world, [0, 1, 0]),
            Ok(true)
        );
        assert!(!mod_feature_type("test:marks").unwrap().disabled());
    });
    // A new load of the mod set builds its entries again: nothing is disabled.
    with_test_feature_types(&loaded(), || {
        assert!(!mod_feature_type("test:fails").unwrap().disabled());
        let mut world = World::default();
        let marks = feature(r#"{"type":"test:marks"}"#);
        assert_eq!(place(&marks, &mut world, [0, 1, 0]), Ok(true));
    });
}

/// A write the level refuses (outside the region decoration may write, or the build height) is
/// a fault: the call's writes before it are discarded too, and the type is disabled.
#[test]
fn a_refused_write_is_a_fault() {
    for (ty, refused_at) in [("test:refuses", None), ("test:column", Some(14))] {
        let log = Log::default();
        with_test_feature_types(&types(&log), || {
            let height = refused_at.map_or(String::new(), |_| r#","height":4"#.to_owned());
            let refuses = feature(&format!(r#"{{"type":"{ty}"{height}}}"#));
            let mut world = World::default();
            // `test:column` runs from y 14 to 17, past the build height at 16.
            assert_eq!(
                place(&refuses, &mut world, [3, refused_at.unwrap_or(1), 3]),
                Ok(false)
            );
            assert!(world.writes.is_empty(), "{ty}");
            assert!(mod_feature_type(ty).unwrap().disabled(), "{ty}");
        });
    }
}

/// A fault mid-root keeps the writes of the calls before it and nothing of the failed call's:
/// in a count-2 root whose second call fails, or runs the shared budget out, the first call's
/// block and block entity stand, the second's are gone, and the root still answers. A root
/// whose calls all succeed publishes them in order. The same seed twice gives the same world,
/// faults included.
#[test]
fn a_fault_mid_root_keeps_the_earlier_calls() {
    let rooted = |ty: &str| PlacedFeatureDefinition {
        feature: Box::new(feature(&format!(r#"{{"type":"{ty}"}}"#))),
        placement: vec![
            PlacementModifier::Count(IntProvider::Constant(2)),
            PlacementModifier::InSquare,
        ],
    };
    let run = |ty: &str| {
        let log = Log::default();
        let mut world = World::default();
        let answer = with_test_feature_types(&types(&log), || {
            rooted(ty)
                .place(
                    &mut world,
                    &mut LegacyRandom::new(42),
                    [16, 4, 32],
                    blocks(),
                )
                .map_err(|error| error.to_string())
        });
        let calls = log.lock().unwrap().len();
        (answer, calls, world)
    };
    let positions = |world: &World| world.writes.iter().map(|w| w.0).collect::<Vec<_>>();

    for ty in ["test:second_fails", "test:second_floods"] {
        let (answer, calls, world) = run(ty);
        assert_eq!(calls, 2, "{ty}: the second call ran");
        assert!(answer.is_ok(), "{ty}: the root carries on");
        assert_eq!(positions(&world), [[27, 4, 32]], "{ty}");
        assert_eq!(
            world.entities.iter().map(|e| e.0).collect::<Vec<_>>(),
            [[27, 4, 32]],
            "{ty}"
        );
        let (_, _, again) = run(ty);
        assert_eq!(positions(&again), positions(&world), "{ty}: same seed");
        assert_eq!(again.entities.len(), world.entities.len(), "{ty}");
    }

    let (answer, calls, world) = run("test:marks");
    assert_eq!(answer, Ok(true));
    assert_eq!(calls, 2);
    assert_eq!(positions(&world), [[27, 4, 32], [16, 4, 36]]);
}

/// In one root a mod call writes state 1 at the origin, the built-in end platform then reads that
/// position, sees the mod's block and replaces it with air (state 0), and a later mod call faults:
/// the mod's block and the built-in's replacement both stay, and the faulting call's write
/// (state 2) does not.
#[test]
fn a_built_in_step_sees_a_mod_write_and_a_later_fault_keeps_both() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let root = feature(
            r#"{"type":"minecraft:sequence","features":[
                {"feature":{"type":"test:marks"},"placement":[]},
                {"feature":{"type":"minecraft:end_platform"},"placement":[]},
                {"feature":{"type":"test:fails"},"placement":[]}]}"#,
        );
        let origin = [10, 4, 10];
        let mut world = World::default();
        assert!(place(&root, &mut world, origin).is_ok());
        let at_origin: Vec<_> = world
            .writes
            .iter()
            .filter(|w| w.0 == origin)
            .map(|w| w.1)
            .collect();
        // The mod's block, then the built-in's replacement of it; the fault's block never.
        assert_eq!(at_origin, [BlockStateId(1), BlockStateId(0)]);
        assert_eq!(world.block(origin), Ok(BlockStateId(0)));
        // The platform's obsidian layer (5 by 5) is the built-in's own writes.
        assert_eq!(
            world
                .writes
                .iter()
                .filter(|w| w.1 == BlockStateId(4))
                .count(),
            25
        );
        assert!(world.writes.iter().all(|w| w.1 != BlockStateId(2)));
        assert!(mod_feature_type("test:fails").unwrap().disabled());
    });
}

/// A call that writes thousands of blocks with block entities, then reads thousands of other
/// positions, stays within the budget and publishes every write and entity once: the staged
/// state of a position is one lookup, not a scan of all that came before.
#[test]
fn a_root_of_thousands_of_writes_publishes_them_all() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let mut world = World::default();
        assert_eq!(
            place(&feature(r#"{"type":"test:fill"}"#), &mut world, [0, 1, 0]),
            Ok(true)
        );
        assert_eq!(*log.lock().unwrap(), [([0, 1, 0], 8_000)]);
        assert_eq!(world.writes.len(), 8_000);
        assert_eq!(world.entities.len(), 8_000);
    });
}

/// A call past the root's custom budget places nothing and faults, which disables the type; the
/// next root has the whole budget again for the other types, and the graph's own budget is
/// untouched. The budget counts work units, so the outcome does not depend on speed.
#[test]
fn the_custom_budget_bounds_a_call_and_faults_it() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let flood = feature(r#"{"type":"test:flood"}"#);
        let mut world = World::default();
        assert_eq!(place(&flood, &mut world, [0, 1, 0]), Ok(false));
        assert!(world.writes.is_empty());
        assert!(mod_feature_type("test:flood").unwrap().disabled());
        // Another type: a fresh root places a feature of a smaller job.
        let marks = feature(r#"{"type":"test:marks"}"#);
        assert_eq!(place(&marks, &mut world, [0, 1, 0]), Ok(true));
        // Several calls in one root share its budget, and small ones fit it.
        let twice = feature(
            r#"{"type":"minecraft:random_selector","features":[],
                "default":{"feature":{"type":"test:marks"},"placement":[{"type":"minecraft:count","count":3}]}}"#,
        );
        assert!(place(&twice, &mut world, [1, 1, 1]).is_ok());
    });
}

/// A write onto the same state is false, as the level answers it; reads and tags see the call's
/// own staged blocks, and a read the world can't answer is `None`.
#[test]
fn staged_writes_answer_as_the_world_would_and_read_back() {
    struct Probe;
    impl ModFeatureType for Probe {
        fn prepare_config(&self, _: &serde_json::Value) -> Result<ModFeatureConfig, String> {
            Ok(Arc::new(()))
        }
        fn place(
            &self,
            t: &mut dyn ModFeatureTarget,
            _: &mut dyn ModFeatureRandom,
            o: [i32; 3],
            _: &(dyn Any + Send + Sync),
        ) -> Result<bool, String> {
            let answers = [
                t.block_in_tag(o, "minecraft:dirt") == Some(false),
                t.set_block(o, BlockStateId(1), 3),
                t.block(o) == Some(BlockStateId(1)),
                t.block_in_tag(o, "minecraft:dirt") == Some(true),
                !t.set_block(o, BlockStateId(1), 3),
                t.block([i32::MAX, 5, 5]).is_none(),
            ];
            Ok(answers.iter().all(|a| *a))
        }
    }
    let types: ModFeatureTypes = [(
        "test:probe".to_owned(),
        Arc::new(Probe) as Arc<dyn ModFeatureType>,
    )]
    .into();
    with_test_feature_types(&types, || {
        let mut world = World::default();
        assert_eq!(
            place(&feature(r#"{"type":"test:probe"}"#), &mut world, [5, 5, 5]),
            Ok(true)
        );
        assert_eq!(world.writes, [([5, 5, 5], BlockStateId(1), 3)]);
    });
}

/// Block-entity data is taken only for the block the call placed there: beside it and at
/// extreme coordinates it is refused, and it reaches the world after its block.
#[test]
fn block_entities_only_for_the_calls_own_blocks() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let chest = feature(r#"{"type":"test:chest"}"#);
        let mut world = World::default();
        assert_eq!(place(&chest, &mut world, [2, 2, 2]), Ok(true));
        // Placed (4) and its own data taken (2); beside and far refused (0).
        assert_eq!(*log.lock().unwrap(), [([2, 2, 2], 6)]);
        assert_eq!(world.writes, [([2, 2, 2], BlockStateId(5), 3)]);
        assert_eq!(
            world.entities.iter().map(|e| e.0).collect::<Vec<_>>(),
            [[2, 2, 2]]
        );
        // Outside the write zone nothing is placed, so no data is taken either.
        log.lock().unwrap().clear();
        let mut world = World::default();
        assert_eq!(place(&chest, &mut world, [50, 2, 2]), Ok(false));
        assert_eq!(*log.lock().unwrap(), [([50, 2, 2], 0)]);
        assert!(world.writes.is_empty() && world.entities.is_empty());
    });
}

/// The random adapter: a bound below 1 answers 0 without a draw.
#[test]
fn a_bound_below_one_draws_nothing() {
    let mut random = LegacyRandom::new(3);
    let mut copy = random.clone();
    assert_eq!(Draws(&mut random).int(0), 0);
    assert_eq!(random.next_long(), copy.next_long());
}
