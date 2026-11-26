use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_world::{
    heightmap::HeightmapType,
    registry::BlockStateId,
    worldgen::{
        anchor::WorldGenerationContext,
        feature::{FeatureBlocks, FeatureWorld},
        feature_graph::{FeatureDefinition, PlacedFeatureDefinition},
        placement::{PlacementContext, PlacementModifier, PlacementQueryError},
        value_provider::IntProvider,
    },
};

struct World {
    height_queries: usize,
}
impl PlacementContext for World {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 16, 8)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        self.height_queries += 1;
        Ok(8)
    }
}
impl FeatureWorld for World {
    type Error = std::convert::Infallible;
    fn block(&mut self, _: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        panic!("no-op graph read blocks")
    }
    fn set_block(&mut self, _: [i32; 3], _: BlockStateId, _: i32) -> Result<bool, Self::Error> {
        panic!("no-op graph wrote blocks")
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

#[test]
fn nested_count_chain_exceeds_former_graph_quota_without_drawing_randomness() {
    let probe = PlacedFeatureDefinition {
        feature: Box::new(FeatureDefinition::NoOp),
        placement: vec![PlacementModifier::Heightmap(HeightmapType::WorldSurface)],
    };
    let inner = PlacedFeatureDefinition {
        feature: Box::new(FeatureDefinition::Sequence(vec![probe])),
        placement: vec![PlacementModifier::Count(IntProvider::Constant(41))],
    };
    let middle = PlacedFeatureDefinition {
        feature: Box::new(FeatureDefinition::Sequence(vec![inner])),
        placement: vec![PlacementModifier::Count(IntProvider::Constant(41))],
    };
    let root = PlacedFeatureDefinition {
        feature: Box::new(FeatureDefinition::Sequence(vec![middle])),
        placement: vec![PlacementModifier::Count(IntProvider::Constant(41))],
    };
    let mut world = World { height_queries: 0 };
    let mut random = LegacyRandom::new(5);
    let mut expected = random.clone();
    assert!(
        root.place(&mut world, &mut random, [0; 3], blocks())
            .unwrap()
    );
    assert_eq!(world.height_queries, 68_921);
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn nested_inline_decoding_rejects_excessive_depth() {
    let mut value = String::from(r#"{"type":"no_op"}"#);
    for _ in 0..70 {
        value =
            format!(r#"{{"type":"sequence","features":[{{"feature":{value},"placement":[]}}]}}"#);
    }
    assert!(FeatureDefinition::from_json(&value).is_err());
}
