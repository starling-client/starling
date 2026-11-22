//! Native simple-block feature selection, survival and ordered placement.
use super::feature::FeatureWorld;
use super::{
    DecodeError, Object, optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
    state_provider::{
        BlockStateProvider, StateProviderBindings, StateProviderCatalog, StateProviderError,
    },
};
use crate::block_placement::{BlockPlacement, BlockPlacementBindings, BlockPlacementError};
use std::sync::Arc;
use sunrise_core::random::RandomSource;

/// Immutable state providers and placement transitions share one registry.
#[derive(Clone, Debug)]
pub struct SimpleBlockFeature {
    provider: BlockStateProvider,
    placement: Arc<BlockPlacement>,
    schedule_tick: bool,
}
#[derive(Debug)]
pub enum SimpleBlockError<E> {
    Provider(StateProviderError<E>),
    Placement(BlockPlacementError<E>),
    Query(PlacementQueryError),
}
impl<E: std::fmt::Display> std::fmt::Display for SimpleBlockError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => e.fmt(f),
            Self::Placement(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SimpleBlockError<E> {}
impl SimpleBlockFeature {
    /// Decode the feature's map codec, including optional named providers.
    pub fn from_json(
        json: &str,
        providers: &StateProviderBindings<'_>,
        catalog: &StateProviderCatalog,
        placement: &BlockPlacementBindings<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, providers, catalog, placement, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        providers: &StateProviderBindings<'_>,
        catalog: &StateProviderCatalog,
        placement: &BlockPlacementBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if !std::ptr::eq(providers.registry(), placement.registry()) {
            return Err(DecodeError::Field("simple block registry identity"));
        }
        let object: Object = serde_json::from_str(json)?;
        let provider = catalog.provider_with_budget(
            required(&object, "to_place")?.get(),
            providers,
            depth + 1,
            remaining,
        )?;
        let schedule_tick = optional(&object, "schedule_tick")
            .map(|v| serde_json::from_str::<bool>(v.get()))
            .transpose()?
            .unwrap_or(false);
        Ok(Self {
            provider,
            placement: placement.operations(),
            schedule_tick,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        position: [i32; 3],
    ) -> Result<bool, SimpleBlockError<W::Error>> {
        let Some(state) = self
            .provider
            .get_optional_state(world, random, position)
            .map_err(SimpleBlockError::Provider)?
        else {
            return Ok(false);
        };
        if !world
            .block_survives(state, position)
            .map_err(SimpleBlockError::Query)?
        {
            return Ok(false);
        }
        if !self
            .placement
            .place(world, state, position, 2)
            .map_err(SimpleBlockError::Placement)?
        {
            return Ok(false);
        }
        if self.schedule_tick {
            self.placement
                .schedule_tick(world, position, 1)
                .map_err(SimpleBlockError::Placement)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block_properties::BlockPropertiesRegistry,
        collision::CachedCollisionRegistry,
        heightmap::HeightmapType,
        registry::{BlockRegistry, BlockStateId},
        worldgen::anchor::WorldGenerationContext,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_core::{
        nbt::{Compound, Tag},
        random::LegacyRandom,
    };

    #[derive(Default)]
    struct World(BTreeMap<[i32; 3], BlockStateId>);
    impl FeatureWorld for World {
        type Error = ();
        fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, ()> {
            Ok(self.0.get(&p).copied().unwrap_or(BlockStateId(0)))
        }
        fn set_block(&mut self, p: [i32; 3], state: BlockStateId, _: i32) -> Result<bool, ()> {
            self.0.insert(p, state);
            Ok(true)
        }
    }
    impl PlacementContext for World {
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            Err(PlacementQueryError::Unavailable)
        }
        fn block_survives(
            &mut self,
            _: BlockStateId,
            _: [i32; 3],
        ) -> Result<bool, PlacementQueryError> {
            Ok(true)
        }
    }

    #[test]
    fn simple_block_places_registered_double_plant_halves() {
        let names = [
            "minecraft:air",
            "minecraft:cave_air",
            "minecraft:void_air",
            "minecraft:stone",
        ];
        let mut states: Vec<Tag> = names
            .iter()
            .map(|name| Tag::String((*name).into()))
            .collect();
        // Placement binds all 2 * 3^4 moss transitions even when the feature
        // only places plants. These are synthetic states, not prepared game data.
        for bottom in ["false", "true"] {
            for code in 0..81 {
                let mut n = code;
                let mut props = Compound::from([("bottom".into(), Tag::String(bottom.into()))]);
                for side in ["north", "east", "south", "west"] {
                    props.insert(
                        side.into(),
                        Tag::String(["none", "low", "tall"][n % 3].into()),
                    );
                    n /= 3;
                }
                states.push(Tag::compound([
                    ("id", Tag::String("minecraft:pale_moss_carpet".into())),
                    ("properties", Tag::Compound(props)),
                ]));
            }
        }
        let mut defaults: Compound = names
            .iter()
            .enumerate()
            .map(|(i, name)| ((*name).into(), Tag::Int(i as i32)))
            .collect();
        defaults.insert("minecraft:pale_moss_carpet".into(), Tag::Int(4));
        for name in ["example:wild_crop", "minecraft:tall_grass"] {
            defaults.insert(name.into(), Tag::Int(states.len() as i32));
            for half in ["lower", "upper"] {
                states.push(Tag::compound([
                    ("id", Tag::String(name.into())),
                    (
                        "properties",
                        Tag::compound([("half", Tag::String(half.into()))]),
                    ),
                ]));
            }
        }
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states.clone())),
            ("defaults", Tag::Compound(defaults)),
        ]))
        .unwrap();
        let empty = Tag::compound([
            ("x", Tag::LongArray(vec![0])),
            ("y", Tag::LongArray(vec![0])),
            ("z", Tag::LongArray(vec![0])),
            ("occupied", Tag::ByteArray(vec![])),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![0; 3])),
        ]);
        let collision_tag = Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states.clone())),
            ("shapes", Tag::List(vec![empty])),
            ("cached_shapes", Tag::IntArray(vec![0; states.len()])),
        ]);
        let collisions = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
        let blocks = names
            .iter()
            .copied()
            .chain([
                "minecraft:pale_moss_carpet",
                "example:wild_crop",
                "minecraft:tall_grass",
            ])
            .map(|name| (name.into(), Tag::Byte(0)))
            .collect();
        let properties = BlockPropertiesRegistry::from_nbt(
            &Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states)),
                ("blocks", Tag::Compound(blocks)),
                ("occlusion", collision_tag),
            ]),
            &collisions,
        )
        .unwrap();

        let behaviours = serde_json::json!({"blocks":[
            {"name":"example:wild_crop", "class":["WildCropBlock", "DoublePlantBlock", "Block"]},
            {"name":"minecraft:tall_grass", "class":["TallGrassBlock", "DoublePlantBlock", "Block"]},
            {"name":"minecraft:stone", "class":["Block"]}
        ]});
        let double_plants =
            super::super::generator_blocks::double_plant_blocks(&behaviours).unwrap();
        let placement =
            BlockPlacementBindings::new(&registry, &properties, &double_plants, &BTreeSet::new())
                .unwrap();
        let providers =
            StateProviderBindings::new(&registry, BTreeSet::new(), &BTreeMap::new()).unwrap();
        let catalog = StateProviderCatalog::from_resources(&BTreeMap::new()).unwrap();
        for name in ["example:wild_crop", "minecraft:tall_grass"] {
            let feature = SimpleBlockFeature::from_json(
                &format!(r#"{{"to_place":{{"type":"minecraft:simple","state":{{"id":"{name}","properties":{{"half":"upper"}}}}}}}}"#),
                &providers, &catalog, &placement,
            ).unwrap();
            let lower = registry.default_state(name).unwrap();
            let mut upper_state = registry.state(lower).unwrap().clone();
            upper_state.properties.insert("half".into(), "upper".into());
            let upper = registry.id(&upper_state).unwrap();
            let mut world = World::default();
            assert!(
                feature
                    .place(&mut world, &mut LegacyRandom::new(1), [0, 1, 0])
                    .unwrap()
            );
            assert_eq!(
                world.0,
                BTreeMap::from([([0, 1, 0], lower), ([0, 2, 0], upper)])
            );
            // A blocked upper position refuses the entire placement.
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut world = World(BTreeMap::from([([0, 2, 0], stone)]));
            assert!(
                !feature
                    .place(&mut world, &mut LegacyRandom::new(1), [0, 1, 0])
                    .unwrap()
            );
            assert_eq!(world.0, BTreeMap::from([([0, 2, 0], stone)]));
        }
    }
}
