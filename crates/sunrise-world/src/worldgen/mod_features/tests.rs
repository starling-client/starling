//! Mod feature types through the engine: decoding, the caller's random, the placement chain's
//! order, nesting, direct writes, failures and block entities.
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
        if !self.ensure_can_write(p)? || !self.inside_build_height(p)? || self.block(p)? == s {
            return Ok(false);
        }
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
    fn height(&mut self, _: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        Ok((0..16)
            .rev()
            .find(|&y| {
                self.block([x, y, z])
                    .is_ok_and(|state| state != BlockStateId(0))
            })
            .map_or(0, |y| y + 1))
    }
    fn ensure_can_write(&mut self, p: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok((0..48).contains(&p[0]) && (0..48).contains(&p[2]))
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

#[derive(Clone, Copy)]
enum Kind {
    Marks,
    Column,
    Fails,
    Reads,
    Chest,
    SecondFails,
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
                let height = *config.downcast_ref::<i32>().unwrap();
                for dy in 0..height {
                    target.set_block([x, y + dy, z], BlockStateId(1), 2);
                }
                Ok(false)
            }
            Kind::Fails => {
                target.set_block(origin, BlockStateId(2), 3);
                Err("no room".to_owned())
            }
            Kind::Reads => {
                let read_all = (0..70_000).all(|_| target.block(origin) == Some(BlockStateId(0)));
                Ok(read_all && target.set_block(origin, BlockStateId(2), 3))
            }
            Kind::Refuses => {
                let outside = [x + 48, y, z];
                assert!(target.ensure_can_write(origin));
                assert!(!target.ensure_can_write(outside));
                assert!(target.set_block(origin, BlockStateId(1), 3));
                Ok(target.set_block(outside, BlockStateId(1), 2))
            }
            Kind::SecondFails => {
                let draw = random.int(1);
                let calls = {
                    let mut log = self.1.lock().unwrap();
                    log.push((origin, draw));
                    log.len()
                };
                let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
                assert!(target.set_block(origin, BlockStateId(5), 3));
                assert!(target.set_block_entity(origin, chest));
                if calls == 2 {
                    Err("second call".to_owned())
                } else {
                    Ok(true)
                }
            }
            Kind::Chest => {
                let nbt = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
                let own = target.set_block_entity(origin, nbt.clone());
                assert!(!target.set_block_entity([x + 48, y, z], nbt));
                Ok(own)
            }
        }
    }
}
fn types(log: &Log) -> ModFeatureTypes {
    [
        ("test:marks", Kind::Marks),
        ("test:column", Kind::Column),
        ("test:fails", Kind::Fails),
        ("test:reads", Kind::Reads),
        ("test:chest", Kind::Chest),
        ("test:refuses", Kind::Refuses),
        ("test:second_fails", Kind::SecondFails),
    ]
    .into_iter()
    .map(|(id, kind)| {
        let entry = ModFeatureEntry {
            mod_id: "test",
            ty: Arc::new(Test(kind, log.clone())),
        };
        (id.to_owned(), Arc::new(entry))
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

#[test]
fn a_refused_write_keeps_earlier_writes_and_later_invocations() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let refuses = feature(r#"{"type":"test:refuses"}"#);
        let mut world = World::default();
        for origin in [[3, 1, 3], [4, 1, 3]] {
            assert_eq!(place(&refuses, &mut world, origin), Ok(false));
        }
        assert_eq!(
            world.writes,
            [
                ([3, 1, 3], BlockStateId(1), 3),
                ([4, 1, 3], BlockStateId(1), 3)
            ]
        );
    });
}

#[test]
fn a_nested_call_error_keeps_its_blocks_entities_and_random_draws() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let root = feature(
            r#"{"type":"minecraft:random_selector","features":[],"default":{
                "feature":{"type":"test:second_fails"},"placement":[
                    {"type":"minecraft:count","count":3},{"type":"minecraft:in_square"}]}}"#,
        );
        let mut world = World::default();
        let mut random = LegacyRandom::new(42);
        assert!(
            root.place(&mut world, &mut random, [16, 4, 32], blocks())
                .unwrap()
        );
        // From the Java random sequence in depth-first order: two nextInt(16) and one
        // nextInt(1) per placement, including the invocation that returned the error.
        let positions = [[27, 4, 32], [16, 4, 36], [20, 4, 43]];
        assert_eq!(*log.lock().unwrap(), positions.map(|p| (p, 0)));
        assert_eq!(world.writes, positions.map(|p| (p, BlockStateId(5), 3)));
        let chest = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
        assert_eq!(world.entities, positions.map(|p| (p, chest.clone())));
        assert_eq!(random.next_long(), 1684641590762760125);
    });
}

/// In one root a mod call writes state 1 at the origin, the built-in end platform then reads that
/// position, sees the mod's block and replaces it with air (state 0), and a later mod call
/// writes state 2 before returning an error. All three writes happened, in order.
#[test]
fn a_built_in_step_reads_the_preceding_mod_write() {
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
        assert_eq!(
            at_origin,
            [BlockStateId(1), BlockStateId(0), BlockStateId(2)]
        );
        assert_eq!(world.block(origin), Ok(BlockStateId(2)));
        // The platform's obsidian layer (5 by 5) is the built-in's own writes.
        assert_eq!(
            world
                .writes
                .iter()
                .filter(|w| w.1 == BlockStateId(4))
                .count(),
            25
        );
    });
}

#[test]
fn more_than_65536_valid_reads_do_not_end_placement() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let mut world = World::default();
        assert_eq!(
            place(&feature(r#"{"type":"test:reads"}"#), &mut world, [0, 1, 0]),
            Ok(true)
        );
        assert_eq!(world.writes, [([0, 1, 0], BlockStateId(2), 3)]);
    });
}

#[test]
fn immediate_reads_see_the_updated_block_tag_and_height() {
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
            let [x, y, z] = o;
            assert_eq!(t.block_in_tag(o, "minecraft:dirt"), Some(false));
            assert_eq!(t.height(HeightmapType::WorldSurfaceWg, x, z), Some(0));
            assert!(t.set_block(o, BlockStateId(1), 3));
            assert_eq!(t.block(o), Some(BlockStateId(1)));
            assert_eq!(t.block_in_tag(o, "minecraft:dirt"), Some(true));
            assert_eq!(t.height(HeightmapType::WorldSurfaceWg, x, z), Some(y + 1));
            assert!(!t.set_block(o, BlockStateId(1), 3));
            assert_eq!(t.block([i32::MAX, 5, 5]), None);
            Ok(true)
        }
    }
    let types: ModFeatureTypes = [(
        "test:probe".to_owned(),
        Arc::new(ModFeatureEntry {
            mod_id: "test",
            ty: Arc::new(Probe),
        }),
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

#[test]
fn an_existing_block_can_take_block_entity_data() {
    let log = Log::default();
    with_test_feature_types(&types(&log), || {
        let chest = feature(r#"{"type":"test:chest"}"#);
        let mut world = World::default();
        let origin = [2, 2, 2];
        assert!(world.set_block(origin, BlockStateId(5), 3).unwrap());
        assert_eq!(place(&chest, &mut world, origin), Ok(true));
        assert_eq!(world.writes, [(origin, BlockStateId(5), 3)]);
        let nbt = Compound::from([("id".into(), Tag::String("minecraft:chest".into()))]);
        assert_eq!(world.entities, [(origin, nbt)]);
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
