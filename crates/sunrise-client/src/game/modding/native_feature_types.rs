//! Native worker adapter for API-owned feature behaviours: the engine's `ModFeatureType` around a
//! mod's `FeatureType`, over the caller's own world and random.
use super::native_worldgen::WorkerResources;
use mod_api::{
    BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, DatapackEntry, DatapackField,
    FeatureContext, FeatureType, HeightmapType, RandomSource,
};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_world::registry::BlockRegistry;
use sunrise_world::worldgen::mod_features::{
    ModFeatureConfig, ModFeatureRandom, ModFeatureTarget, ModFeatureType, ModFeatureTypes,
};

fn engine_heightmap(kind: HeightmapType) -> sunrise_world::heightmap::HeightmapType {
    use sunrise_world::heightmap::HeightmapType as Engine;
    match kind {
        HeightmapType::WorldSurfaceWg => Engine::WorldSurfaceWg,
        HeightmapType::WorldSurface => Engine::WorldSurface,
        HeightmapType::OceanFloorWg => Engine::OceanFloorWg,
        HeightmapType::OceanFloor => Engine::OceanFloor,
        HeightmapType::MotionBlocking => Engine::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves => Engine::MotionBlockingNoLeaves,
        _ => Engine::WorldSurfaceWg,
    }
}

/// The caller's random, as a [`RandomSource`].
struct Draws<'a>(&'a mut dyn ModFeatureRandom);
impl RandomSource for Draws<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.int(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.long()
    }
    fn next_float(&mut self) -> f32 {
        self.0.float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.double()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.boolean()
    }
    fn next_int(&mut self) -> i32 {
        self.0.int_unbounded()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.gaussian()
    }
}

/// The engine's target, as a [`FeatureContext`].
struct Placing<'a> {
    target: &'a mut dyn ModFeatureTarget,
    worker: &'static WorkerResources,
    registry: &'static BlockRegistry,
    random: Draws<'a>,
    config: &'a DatapackEntry,
}

impl FeatureContext for Placing<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn config(&self) -> &DatapackEntry {
        self.config
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        &mut self.random
    }
    fn min_y(&mut self) -> Option<i32> {
        self.target.min_y()
    }
    fn max_y(&mut self) -> Option<i32> {
        self.target.max_y()
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker.state_with(self.registry, block, properties)
    }
    fn is_air(&self, state: BlockState) -> bool {
        self.worker.is_air(state)
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState> {
        let state = self.target.block(pos)?;
        self.worker.index.to_api(state).ok()
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        self.target.block_in_tag(pos, tag)
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool {
        let Ok(state) = self.worker.index.to_engine(state) else {
            return false;
        };
        self.target.set_block(pos, state, flags.bits())
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        self.target.height(engine_heightmap(kind), x, z)
    }
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> bool {
        // The data must be the block entity of the block this call placed there, as the game's
        // own block-entity loading checks it (`decode_block_entity_for_block`); the target
        // refuses a position the call placed no block at.
        let Some(name) = self
            .target
            .block(pos)
            .and_then(|state| self.registry.state(state))
            .map(|state| state.name.to_string())
        else {
            return false;
        };
        let mut placed = nbt.clone();
        for (axis, value) in ["x", "y", "z"].into_iter().zip(pos) {
            placed.insert(axis.into(), Tag::Int(value));
        }
        sunrise_client::save::decode_block_entity_for_block(&Tag::Compound(placed), &name).is_some()
            && self.target.set_block_entity(pos, nbt)
    }
}

/// The adapter: the game's `ModFeatureType` around a mod's `FeatureType`.
pub(crate) struct Adapter {
    inner: Arc<dyn FeatureType>,
    schema: Vec<DatapackField>,
}

impl Adapter {
    pub(crate) fn new(schema: Vec<DatapackField>, inner: Arc<dyn FeatureType>) -> Self {
        Self { inner, schema }
    }
}

impl ModFeatureType for Adapter {
    fn prepare_config(&self, config: &serde_json::Value) -> Result<ModFeatureConfig, String> {
        let entry = mod_api::__private::decode_feature_config(&self.schema, config)?;
        Ok(Arc::new(entry))
    }
    fn place(
        &self,
        target: &mut dyn ModFeatureTarget,
        random: &mut dyn ModFeatureRandom,
        origin: [i32; 3],
        config: &(dyn Any + Send + Sync),
    ) -> Result<bool, String> {
        let Some(registry) = target.registry() else {
            return Err("this world has no block registry for mod features".into());
        };
        let config = config
            .downcast_ref::<DatapackEntry>()
            .ok_or("config was not decoded by this adapter")?;
        let worker = WorkerResources::get(registry).map_err(|error| format!("{error}"))?;
        let mut context = Placing {
            target,
            worker,
            registry,
            random: Draws(random),
            config,
        };
        self.inner
            .place(&mut context, origin)
            .map_err(|error| error.0)
    }
}

#[cfg(test)]
#[path = "native_feature_types/tests.rs"]
mod tests;

/// Registered API declarations become engine adapters once, not once per chunk.
pub(in crate::game) fn installed_feature_types() -> ModFeatureTypes {
    mod_api::__private::feature_type_declarations()
        .into_iter()
        .map(|(id, schema, ty)| {
            let adapter: Arc<dyn ModFeatureType> = Arc::new(Adapter::new(schema, ty));
            (id, adapter)
        })
        .collect()
}
