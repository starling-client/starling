//! FEATURES status over a 3×3 neighborhood of generated chunks.
//!
//! Placed features are resolved by name from the bundled feature registries.
//! Roots whose algorithm or bindings are not implemented are left out of
//! execution (but keep their place in vanilla's per-step feature ordering, so
//! decoration seeds of the remaining features are unchanged).
use super::generator_blocks::BlockBehavior;
use super::{
    anchor::WorldGenerationContext,
    biome::BiomeId,
    biome_chunk::BiomeRegion,
    biome_source::ResolvedBiomeSource,
    biome_temperature::BiomeTemperature,
    biome_weather::BiomeWeatherBindings,
    block_feature::FeatureBlockBindings,
    decoration::{DecorationWorld, FeatureDecorationPlan},
    feature::{FeatureBlocks, FeatureWorld},
    feature_catalog::{BiomePlacedFeature, FeatureCatalog},
    feature_graph::PlacedFeatureDefinition,
    feature_region::{RegionError, TerrainRegion},
    feature_sort::FeatureIdentity,
    generator::{
        DecorateStats, DecorationDependencies, GenBlockStates, GeneratorData, GeneratorError,
        ProtoChunk, WorkerState,
    },
    placement::{LayerBlock, PlacementContext, PlacementQueryError},
    terrain::PreLightChunkStage,
    write_zone::GenerationWriteZone,
};
use crate::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    block_support::SupportType,
    block_survival::SurvivalWorld,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidKind, FluidState},
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
};
use serde_json::Value;
use std::collections::HashMap;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use sunrise_core::random::{WorldgenRandom, XoroshiroRandom};
use sunrise_core::{
    biome::obfuscate_seed,
    geometry::{Axis, Direction, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use {
    super::structure::{
        jigsaw::FeatureKey,
        start::{STEPS, place_in_chunk},
    },
    sunrise_core::bounding_box::BoundingBox,
};

/// `ChunkPyramid.GENERATION_PYRAMID` sets the FEATURES block-state write radius to one chunk.
const FEATURES_WRITE_RADIUS: i32 = 1;
/// `ChunkPyramid.GENERATION_PYRAMID` gives FEATURES `STRUCTURE_STARTS` direct dependencies
/// out to eight chunks (`directDependencies().size() == 9`).
const FEATURES_DEPENDENCY_RADIUS: i32 = 8;

type PieceFeatures = HashMap<String, Option<Arc<PlacedFeatureDefinition>>>;

pub(super) struct Decoration {
    plan: FeatureDecorationPlan,
    names: Vec<String>,
    /// Executable roots by identity, or why the root cannot be executed.
    features: Vec<Result<Arc<PlacedFeatureDefinition>, String>>,
    /// Placed features of each biome's generation settings, by BiomeId: a
    /// bit per FeatureIdentity.
    biome_features: Vec<Vec<u64>>,
    bindings: FeatureBlockBindings<'static>,
    weather: Option<BiomeWeatherBindings>,
    blocks: FeatureBlocks,
    catalog: FeatureCatalog,
    /// Features structure pieces place by id, decoded lazily: placed
    /// features (feature pool elements), then configured ones.
    named: std::sync::Mutex<(PieceFeatures, PieceFeatures)>,
    /// BlockBehaviour.Properties.postProcess: states whose worldgen writes mark
    /// their own position (mushrooms) or the one above (soul sand, magma).
    postprocess_self: Vec<BlockStateId>,
    postprocess_above: Vec<BlockStateId>,
    /// A superflat world's `FillLayerFeature`s (`TOP_LAYER_MODIFICATION`): the level Y and state
    /// of each layer that is not opaque to `MOTION_BLOCKING`, filled after the structures.
    pub(super) top_layers: Vec<(i32, BlockStateId)>,
}
fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}
/// Biomes in vanilla's possible-biome encounter order for this source.
fn possible_biomes(source: &ResolvedBiomeSource) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let ordered: Vec<String> = match source {
        ResolvedBiomeSource::MultiNoise(source) => source
            .parameters()
            .entries()
            .iter()
            .map(|(_, name)| name.clone())
            .collect(),
        ResolvedBiomeSource::Regions(source) => source.biomes().map(str::to_owned).collect(),
        ResolvedBiomeSource::End(source) => [
            &source.center,
            &source.highlands,
            &source.midlands,
            &source.islands,
            &source.barrens,
        ]
        .into_iter()
        .cloned()
        .collect(),
        other => other
            .possible_biomes()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    };
    ordered
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}
/// `FlatLevelGeneratorSettings.adjustGenerationSettings`: what a superflat world keeps of its
/// biome's generation settings.
#[derive(Clone, Copy, Debug)]
pub(super) struct FlatDecoration {
    /// `(!voidGen || biome is the_void) && decoration`: the biome's own placed features, but
    /// those of the two structure steps (and of the lakes step when `lakes` is on).
    pub(super) features: bool,
    /// `addLakes`: the lakes step is `createLakesList`.
    pub(super) lakes: bool,
}
/// `GenerationStep.Decoration` ordinals `adjustGenerationSettings` treats on their own.
const LAKES_STEP: usize = 1;
const UNDERGROUND_STRUCTURES_STEP: usize = 3;
const SURFACE_STRUCTURES_STEP: usize = 4;
/// `FlatLevelGeneratorSettings.createLakesList`.
const FLAT_LAKES: [&str; 2] = [
    "minecraft:lake_lava_underground",
    "minecraft:lake_lava_surface",
];
impl FlatDecoration {
    /// None of the biome's entries: the structure pieces alone, for a mod's chunk generator that
    /// asks for no features.
    pub(super) const NONE: Self = Self {
        features: false,
        lakes: false,
    };
    /// The settings' `features` and `lakes`, for layers that are all air (`void_gen`) or not and a
    /// biome that is the_void or not.
    pub(super) fn new(features: bool, lakes: bool, void_gen: bool, the_void: bool) -> Self {
        Self {
            features: (!void_gen || the_void) && features,
            lakes,
        }
    }
    /// Which of the biome's own entries step `step` keeps.
    fn keeps_biome_step(self, step: usize) -> bool {
        self.features
            && step != UNDERGROUND_STRUCTURES_STEP
            && step != SURFACE_STRUCTURES_STEP
            && !(self.lakes && step == LAKES_STEP)
    }
}
impl Decoration {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        source: &ResolvedBiomeSource,
    ) -> Result<Option<Self>, GeneratorError> {
        Self::new_with(data, source, None)
    }
    /// `new` for a superflat world (`flat`): its fixed biome's settings as
    /// `adjustGenerationSettings` leaves them. The steps stay (structures place in them);
    /// the `FillLayerFeature`s are `top_layers`.
    pub(super) fn new_with(
        data: &Arc<GeneratorData>,
        source: &ResolvedBiomeSource,
        flat: Option<FlatDecoration>,
    ) -> Result<Option<Self>, GeneratorError> {
        let Some(resources) = &data.feature_resources else {
            return Ok(None);
        };
        let registry = data.registry();
        let catalog = FeatureCatalog::from_resources(
            &resources.features,
            &resources.placed,
            &resources.placed_tags,
        )?;
        let bindings = data.geometry_feature_bindings(resources)?;
        let mut ids = BTreeMap::<String, FeatureIdentity>::new();
        let mut names = Vec::new();
        let mut features = Vec::new();
        let mut named = HashMap::new();
        let mut identity = |holder,
                            biome: &str,
                            decoded: Option<Arc<PlacedFeatureDefinition>>|
         -> FeatureIdentity {
            let (name, json) = match holder {
                BiomePlacedFeature::Named(name) => {
                    match ids.entry(name.clone()) {
                        std::collections::btree_map::Entry::Occupied(entry) => return *entry.get(),
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(FeatureIdentity(names.len() as u32));
                        }
                    }
                    if let Some(decoded) = &decoded {
                        named.insert(name.clone(), Some(decoded.clone()));
                    }
                    let json = Value::from(name.as_str()).to_string();
                    (name, json)
                }
                BiomePlacedFeature::Direct(json) => (format!("inline feature in {biome}"), json),
            };
            let id = FeatureIdentity(names.len() as u32);
            names.push(name);
            features.push(match decoded {
                Some(decoded) => Ok(decoded),
                None => catalog
                    .placed_feature_with_blocks(&json, &bindings)
                    .map(Arc::new)
                    .map_err(|error| error.to_string()),
            });
            id
        };
        let mut biome_features = vec![Vec::new(); data.biome_names().len()];
        let mut plan = Vec::new();
        for name in possible_biomes(source) {
            let id = data
                .biome_id(&name)
                .ok_or_else(|| resource(format!("unknown biome {name}")))?;
            let mut steps = Vec::new();
            for (index, step) in data
                .biome_feature_steps(id, &catalog)?
                .into_iter()
                .enumerate()
            {
                let mut list = Vec::new();
                // The builder takes the lakes first, then the biome's steps it keeps.
                if flat.is_some_and(|f| f.lakes) && index == LAKES_STEP {
                    for lake in FLAT_LAKES {
                        let feature = identity(BiomePlacedFeature::Named(lake.into()), &name, None);
                        list.push(feature);
                        set_feature_bit(&mut biome_features[id.0 as usize], feature);
                    }
                }
                if flat.is_some_and(|f| !f.keeps_biome_step(index)) {
                    steps.push(list);
                    continue;
                }
                for (holder, decoded) in step {
                    let feature = identity(holder, &name, decoded);
                    list.push(feature);
                    set_feature_bit(&mut biome_features[id.0 as usize], feature);
                }
                steps.push(list);
            }
            // A biome with fewer steps still gets the lakes (`PlainBuilder` grows its list).
            if flat.is_some_and(|f| f.lakes) && steps.len() <= LAKES_STEP {
                steps.resize_with(LAKES_STEP, Vec::new);
                let lakes = FLAT_LAKES
                    .map(|id| identity(BiomePlacedFeature::Named(id.into()), &name, None))
                    .to_vec();
                for &feature in &lakes {
                    set_feature_bit(&mut biome_features[id.0 as usize], feature);
                }
                steps.push(lakes);
            }
            plan.push((id, steps));
        }
        let plan = FeatureDecorationPlan::new(plan).map_err(|e| resource(e.to_string()))?;
        let weather = BiomeWeatherBindings::new(&bindings).ok();
        let states = |names: &[&str]| -> Vec<BlockStateId> {
            names
                .iter()
                .filter_map(|n| registry.default_state(n))
                .collect()
        };
        let postprocess_self = states(&["minecraft:brown_mushroom", "minecraft:red_mushroom"]);
        let postprocess_above = states(&["minecraft:soul_sand", "minecraft:magma_block"]);
        Ok(Some(Self {
            postprocess_self,
            postprocess_above,
            plan,
            names,
            features,
            biome_features,
            weather,
            blocks: FeatureBlocks::from_registry(registry)?,
            bindings,
            catalog,
            named: std::sync::Mutex::new((named, HashMap::new())),
            top_layers: Vec::new(),
        }))
    }
    /// A structure piece's feature, as a placed feature: a configured one is
    /// placed as one with no placement modifiers, which places it once at the
    /// origin, as `ConfiguredFeature.place` does. Cached by its JSON.
    fn piece_feature(&self, feature: FeatureKey<'_>) -> Option<Arc<PlacedFeatureDefinition>> {
        let mut named = self.named.lock().unwrap();
        let (cache, id) = match feature {
            FeatureKey::Placed(id) => (&mut named.0, id),
            FeatureKey::Configured(id) => (&mut named.1, id),
        };
        if let Some(found) = cache.get(id) {
            return found.clone();
        }
        let json = match feature {
            FeatureKey::Placed(id) => Value::from(id).to_string(),
            FeatureKey::Configured(id) => {
                serde_json::json!({ "feature": id, "placement": [] }).to_string()
            }
        };
        let decoded = self
            .catalog
            .placed_feature_with_blocks(&json, &self.bindings)
            .ok()
            .map(Arc::new);
        cache.insert(id.to_owned(), decoded.clone());
        decoded
    }
    /// A structure piece's feature: a feature pool element's placed feature,
    /// or what a mod piece's `place_feature` and `place_placed_feature` ask
    /// for. Whether it placed anything; `Ok(false)` for an id the data pack
    /// lacks.
    pub(super) fn place_piece_feature<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl sunrise_core::random::RandomSource,
        feature: FeatureKey<'_>,
        pos: [i32; 3],
    ) -> Result<bool, PlacementQueryError>
    where
        W::Error: std::fmt::Display,
    {
        match self.piece_feature(feature) {
            Some(feature) => feature
                .place(world, random, pos, self.blocks)
                .map_err(|e| PlacementQueryError::Failed(e.to_string())),
            None => Ok(false),
        }
    }
    /// Vanilla's feature identity and the step-local index/step mixed into
    /// `set_feature_seed`.
    #[cfg(test)]
    pub(super) fn seed(&self, name: &str) -> Option<(u32, i32, i32)> {
        let identity = self.names.iter().position(|candidate| candidate == name)? as u32;
        let (index, step) = self.plan.seed(FeatureIdentity(identity))?;
        Some((identity, index as i32, step as i32))
    }
    /// Placed-feature roots that cannot run, with their decode errors.
    pub(super) fn unsupported(&self) -> Vec<(String, String)> {
        self.names
            .iter()
            .zip(&self.features)
            .filter_map(|(name, feature)| feature.as_ref().err().map(|e| (name.clone(), e.clone())))
            .collect()
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn decorate(
        &self,
        data: &Arc<GeneratorData>,
        seed: i64,
        generation: WorldGenerationContext,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
        level_random: &mut dyn super::structure::legacy::PieceRandom,
        structures: Option<&super::generator_structures::StructureState>,
        dependencies: Option<&dyn DecorationDependencies>,
    ) -> Result<DecorateStats, GeneratorError> {
        let center = region[4].position();
        for (i, chunk) in region.iter().enumerate() {
            let expected = ChunkPos::new(
                center.x.wrapping_add(i as i32 % 3 - 1),
                center.z.wrapping_add(i as i32 / 3 - 1),
            );
            if chunk.position() != expected {
                return Err(resource("decoration region is not a 3×3 neighborhood"));
            }
            if (chunk.terrain.generation_stage() as u8) < PreLightChunkStage::Terrain as u8 {
                return Err(resource("decoration requires terrain-stage chunks"));
            }
        }
        if region[4].is_decorated() {
            return Ok(DecorateStats::default());
        }
        // The full chunks of the region, before their terrain moves into it.
        let full: Vec<ChunkPos> = region
            .iter()
            .filter(|chunk| chunk.is_full())
            .map(|chunk| chunk.position())
            .collect();
        let mut chunks = Vec::with_capacity(9);
        let mut biomes = Vec::with_capacity(9);
        let (min_y, height) = (region[4].terrain.min_y(), region[4].terrain.height());
        for chunk in region.iter_mut() {
            // A one-section stand-in while the region holds the terrain: it
            // is swapped back below, and allocating the full height for nine
            // stand-ins per region cost more than some features.
            let empty = super::terrain::TerrainChunk::new(
                chunk.position(),
                min_y,
                16,
                data.registry(),
                data.block_states().clone(),
            )
            .map_err(|e| resource(e.to_string()))?;
            chunks.push(std::mem::replace(&mut chunk.terrain, empty));
            biomes.push(chunk.biomes().clone());
        }
        let terrain = TerrainRegion::new(chunks, generation)
            .and_then(|region| region.with_block_bindings(&self.bindings))
            .map(|region| region.with_world_seed(seed));
        let terrain = match terrain {
            Ok(terrain) => terrain,
            Err(e) => return Err(resource(e.to_string())),
        };
        let structures_only = worker.structures_only;
        let references = region[4].structure_starts.clone();
        let mut block_entities = Vec::new();
        for chunk in region.iter_mut() {
            block_entities.append(&mut chunk.block_entities);
        }
        let mut world = World {
            block_entities,
            entities: Vec::new(),
            level_random,
            structure_mobs: worker.structure_mobs.clone(),
            biome_keys: worker.mod_biome_keys.current(),
            starts: references.clone(),
            biome_names: data.biome_names(),
            blocks: data
                .feature_resources
                .as_ref()
                .and_then(|r| r.blocks.as_ref()),
            region: terrain,
            biomes: BiomeRegion::new(biomes, obfuscate_seed(seed))
                .map_err(|e| resource(e.to_string()))?,
            zone: GenerationWriteZone::new(center, FEATURES_WRITE_RADIUS),
            center,
            uniform_biomes: vec![UNKNOWN_BIOME; 9 * (height / 16).max(0) as usize],
            biome_quarts: region[4].biomes().quart_range(),
            decoration: self,
            temperatures: &mut worker.temperatures,
            behaviours: data.behaviours.as_deref(),
            // WorldGenRegion.isOutsideBuildHeight: the level's range, not the noise's.
            build: (min_y, min_y + height),
            shape_ticks: std::cell::RefCell::new(crate::level::Ticks::new(0)),
            last_tag: None,
            floor_bounds: [None; 9],
            shape_capture: std::cell::RefCell::new(Vec::new()),
            full,
            dependencies,
        };
        let mut stats = DecorateStats::default();
        let blocks = self.blocks;
        // The blocks' facts: `BlockState.isSolidRender` and the rest, the
        // structure pieces ask them of the blocks around them.
        let tree_blocks = self
            .bindings
            .tree_blocks()
            .map_err(|e| resource(e.to_string()))?;
        // ChunkGenerator.getWritableArea: the chunk's minimum Y + 1 to its maximum Y.
        let chunk_box = BoundingBox::new(
            [center.x * 16, min_y + 1, center.z * 16],
            [center.x * 16 + 15, min_y + height - 1, center.z * 16 + 15],
        );
        let mut structure_errors = 0;
        let result = self.plan.decorate(
            &mut world,
            center,
            seed,
            STEPS.len(),
            |world, random, decoration_seed, step| {
                let Some(structures) = structures else {
                    return Ok(());
                };
                let Some(in_step) = structures.by_step.get(step) else {
                    return Ok(());
                };
                for (index, &structure) in in_step.iter().enumerate() {
                    random.set_feature_seed(decoration_seed, index as i32, step as i32);
                    for start in references.iter().filter(|s| s.structure == structure) {
                        let mut place_feature =
                            |world: &mut World<'_>,
                             random: &mut WorldgenRandom<XoroshiroRandom>,
                             feature: FeatureKey<'_>,
                             pos: [i32; 3]| {
                                self.place_piece_feature(world, random, feature, pos)
                            };
                        if place_in_chunk(
                            start,
                            world,
                            &structures.resources,
                            &tree_blocks,
                            random,
                            chunk_box,
                            &mut place_feature,
                        )
                        .is_err()
                        {
                            structure_errors += 1;
                        }
                    }
                }
                Ok(())
            },
            |world, placer, random, id, origin| {
                if structures_only {
                    return Ok(false);
                }
                let Ok(feature) = &self.features[id.0 as usize] else {
                    stats.unsupported += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                    return Ok(false);
                };
                stats.placed += 1;
                let placed = placer.place_with_biome_check(
                    &feature.placement,
                    world,
                    random,
                    origin,
                    id,
                    |world, random, p| feature.feature.place(world, random, p, blocks),
                );
                if placed.is_err() {
                    stats.failed += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                }
                Ok(placed.unwrap_or(false))
            },
        );
        // `FillLayerFeature.place` for the chunk: air at the layer's Y becomes the layer.
        if result.is_ok() && !structures_only {
            for &(y, state) in &self.top_layers {
                for dz in 0..16 {
                    for dx in 0..16 {
                        let at = [center.x * 16 + dx, y, center.z * 16 + dz];
                        let current = world
                            .block(at)
                            .map_err(|e| GeneratorError::Stage("features", e.to_string()))?;
                        if data.block_states().get(current).is_air() {
                            world
                                .set_block(at, state, 2)
                                .map_err(|e| GeneratorError::Stage("features", e.to_string()))?;
                        }
                    }
                }
            }
        }
        for (pos, nbt) in std::mem::take(&mut world.block_entities) {
            let index = ((pos[2] >> 4).wrapping_sub(center.z) + 1) * 3
                + (pos[0] >> 4).wrapping_sub(center.x)
                + 1;
            if let Some(chunk) = usize::try_from(index).ok().and_then(|i| region.get_mut(i)) {
                chunk.block_entities.push((pos, nbt));
            }
        }
        // WorldGenRegion.addFreshEntity: into the chunk holding the position.
        for nbt in std::mem::take(&mut world.entities) {
            let Some(at) = block_of(&nbt) else {
                continue;
            };
            let index = ((at[2] >> 4).wrapping_sub(center.z) + 1) * 3
                + (at[0] >> 4).wrapping_sub(center.x)
                + 1;
            if let Some(chunk) = usize::try_from(index).ok().and_then(|i| region.get_mut(i)) {
                chunk.entities.push(nbt);
            }
        }
        for terrain in world.region.into_chunks() {
            let p = terrain.position();
            let index = (p.z.wrapping_sub(center.z) + 1) * 3 + p.x.wrapping_sub(center.x) + 1;
            region[index as usize].terrain = terrain;
        }
        result.map_err(|e| GeneratorError::Stage("features", e.to_string()))?;
        stats.failed += structure_errors;
        region[4]
            .terrain
            .set_generation_stage(PreLightChunkStage::Features);
        region[4].set_decorated();
        Ok(stats)
    }
}

/// `a`, and `b` when it differs.
fn span<T: PartialEq>(a: T, b: T) -> (T, Option<T>) {
    let b = (a != b).then_some(b);
    (a, b)
}
const UNKNOWN_BIOME: u32 = u32::MAX;
const MIXED_BIOMES: u32 = u32::MAX - 1;
impl World<'_> {
    /// The biome of a biome section when all its quarts hold the same one
    /// (and its chunk has biomes); `None` when mixed or outside the region.
    fn uniform_section(&mut self, cx: i32, cz: i32, section: usize) -> Option<BiomeId> {
        let (dx, dz) = (
            i64::from(cx) - i64::from(self.center.x) + 1,
            i64::from(cz) - i64::from(self.center.z) + 1,
        );
        if !(0..3).contains(&dx) || !(0..3).contains(&dz) {
            return None;
        }
        let sections = self.uniform_biomes.len() / 9;
        if section >= sections {
            return None;
        }
        let slot = (dz * 3 + dx) as usize * sections + section;
        if self.uniform_biomes[slot] == UNKNOWN_BIOME {
            let chunk = ChunkPos::new(cx, cz);
            let ready = self.region.chunk(cx, cz).is_ok_and(|c| {
                c.highest_generated_status() >= crate::chunk_status::ChunkStatus::Biomes
            });
            let values = self
                .biomes
                .chunk(chunk)
                .ok()
                .and_then(|c| c.section(section));
            self.uniform_biomes[slot] = match values {
                Some(values) if ready && values.iter().all(|b| *b == values[0]) => values[0].0,
                _ => MIXED_BIOMES,
            };
        }
        match self.uniform_biomes[slot] {
            MIXED_BIOMES => None,
            id => Some(BiomeId(id)),
        }
    }
    /// The region chunk slot of chunk `cx`, `cz` (as `uniform_section`).
    fn chunk_slot(&self, cx: i32, cz: i32) -> Option<usize> {
        let (dx, dz) = (
            i64::from(cx) - i64::from(self.center.x) + 1,
            i64::from(cz) - i64::from(self.center.z) + 1,
        );
        ((0..3).contains(&dx) && (0..3).contains(&dz)).then_some((dz * 3 + dx) as usize)
    }
    /// A block write at `[x, y, z]` may raise its chunk's heights up to y + 1.
    fn raise_floor_bound(&mut self, [x, y, z]: [i32; 3]) {
        if let Some(slot) = self.chunk_slot(x >> 4, z >> 4)
            && let Some(bound) = &mut self.floor_bounds[slot]
        {
            *bound = (*bound).max(y.saturating_add(1));
        }
    }
    /// `floor_bounds` for chunk `cx`, `cz`, computed from its heightmap on
    /// first use; `None` outside the region or without the heightmap.
    fn floor_bound(&mut self, cx: i32, cz: i32) -> Option<i32> {
        let slot = self.chunk_slot(cx, cz)?;
        if self.floor_bounds[slot].is_none() {
            let map = self
                .region
                .chunk(cx, cz)
                .ok()?
                .heightmap(HeightmapType::OceanFloorWg)?;
            let mut bound = i32::MIN;
            for x in 0..16 {
                for z in 0..16 {
                    bound = bound.max(map.first_available(x, z).ok()?);
                }
            }
            self.floor_bounds[slot] = Some(bound);
        }
        self.floor_bounds[slot]
    }
    /// BiomeManager.getBiome without the zoom when every quart it can pick
    /// lies in biome sections of one and the same biome.
    fn uniform_biome(&mut self, [x, y, z]: [i32; 3]) -> Option<BiomeId> {
        let base = [x, y, z].map(|v| v.wrapping_sub(2) >> 2);
        let (bottom, top) = self.biome_quarts;
        let section = |q: i32| ((q.clamp(bottom, top) - bottom) >> 2) as usize;
        // The chunks and sections holding the quarts base..=base + 1.
        let (x0, x1) = span(base[0] >> 2, base[0].wrapping_add(1) >> 2);
        let (z0, z1) = span(base[2] >> 2, base[2].wrapping_add(1) >> 2);
        let (s0, s1) = span(section(base[1]), section(base[1].wrapping_add(1)));
        let found = self.uniform_section(x0, z0, s0)?;
        for cx in std::iter::once(x0).chain(x1) {
            for cz in std::iter::once(z0).chain(z1) {
                for s in std::iter::once(s0).chain(s1) {
                    if self.uniform_section(cx, cz, s)? != found {
                        return None;
                    }
                }
            }
        }
        Some(found)
    }
}
/// WorldGenRegion-style view: reads across the FEATURES `STRUCTURE_STARTS`
/// dependency radius (8 chunks, warning outside the 3×3 write zone), writes
/// within the FEATURES write radius (1 chunk), biome lookups through the
/// 3×3 biome zoom.
struct World<'a> {
    full: Vec<ChunkPos>,
    dependencies: Option<&'a dyn DecorationDependencies>,
    block_entities: Vec<([i32; 3], sunrise_core::nbt::Compound)>,
    entities: Vec<sunrise_core::nbt::Compound>,
    level_random: &'a mut dyn super::structure::legacy::PieceRandom,
    structure_mobs: Option<Arc<dyn super::generator::StructureMobs>>,
    /// The world's biome keys when the region opened, for mod features.
    biome_keys: Arc<mod_api::__private::BiomeKeys>,
    starts: Vec<Arc<super::structure::start::StructureStart>>,
    biome_names: &'a [Arc<str>],
    blocks: Option<&'a BlockBehavior>,
    region: TerrainRegion<'static, GenBlockStates>,
    biomes: BiomeRegion<BiomeId>,
    zone: GenerationWriteZone,
    center: ChunkPos,
    /// Per biome section of the nine chunks (chunk-major): its one biome id,
    /// MIXED_BIOMES, or UNKNOWN_BIOME until first asked.
    uniform_biomes: Vec<u32>,
    /// The quart Ys biome lookups clamp to (`BiomeChunk::quart_range`).
    biome_quarts: (i32, i32),
    decoration: &'a Decoration,
    temperatures: &'a mut [BiomeTemperature],
    behaviours: Option<&'a crate::level::Behaviours>,
    /// Inclusive minimum and exclusive maximum build height.
    build: (i32, i32),
    /// The tick capture `shape_update` passes to `updateShape`.
    shape_ticks: std::cell::RefCell<crate::level::Ticks>,
    /// `block_matches_tag`'s last tag and its states.
    last_tag: Option<(Box<str>, Option<Arc<BTreeSet<BlockStateId>>>)>,
    /// Per region chunk (X-major in Z rows, as `uniform_biomes`): a bound no
    /// OCEAN_FLOOR_WG first-available height in it exceeds, once asked.
    /// Heightmaps change only through this world's writes, which raise it.
    floor_bounds: [Option<i32>; 9],
    /// Reusable capture storage for shape updates in this decoration region.
    shape_capture: std::cell::RefCell<Vec<crate::level::ticks::CapturedTick>>,
}
/// Read-only `LevelAccess` view of the decoration region for
/// `BlockState.updateShape` (writes go through `World::set_block`).
struct ShapeLevel<'w, 'a> {
    world: &'w World<'a>,
    random: sunrise_core::random::LegacyRandom,
}
impl crate::level::BlockEntityNeighbors for ShapeLevel<'_, '_> {}
impl crate::level::LevelAccess for ShapeLevel<'_, '_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world
            .read_block([pos.x, pos.y, pos.z])
            .unwrap_or(BlockStateId(0))
    }
    fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.in_dependency_region(pos.chunk())
    }
    fn min_y(&self) -> i32 {
        self.world.build.0
    }
    fn max_y(&self) -> i32 {
        self.world.build.1
    }
    fn random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        0
    }
    /// Lighting has not run during FEATURES: the light engine has no data for
    /// these chunks, so sky light reads zero too (MushroomBlock.canSurvive).
    fn sky_light(&self, _: BlockPos) -> i32 {
        0
    }
}
impl World<'_> {
    /// `BlockState.updateShape` of the state at `pos` against `neighbor`.
    fn shape_update(
        &self,
        behaviours: &crate::level::Behaviours,
        state: BlockStateId,
        pos: [i32; 3],
        direction: Direction,
        neighbor_pos: [i32; 3],
        neighbor: BlockStateId,
    ) -> (BlockStateId, Vec<crate::level::ticks::CapturedTick>) {
        let mut level = ShapeLevel {
            world: self,
            random: sunrise_core::random::LegacyRandom::new(0),
        };
        // The ticks updateShape schedules (LeavesBlock re-measuring its
        // distance, waterlogged blocks' fluid) go to the proto chunks. While
        // capturing, scheduling leaves the rest of `Ticks` untouched, so the
        // region keeps one instead of building one per update.
        let mut ticks = self.shape_ticks.borrow_mut();
        ticks.capture = Some(std::mem::take(&mut *self.shape_capture.borrow_mut()));
        let updated = behaviours.update_shape(
            &mut level,
            &mut ticks,
            state,
            BlockPos::new(pos[0], pos[1], pos[2]),
            direction,
            BlockPos::new(neighbor_pos[0], neighbor_pos[1], neighbor_pos[2]),
            neighbor,
        );
        (updated, ticks.capture.take().unwrap_or_default())
    }
    /// Schedule ticks a shape update requested in the region's proto chunks.
    fn schedule_captured(
        &mut self,
        behaviours: &crate::level::Behaviours,
        mut ticks: Vec<crate::level::ticks::CapturedTick>,
    ) -> Result<(), PlacementQueryError> {
        let result = (|| {
            for t in &ticks {
                if !self.zone.contains_block(t.pos) {
                    continue;
                }
                let at = [t.pos.x, t.pos.y, t.pos.z];
                if t.fluid {
                    if let Some(fluid) = crate::fluid::FluidKind::from_tick_id(t.kind) {
                        self.schedule_fluid_tick(at, fluid, t.delay)?;
                    }
                } else {
                    let state = behaviours
                        .table
                        .default_state(crate::level::BlockId(t.kind as u16));
                    self.schedule_block_tick(at, state, t.delay)?;
                }
            }
            Ok(())
        })();
        ticks.clear();
        *self.shape_capture.borrow_mut() = ticks;
        result
    }
}
impl DecorationWorld for World<'_> {
    type Error = RegionError;
    #[inline]
    fn min_section_y(&mut self) -> Result<i32, RegionError> {
        Ok(self
            .region
            .min_y()
            .map_err(|_| RegionError::InvalidChunks)?
            >> 4)
    }
    /// `BiomeRegion::present_in_chunk`: the distinct ids, ascending, from a
    /// bitset instead of sorting every quart of the column.
    #[inline]
    fn chunk_biomes(&mut self, chunk: ChunkPos) -> Result<Vec<BiomeId>, RegionError> {
        let biomes = self.biomes.chunk(chunk).map_err(RegionError::Biome)?;
        // Ordinary registry ids fit in the stack array. Keep a spill for
        // unusually large modded registries without changing the ascending
        // iteration order of the bitset.
        let mut seen = [0u64; 128];
        let mut seen_spill = Vec::new();
        let mut last = None;
        let mut section = 0;
        while let Some(values) = biomes.section(section) {
            for &id in values {
                if last == Some(id) {
                    continue;
                }
                last = Some(id);
                let word = id.0 as usize / 64;
                if let Some(bits) = seen.get_mut(word) {
                    *bits |= 1 << (id.0 % 64);
                } else {
                    let spill_word = word - seen.len();
                    if seen_spill.len() <= spill_word {
                        seen_spill.resize(spill_word + 1, 0);
                    }
                    seen_spill[spill_word] |= 1 << (id.0 % 64);
                }
            }
            section += 1;
        }
        let count = seen
            .iter()
            .chain(seen_spill.iter())
            .map(|bits| bits.count_ones() as usize)
            .sum();
        let mut out = Vec::with_capacity(count);
        for (word, &bits) in seen.iter().chain(seen_spill.iter()).enumerate() {
            let mut bits = bits;
            while bits != 0 {
                out.push(BiomeId(word as u32 * 64 + bits.trailing_zeros()));
                bits &= bits - 1;
            }
        }
        Ok(out)
    }
    #[inline]
    fn set_currently_generating(&mut self, _: Option<FeatureIdentity>) -> Result<(), RegionError> {
        Ok(())
    }
}
impl World<'_> {
    #[inline]
    fn chunk_distance(&self, chunk: ChunkPos) -> i64 {
        let dx = (i64::from(self.center.x) - i64::from(chunk.x)).abs();
        let dz = (i64::from(self.center.z) - i64::from(chunk.z)).abs();
        dx.max(dz)
    }

    #[inline]
    fn in_dependency_region(&self, chunk: ChunkPos) -> bool {
        self.chunk_distance(chunk) <= i64::from(FEATURES_DEPENDENCY_RADIUS)
    }

    /// `WorldGenRegion.warnIfReadOutsideWriteZone`.
    #[cold]
    fn warn_if_read_outside_write_zone(&self, chunk: ChunkPos) {
        let distance = self.chunk_distance(chunk);
        let stage = PreLightChunkStage::Features.name();
        log::warn!(
            "Detected unsafe terrain read during worldgen: reading from chunk [{}, {}] while generating chunk [{}, {}] (distance: {distance}, write radius: {FEATURES_WRITE_RADIUS}), step: {stage}",
            chunk.x,
            chunk.z,
            self.center.x,
            self.center.z,
        );
    }

    /// Block state of a chunk in the `STRUCTURE_STARTS` dependency ring (distance `2..=8`):
    /// outside build height is `minecraft:void_air`, a resident column supplies its own block,
    /// and a `STRUCTURE_STARTS`-only holder with no resident terrain reads as `minecraft:air`.
    fn outer_block_in_dependency(&self, position: [i32; 3]) -> BlockStateId {
        if position[1] < self.build.0 || position[1] >= self.build.1 {
            return self.region.void_air();
        }
        self.dependencies
            .and_then(|deps| deps.resident_block(position))
            .unwrap_or_else(|| self.region.air())
    }

    #[cold]
    fn read_outer_block(
        &self,
        chunk: ChunkPos,
        position: [i32; 3],
    ) -> Result<BlockStateId, RegionError> {
        self.warn_if_read_outside_write_zone(chunk);
        if !self.in_dependency_region(chunk) {
            return Err(RegionError::MissingChunk([chunk.x, chunk.z]));
        }
        Ok(self.outer_block_in_dependency(position))
    }

    #[inline]
    fn read_block(&self, position: [i32; 3]) -> Result<BlockStateId, RegionError> {
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.block(position);
        }
        self.read_outer_block(chunk, position)
    }

    fn read_fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.fluid_state(position);
        }
        self.warn_if_read_outside_write_zone(chunk);
        if !self.in_dependency_region(chunk) {
            return Err(RegionError::MissingChunk([chunk.x, chunk.z]).into());
        }
        if position[1] < self.build.0 || position[1] >= self.build.1 {
            return Ok(FluidState::EMPTY);
        }
        match self
            .dependencies
            .and_then(|deps| deps.resident_block(position))
        {
            Some(state) => self.decoration.bindings.fluids.state(state).ok_or_else(|| {
                PlacementQueryError::Failed("fluid state outside bound registry".into())
            }),
            None => Ok(FluidState::EMPTY),
        }
    }

    /// `WorldGenRegion.ensureCanWrite`: the step's horizontal radius and, while
    /// upgrading, the center's generation interval. Proto storage handles its
    /// own bounds after admission.
    fn can_write(&self, position: [i32; 3]) -> Result<bool, RegionError> {
        let [x, y, z] = position;
        let pos = BlockPos::new(x, y, z);
        if !self.zone.contains_block(pos) {
            let ChunkPos { x: cx, z: cz } = pos.chunk();
            let stage = PreLightChunkStage::Features.name();
            log::error!(
                "Refused worldgen write at {position:?} in far chunk [{cx}, {cz}], stage: {stage}"
            );
            return Ok(false);
        }
        Ok(self
            .region
            .upgrade_write_height(self.center)?
            .is_none_or(|height| height.contains(y)))
    }

    /// Whether `position` is in one of the region's full chunks.
    fn is_full_chunk(&self, [x, _, z]: [i32; 3]) -> bool {
        self.full.iter().any(|p| p.x == x >> 4 && p.z == z >> 4)
    }
}
impl FeatureWorld for World<'_> {
    type Error = RegionError;
    #[inline]
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, RegionError> {
        self.read_block(position)
    }
    #[inline]
    fn scan_column(
        &mut self,
        position: [i32; 3],
        step: i32,
        limit: i32,
        mut keep: impl FnMut(BlockStateId) -> bool,
    ) -> Result<i32, RegionError> {
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.scan_column(position, step, limit, keep);
        }
        self.warn_if_read_outside_write_zone(chunk);
        if !self.in_dependency_region(chunk) {
            return Err(RegionError::MissingChunk([chunk.x, chunk.z]));
        }
        let mut p = position;
        for _ in 0..limit {
            if !keep(self.outer_block_in_dependency(p)) {
                break;
            }
            p[1] = p[1].wrapping_add(step);
        }
        Ok(p[1])
    }
    #[inline]
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, RegionError> {
        if !self.can_write(position)? {
            return Ok(false);
        }
        // ImposterProtoChunk(chunk, false): setBlockState, setBlockEntity,
        // removeBlockEntity and markPosForPostProcessing change nothing, and
        // WorldGenRegion.setBlock still answers true.
        if self.is_full_chunk(position) {
            return Ok(true);
        }
        let old = self.region.set_block_worldgen(position, state)?;
        self.raise_floor_bound(position);
        // ProtoChunk keeps a block entity that already exists when the same kind
        // of block is set again (a chest turning waterlogged); other changes drop it.
        let registry = self.decoration.bindings.registry();
        let same_block = old == state
            || registry.state(old).map(|s| s.name.as_str())
                == registry.state(state).map(|s| s.name.as_str());
        if !same_block {
            self.block_entities.retain(|(p, _)| *p != position);
        }
        // WorldGenRegion.setBlock: BlockState.getPostProcessPos unless UPDATE_KNOWN_SHAPE.
        if flags & 16 == 0 {
            let [x, y, z] = position;
            let mark = if self.decoration.postprocess_self.contains(&state) {
                Some(y)
            } else if self.decoration.postprocess_above.contains(&state) {
                Some(y.wrapping_add(1))
            } else {
                None
            };
            if let Some(y) = mark {
                self.region
                    .chunk_mut(x >> 4, z >> 4)?
                    .mark_postprocessing(x, y, z);
            }
        }
        Ok(true)
    }
}
fn query(e: impl std::fmt::Display) -> PlacementQueryError {
    PlacementQueryError::Failed(e.to_string())
}
/// The blocks whose block entity is a `RandomizableContainer`: every
/// `RandomizableContainerBlockEntity` subclass (chest, trapped chest, barrel,
/// dispenser, dropper, hopper, shulker box, crafter) and the decorated pot, which
/// implements the interface on a plain `BlockEntity`. `RandomizableContainer`'s
/// `setBlockEntityLootTable` hands a loot table to those and to nothing else.
const RANDOMIZABLE_CONTAINERS: [&str; 25] = [
    "minecraft:chest",
    "minecraft:trapped_chest",
    "minecraft:barrel",
    "minecraft:dispenser",
    "minecraft:dropper",
    "minecraft:hopper",
    "minecraft:shulker_box",
    "minecraft:white_shulker_box",
    "minecraft:orange_shulker_box",
    "minecraft:magenta_shulker_box",
    "minecraft:light_blue_shulker_box",
    "minecraft:yellow_shulker_box",
    "minecraft:lime_shulker_box",
    "minecraft:pink_shulker_box",
    "minecraft:gray_shulker_box",
    "minecraft:light_gray_shulker_box",
    "minecraft:cyan_shulker_box",
    "minecraft:purple_shulker_box",
    "minecraft:blue_shulker_box",
    "minecraft:brown_shulker_box",
    "minecraft:green_shulker_box",
    "minecraft:red_shulker_box",
    "minecraft:black_shulker_box",
    "minecraft:crafter",
    "minecraft:decorated_pot",
];
/// Whether the block entity of a block of this name is a `RandomizableContainer`.
/// Every worldgen caller places a chest today, so the wider set is defensive.
fn randomizable_container(name: &str) -> bool {
    RANDOMIZABLE_CONTAINERS.contains(&name)
}

/// `RandomizableContainer.setBlockEntityLootTable` for a world that keeps block
/// entities as NBT: the compound a randomizable container's block entity takes, or
/// None when the block there is not one. The seed is drawn only where vanilla draws
/// it, inside the `instanceof RandomizableContainer` branch.
fn loot_table_nbt(
    registry: &crate::registry::BlockRegistry,
    state: BlockStateId,
    table: &str,
    random: &mut impl sunrise_core::random::RandomSource,
) -> Option<sunrise_core::nbt::Compound> {
    use sunrise_core::nbt::Tag;
    let name = registry.state(state)?.name.to_string();
    if !randomizable_container(&name) {
        return None;
    }
    let mut nbt = sunrise_core::nbt::Compound::new();
    nbt.insert("id".into(), Tag::String(name.as_str().into()));
    nbt.insert("LootTable".into(), Tag::String(table.into()));
    nbt.insert("LootTableSeed".into(), Tag::Long(random.next_long()));
    Some(nbt)
}
impl<'a> World<'a> {
    fn blocks(&self) -> Result<&'a BlockBehavior, PlacementQueryError> {
        self.blocks.ok_or(PlacementQueryError::Unavailable)
    }
}
/// Proto chunks hold no block entities, so dynamic shapes resolve to absent.
impl DynamicSupportWorld for World<'_> {
    type Error = PlacementQueryError;
    #[inline]
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, PlacementQueryError> {
        Ok(None)
    }
    #[inline]
    fn moving_piston_shape(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<VoxelShape>, PlacementQueryError> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for World<'_> {
    #[inline]
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, PlacementQueryError> {
        Ok(None)
    }
}
impl SurvivalWorld for World<'_> {
    #[inline]
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.read_fluid_state(position)
    }
    /// Unlit generation chunks report no sky or block light.
    #[inline]
    fn raw_brightness(&mut self, _: [i32; 3], _: i32) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    #[inline]
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.read_block(position).map_err(Into::into)
    }
}
impl PlacementContext for World<'_> {
    #[inline]
    fn block_registry(&self) -> Option<&'static crate::registry::BlockRegistry> {
        Some(self.decoration.bindings.registry())
    }
    #[inline]
    fn full_chunk_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(self.is_full_chunk(position))
    }
    #[inline]
    fn state_collision_empty(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        collision
            .collision_shape(self, state, position)
            .map(|shape| shape.is_empty())
            .map_err(query)
    }
    /// RandomizableContainer.setBlockEntityLootTable: only a container block that
    /// is actually there takes the table, and then it draws the loot seed.
    #[inline]
    fn set_block_entity_loot_table(
        &mut self,
        position: [i32; 3],
        table: &str,
        random: &mut impl sunrise_core::random::RandomSource,
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        let Some(nbt) = loot_table_nbt(registry, state, table, random) else {
            return Ok(());
        };
        self.set_block_entity(position, nbt)
    }
    /// BeehiveDecorator: `getBlockEntity(pos, BEEHIVE)` then
    /// `storeBee(Occupant.create(ticks))` for each bee.
    #[inline]
    fn store_beehive_occupants(
        &mut self,
        position: [i32; 3],
        ticks_in_hive: &[i32],
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if !matches!(
            registry.state(state).map(|s| s.name.as_str()),
            Some("minecraft:bee_nest" | "minecraft:beehive")
        ) {
            return Ok(());
        }
        self.set_block_entity(position, super::tree::beehive_occupants(ticks_in_hive))
    }
    /// SpawnerBlockEntity.setEntityId: the next spawn data and no potentials.
    #[inline]
    fn configure_spawner(
        &mut self,
        position: [i32; 3],
        entity: &str,
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if registry.state(state).map(|s| s.name.as_str()) != Some("minecraft:spawner") {
            return Ok(());
        }
        self.set_block_entity(position, super::cave::dungeon_spawner_nbt(entity))
    }
    #[inline]
    fn attachment_at(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_attach_to(self, position, direction)
            .map_err(query)
    }
    #[inline]
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Ok(self.level_random.boolean())
    }
    #[inline]
    fn next_world_float(&mut self) -> Result<f32, PlacementQueryError> {
        Ok(self.level_random.float())
    }
    #[inline]
    fn next_world_int(&mut self, bound: i32) -> Result<i32, PlacementQueryError> {
        Ok(self.level_random.int(bound))
    }
    #[inline]
    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            let mut level = ShapeLevel {
                world: self,
                random: sunrise_core::random::LegacyRandom::new(0),
            };
            let mut ticks = crate::level::Ticks::new(0);
            let pos = BlockPos::new(position[0], position[1], position[2]);
            return Ok(behaviours.can_survive(&mut level, &mut ticks, pos, state));
        }
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_survive(self, state, position)
            .map_err(query)
    }
    #[inline]
    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.read_block(position)?;
        self.state_face_sturdy(state, position, direction)
    }
    #[inline]
    fn state_face_sturdy(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        // Without the exported support tables, fall back to the gameplay
        // behaviour's empty-level sturdiness cache.
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            return Ok(behaviours.is_face_sturdy(state, direction));
        }
        let support = self.blocks()?.support.clone();
        support
            .is_face_sturdy(self, state, position, direction, SupportType::Full)
            .map_err(query)
    }
    #[inline]
    fn center_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.read_block(position)?;
        if let (None, Some(behaviours)) = (self.blocks, self.behaviours) {
            return Ok(behaviours.is_face_sturdy_center(state, direction));
        }
        let support = self.blocks()?.support.clone();
        support
            .is_face_sturdy(self, state, position, direction, SupportType::Center)
            .map_err(query)
    }
    #[inline]
    fn can_occlude(&mut self, state: BlockStateId) -> Result<bool, PlacementQueryError> {
        if let Some(behaviours) = self.behaviours {
            return Ok(behaviours.has_flag(state, crate::level::data::state_flags::CAN_OCCLUDE));
        }
        let properties = &self.blocks()?.properties;
        let name = properties
            .registry()
            .state(state)
            .ok_or(PlacementQueryError::Unavailable)?
            .name
            .clone();
        Ok(properties.definition(&name).is_some_and(|d| d.can_occlude))
    }
    #[inline]
    fn collision_full_block(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        let shape = collision
            .collision_shape(self, state, position)
            .map_err(query)?;
        let [nx, ny, nz] = shape.sizes();
        Ok(!shape.is_empty()
            && [Axis::X, Axis::Y, Axis::Z]
                .into_iter()
                .all(|axis| shape.min(axis) == 0.0 && shape.max(axis) == 1.0)
            && (0..nx).all(|x| {
                (0..ny).all(|y| (0..nz).all(|z| shape.is_full(x as i32, y as i32, z as i32)))
            }))
    }
    #[inline]
    fn add_entity(
        &mut self,
        mut nbt: sunrise_core::nbt::Compound,
    ) -> Result<(), PlacementQueryError> {
        if let Some(mobs) = self.structure_mobs.clone()
            && nbt.contains_key(&"sunrise:finalize".into())
        {
            let at = block_of(&nbt).unwrap_or([0; 3]);
            let biome = self
                .biome_identity(at)
                .ok()
                .and_then(|b| self.biome_names.get(b.0 as usize))
                .map_or("", |n| &**n)
                .to_owned();
            let structures: Vec<String> = self
                .starts
                .iter()
                .filter(|s| s.pieces.iter().any(|p| p.bounding_box().is_inside(at)))
                .map(|s| s.name.clone())
                .collect();
            if mobs.finalize(&mut nbt, &biome, &structures, self.level_random) {
                nbt.remove(&"sunrise:finalize".into());
            }
        }
        self.entities.push(nbt);
        Ok(())
    }
    /// SpikeFeature.placeSpike: the crystal is a fresh entity of the region,
    /// saved with the chunk.
    #[inline]
    fn spawn_end_crystal(
        &mut self,
        position: [f64; 3],
        yaw: f32,
        beam_target: Option<[i32; 3]>,
        invulnerable: bool,
    ) -> Result<(), PlacementQueryError> {
        use sunrise_core::nbt::Tag;
        let mut nbt = sunrise_core::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:end_crystal".into()));
        nbt.insert(
            "Pos".into(),
            Tag::List(position.iter().map(|v| Tag::Double(v.to_bits())).collect()),
        );
        nbt.insert(
            "Rotation".into(),
            Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(0)]),
        );
        nbt.insert("Invulnerable".into(), Tag::Byte(i8::from(invulnerable)));
        nbt.insert("ShowBottom".into(), Tag::Byte(1));
        if let Some(b) = beam_target {
            nbt.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
        }
        self.add_entity(nbt)
    }
    #[inline]
    fn set_block_entity(
        &mut self,
        position: [i32; 3],
        nbt: sunrise_core::nbt::Compound,
    ) -> Result<(), PlacementQueryError> {
        self.block_entities.retain(|(p, _)| *p != position);
        self.block_entities.push((position, nbt));
        Ok(())
    }
    /// WorldGenRegion.getBlockEntity creates the gateway's block entity when the
    /// block is an End gateway; TheEndGatewayBlockEntity.setExitPosition.
    #[inline]
    fn configure_end_gateway(
        &mut self,
        position: [i32; 3],
        exit: [i32; 3],
        exact: bool,
    ) -> Result<(), PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let registry = self.decoration.bindings.registry();
        if registry.state(state).map(|s| s.name.as_str()) != Some("minecraft:end_gateway") {
            return Ok(());
        }
        use sunrise_core::nbt::Tag;
        let mut nbt = sunrise_core::nbt::Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:end_gateway".into()));
        nbt.insert("Age".into(), Tag::Long(0));
        nbt.insert("exit_portal".into(), Tag::IntArray(exit.to_vec()));
        if exact {
            nbt.insert("ExactTeleport".into(), Tag::Byte(1));
        }
        self.set_block_entity(position, nbt)
    }
    #[inline]
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        self.region.world_seed()
    }
    #[inline]
    fn update_shape_at_face(
        &mut self,
        position: [i32; 3],
        direction: Direction,
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let Some(behaviours) = self.behaviours else {
            return Ok(());
        };
        let n = direction.normal();
        let neighbor_pos = [position[0] + n[0], position[1] + n[1], position[2] + n[2]];
        let state = self.read_block(position).map_err(query)?;
        let neighbor = self.read_block(neighbor_pos).map_err(query)?;
        let (new_state, ticks) = self.shape_update(
            behaviours,
            state,
            position,
            direction,
            neighbor_pos,
            neighbor,
        );
        self.schedule_captured(behaviours, ticks)?;
        if new_state != state {
            self.set_block(position, new_state, flags & !1)
                .map_err(query)?;
        }
        let (new_neighbor, ticks) = self.shape_update(
            behaviours,
            neighbor,
            neighbor_pos,
            direction.opposite(),
            position,
            new_state,
        );
        self.schedule_captured(behaviours, ticks)?;
        if new_neighbor != neighbor {
            self.set_block(neighbor_pos, new_neighbor, flags & !1)
                .map_err(query)?;
        }
        Ok(())
    }
    #[inline]
    fn update_from_neighbour_shapes(
        &mut self,
        position: [i32; 3],
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let Some(behaviours) = self.behaviours else {
            return Ok(());
        };
        let state = self.read_block(position).map_err(query)?;
        // Block.UPDATE_SHAPE_ORDER.
        let mut new_state = state;
        for direction in [
            Direction::West,
            Direction::East,
            Direction::North,
            Direction::South,
            Direction::Down,
            Direction::Up,
        ] {
            let n = direction.normal();
            let neighbor_pos = [position[0] + n[0], position[1] + n[1], position[2] + n[2]];
            let neighbor = self.read_block(neighbor_pos).map_err(query)?;
            let (updated, ticks) = self.shape_update(
                behaviours,
                new_state,
                position,
                direction,
                neighbor_pos,
                neighbor,
            );
            self.schedule_captured(behaviours, ticks)?;
            new_state = updated;
        }
        if new_state != state {
            self.set_block(position, new_state, flags).map_err(query)?;
        }
        Ok(())
    }
    #[inline]
    fn ensure_can_write(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.can_write(position).map_err(Into::into)
    }
    /// ProtoChunk.setBlockState: the section and the stage's heightmaps
    /// (nothing on a full chunk: ImposterProtoChunk(chunk, false)).
    #[inline]
    fn set_chunk_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        if self.is_full_chunk(position) {
            return Ok(());
        }
        self.region
            .set_block_worldgen(position, state)
            .map_err(|e| query(format!("{e:?}")))?;
        self.raise_floor_bound(position);
        Ok(())
    }
    /// BulkSectionAccess write: section storage only, no heightmap updates.
    #[inline]
    fn set_section_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_raw(x, y, z, state)
            .map_err(|e| query(format!("{e:?}")))?;
        Ok(())
    }
    /// Generation regions contain no entities.
    #[inline]
    fn unobstructed_block(&mut self, _: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(true)
    }
    #[inline]
    fn destroy_block_without_entity(
        &mut self,
        position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.read_block(position)?;
        if self.region.block_properties_table().get(state).is_air() {
            return Ok(false);
        }
        let air = self.region.air();
        self.set_block(position, air, 3).map_err(query)
    }
    #[inline]
    fn biome_should_freeze(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
        require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_freeze(self, &mut temperature, position, require_edge);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    #[inline]
    fn biome_should_snow(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_snow(self, &mut temperature, position);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    /// Lighting has not run during FEATURES; block light reads zero.
    #[inline]
    fn block_brightness(&mut self, _: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    #[inline]
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.read_fluid_state(position)
    }
    #[inline]
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.sea_level()
    }
    #[inline]
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_fluid_tick(position, fluid, delay)
    }
    #[inline]
    fn mark_postprocessing(&mut self, position: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.region.mark_postprocessing(position)
    }
    #[inline]
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.water_at(position);
        }
        let state = self.read_outer_block(chunk, position)?;
        Ok(self
            .decoration
            .bindings
            .fluids
            .state_matches_tag(state, "minecraft:water"))
    }
    #[inline]
    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_block_tick(position, block, delay)
    }
    #[inline]
    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        if let Some(biome) = self.uniform_biome(position) {
            return Ok(biome);
        }
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if !self.zone.contains_chunk(chunk) && self.in_dependency_region(chunk) {
            // WorldGenRegion.getChunk(cx, cz, BIOMES, false): FEATURES direct dependencies
            // past radius 1 only reach STRUCTURE_STARTS (< BIOMES).
            return Err(RegionError::BiomesNotGenerated([chunk.x, chunk.z]).into());
        }
        self.region
            .biome_at(
                &self.biomes,
                BlockPos::new(position[0], position[1], position[2]),
            )
            .copied()
            .map_err(Into::into)
    }
    #[inline]
    fn biome_key(&mut self, position: [i32; 3]) -> Result<mod_api::BiomeKey, PlacementQueryError> {
        let biome = self.biome_identity(position)?;
        self.biome_names
            .get(biome.0 as usize)
            .and_then(|name| {
                self.biome_keys
                    .view(name)
                    .ok()
                    .map(|view| view.key().clone())
            })
            .ok_or(PlacementQueryError::Unavailable)
    }
    #[inline]
    fn biome_allows_feature(
        &mut self,
        position: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        let biome = self.biome_identity(position)?;
        let bits = &self.decoration.biome_features[biome.0 as usize];
        let word = feature.0 as usize / 64;
        Ok(bits
            .get(word)
            .is_some_and(|w| w >> (feature.0 % 64) & 1 != 0))
    }
    #[inline]
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.read_block(position).map_err(Into::into)
    }
    /// TerrainRegion::block_matches_tag (the region holds the same tag
    /// table), with the last tag's states kept: predicates ask one tag for
    /// many positions (environment scans), and the table is keyed by name.
    #[inline]
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.read_block(position)?;
        self.state_matches_tag(state, tag)
    }
    #[inline]
    fn state_matches_tag(
        &mut self,
        state: BlockStateId,
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        if self
            .last_tag
            .as_ref()
            .is_none_or(|(name, _)| **name != *tag)
        {
            let states = self.decoration.bindings.tags.get(tag).cloned();
            self.last_tag = Some((tag.into(), states));
        }
        let states = self
            .last_tag
            .as_ref()
            .and_then(|(_, states)| states.as_ref());
        Ok(states.is_some_and(|states| states.contains(&state)))
    }
    #[inline]
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    #[inline]
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        let chunk = ChunkPos::new(position[0] >> 4, position[2] >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.layer_block(position);
        }
        let state = self.read_outer_block(chunk, position)?;
        let name = self
            .decoration
            .bindings
            .registry()
            .state(state)
            .map_or("", |s| s.name.as_str());
        Ok(
            if self.region.block_properties_table().get(state).is_air()
                || name == "minecraft:water"
                || name == "minecraft:lava"
            {
                LayerBlock::Empty
            } else if name == "minecraft:bedrock" {
                LayerBlock::Bedrock
            } else {
                LayerBlock::Ground
            },
        )
    }
    #[inline]
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    #[inline]
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    #[inline]
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        let chunk = ChunkPos::new(x >> 4, z >> 4);
        if self.zone.contains_chunk(chunk) {
            return self.region.height(kind, x, z);
        }
        self.warn_if_read_outside_write_zone(chunk);
        if !self.in_dependency_region(chunk) {
            return Err(RegionError::MissingChunk([chunk.x, chunk.z]).into());
        }
        if let Some(y) = self.dependencies.and_then(|deps| {
            deps.resident_height(kind, x, z, self.build, self.region.block_properties_table())
        }) {
            return Ok(y);
        }
        // Empty STRUCTURE_STARTS ProtoChunk: all-air heightmaps have first_available == min_y.
        Ok(self.build.0)
    }
    /// OreFeature's search for a column under its blob, without a trait call
    /// per column. Defers to `height` when a heightmap still has to be primed.
    #[inline]
    fn any_height_at_least(
        &mut self,
        kind: HeightmapType,
        [x0, z0]: [i32; 2],
        size: i32,
        y: i32,
    ) -> Result<Option<bool>, PlacementQueryError> {
        let (x1, z1) = (x0.wrapping_add(size), z0.wrapping_add(size));
        if size < 0 || x1 < x0 || z1 < z0 {
            return Ok(None);
        }
        // Most ore attempts in To The Max's tall range sit above every
        // column: answered from the chunks' bounds without reading columns.
        if kind == HeightmapType::OceanFloorWg {
            let mut below = true;
            for cx in x0 >> 4..=x1 >> 4 {
                for cz in z0 >> 4..=z1 >> 4 {
                    below &= self.floor_bound(cx, cz).is_some_and(|bound| bound < y);
                }
            }
            if below {
                return Ok(Some(false));
            }
        }
        for cx in x0 >> 4..=x1 >> 4 {
            for cz in z0 >> 4..=z1 >> 4 {
                if !self
                    .region
                    .chunk(cx, cz)
                    .is_ok_and(|c| c.heightmap(kind).is_some())
                {
                    return Ok(None);
                }
            }
        }
        // Any column will do, so each chunk's part of the square is read
        // from its heightmap in one go.
        for cx in x0 >> 4..=x1 >> 4 {
            let xs = [x0.max(cx << 4) & 15, x1.min((cx << 4) + 15) & 15];
            for cz in z0 >> 4..=z1 >> 4 {
                let zs = [z0.max(cz << 4) & 15, z1.min((cz << 4) + 15) & 15];
                let map = self.region.chunk(cx, cz)?.heightmap(kind).unwrap();
                if map.any_first_available_at_least(xs, zs, y) {
                    return Ok(Some(true));
                }
            }
        }
        Ok(Some(false))
    }
}

/// The block an entity's `Pos` is in.
fn block_of(nbt: &sunrise_core::nbt::Compound) -> Option<[i32; 3]> {
    let Some(sunrise_core::nbt::Tag::List(p)) = nbt.get(&"Pos".into()) else {
        return None;
    };
    Some(std::array::from_fn(|i| match p.get(i) {
        Some(sunrise_core::nbt::Tag::Double(b)) => f64::from_bits(*b).floor() as i32,
        _ => 0,
    }))
}

/// Marks `feature` in a biome's feature bitset (one bit per feature identity).
fn set_feature_bit(bits: &mut Vec<u64>, feature: FeatureIdentity) {
    let word = feature.0 as usize / 64;
    if bits.len() <= word {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1 << (feature.0 % 64);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunk_status::ChunkStatus,
        registry::BlockRegistry,
        retrogen::BelowZeroRetrogen,
        worldgen::{
            biome_chunk::BiomeChunk, generator::GenBlockState, tags::TagDefinitions,
            terrain::TerrainChunk,
        },
    };
    use sunrise_core::random::{LegacyRandom, RandomSource};

    /// Minimal feature blocks and the containers used in the loot table tests.
    fn registry() -> &'static BlockRegistry {
        let names = [
            "minecraft:air",
            "minecraft:cave_air",
            "minecraft:void_air",
            "minecraft:stone",
            "minecraft:cobblestone",
            "minecraft:end_stone",
            "minecraft:obsidian",
            "minecraft:chest",
            "minecraft:dispenser",
            "minecraft:hopper",
            "minecraft:shulker_box",
            "minecraft:light_gray_shulker_box",
            "minecraft:crafter",
            "minecraft:decorated_pot",
            "minecraft:furnace",
            "minecraft:ender_chest",
        ];
        let rows: Vec<serde_json::Value> = names
            .iter()
            .map(|name| {
                serde_json::json!([name, name.ends_with("air"), false, "",
                    {"id": name, "properties": {}}])
            })
            .collect();
        let defaults: serde_json::Map<String, serde_json::Value> = (names.iter().enumerate())
            .map(|(i, name)| ((*name).to_owned(), serde_json::json!(i)))
            .collect();
        let facts = serde_json::json!({"states": rows, "defaults": defaults});
        let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
        Box::leak(Box::new(registry))
    }

    fn state(name: &str) -> crate::registry::BlockStateId {
        registry().default_state(name).unwrap()
    }

    #[test]
    fn features_writer_uses_resident_center_admission() {
        let registry = registry();
        let blocks = FeatureBlocks::from_registry(registry).unwrap();
        let properties = GenBlockStates::new(
            (0..registry.len())
                .map(|id| {
                    let name = registry
                        .state(BlockStateId(id as u32))
                        .unwrap()
                        .name
                        .as_str();
                    let air = name.ends_with("air");
                    GenBlockState {
                        air,
                        air_block: name == "minecraft:air",
                        motion: !air,
                        no_leaves: !air,
                        fluid: false,
                    }
                })
                .collect(),
        );
        let decoration = Decoration {
            plan: FeatureDecorationPlan::new(vec![]).unwrap(),
            names: Vec::new(),
            features: Vec::new(),
            biome_features: Vec::new(),
            bindings: FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap(),
            weather: None,
            blocks,
            catalog: FeatureCatalog::from_resources(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &TagDefinitions::default(),
            )
            .unwrap(),
            named: Default::default(),
            postprocess_self: Vec::new(),
            postprocess_above: Vec::new(),
            top_layers: Vec::new(),
        };
        let mut chunks = Vec::new();
        let mut biomes = Vec::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let pos = ChunkPos::new(x, z);
                let mut chunk =
                    TerrainChunk::new(pos, -64, 128, registry, properties.clone()).unwrap();
                chunk.set_generation_stage(PreLightChunkStage::Terrain);
                chunks.push(chunk);
                biomes.push(BiomeChunk::new(pos, -64, 128, BiomeId(0)).unwrap());
            }
        }
        let center = ChunkPos::new(0, 0);
        let biome_quarts = biomes[4].quart_range();
        let mut random = LegacyRandom::new(0);
        let biome_keys = Arc::new(mod_api::__private::BiomeKeys::new(0, []));
        let mut world = World {
            full: vec![ChunkPos::new(-1, 0)],
            dependencies: None,
            block_entities: Vec::new(),
            entities: Vec::new(),
            level_random: &mut random,
            structure_mobs: None,
            starts: Vec::new(),
            biome_names: &[],
            biome_keys,
            blocks: None,
            region: TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 128, 0)).unwrap(),
            biomes: BiomeRegion::new(biomes, obfuscate_seed(0)).unwrap(),
            zone: GenerationWriteZone::new(center, FEATURES_WRITE_RADIUS),
            center,
            uniform_biomes: Vec::new(),
            biome_quarts,
            decoration: &decoration,
            temperatures: &mut [],
            behaviours: None,
            build: (-64, 64),
            shape_ticks: std::cell::RefCell::new(crate::level::Ticks::new(0)),
            last_tag: None,
            floor_bounds: [None; 9],
            shape_capture: Default::default(),
        };
        let retrogen = BelowZeroRetrogen::new(ChunkStatus::Features, Vec::new()).unwrap();
        world
            .region
            .chunk_mut(1, 0)
            .unwrap()
            .set_below_zero_retrogen(Some(retrogen.clone()));

        // An upgrading target does not restrict the writes of a normal center.
        let neighbor = [16, 0, 0];
        assert!(world.ensure_can_write(neighbor).unwrap());
        assert!(world.set_block(neighbor, blocks.stone, 2).unwrap());
        assert_eq!(world.block(neighbor).unwrap(), blocks.stone);
        assert!(world.set_block(neighbor, blocks.stone, 2).unwrap());
        assert!(!world.set_block([32, 0, 0], blocks.stone, 2).unwrap());

        // Imposter and out-of-storage ProtoChunk writes can be admitted no-ops.
        let full = [-1, 0, 0];
        assert!(world.set_block(full, blocks.stone, 2).unwrap());
        assert_eq!(world.block(full).unwrap(), blocks.air);
        let above_storage = [16, 64, 0];
        assert!(world.set_block(above_storage, blocks.stone, 2).unwrap());
        assert_eq!(
            world.block(above_storage).unwrap(),
            registry.default_state("minecraft:void_air").unwrap()
        );

        // The center's lower generation interval now applies even to a normal neighbor.
        world
            .region
            .chunk_mut(0, 0)
            .unwrap()
            .set_below_zero_retrogen(Some(retrogen));
        let too_high = [0, 0, -1];
        assert!(!world.ensure_can_write(too_high).unwrap());
        assert!(!world.set_block(too_high, blocks.stone, 2).unwrap());
        assert_eq!(world.block(too_high).unwrap(), blocks.air);
        let in_interval = [0, -1, -1];
        assert!(world.set_block(in_interval, blocks.stone, 2).unwrap());
        assert_eq!(world.block(in_interval).unwrap(), blocks.stone);
    }

    #[test]
    fn features_region_lends_structure_starts_dependencies_out_to_radius_eight() {
        struct OuterStub {
            resident: ChunkPos,
            stone: BlockStateId,
            air: BlockStateId,
        }
        impl DecorationDependencies for OuterStub {
            fn resident_block(&self, [x, y, z]: [i32; 3]) -> Option<BlockStateId> {
                if x >> 4 == self.resident.x && z >> 4 == self.resident.z {
                    Some(if (10..=11).contains(&y) {
                        self.stone
                    } else {
                        self.air
                    })
                } else {
                    None
                }
            }
        }

        let registry = registry();
        let blocks = FeatureBlocks::from_registry(registry).unwrap();
        let void_air = registry.default_state("minecraft:void_air").unwrap();
        let properties = GenBlockStates::new(
            (0..registry.len())
                .map(|id| {
                    let name = registry
                        .state(BlockStateId(id as u32))
                        .unwrap()
                        .name
                        .as_str();
                    let air = name.ends_with("air");
                    GenBlockState {
                        air,
                        air_block: name == "minecraft:air",
                        motion: !air,
                        no_leaves: !air,
                        fluid: false,
                    }
                })
                .collect(),
        );
        let tags = BTreeMap::from([
            ("minecraft:air".to_owned(), vec!["minecraft:air".to_owned()]),
            (
                "minecraft:base_stone_overworld".to_owned(),
                vec!["minecraft:stone".to_owned()],
            ),
        ]);
        let bindings = FeatureBlockBindings::with_tags(registry, BTreeSet::new(), &tags).unwrap();
        let decoration = Decoration {
            plan: FeatureDecorationPlan::new(vec![]).unwrap(),
            names: Vec::new(),
            features: Vec::new(),
            biome_features: Vec::new(),
            bindings,
            weather: None,
            blocks,
            catalog: FeatureCatalog::from_resources(
                &BTreeMap::new(),
                &BTreeMap::new(),
                &TagDefinitions::default(),
            )
            .unwrap(),
            named: Default::default(),
            postprocess_self: Vec::new(),
            postprocess_above: Vec::new(),
            top_layers: Vec::new(),
        };
        let mut chunks = Vec::new();
        let mut biomes = Vec::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let pos = ChunkPos::new(x, z);
                let mut chunk =
                    TerrainChunk::new(pos, -64, 128, registry, properties.clone()).unwrap();
                chunk.set_generation_stage(PreLightChunkStage::Terrain);
                chunks.push(chunk);
                biomes.push(BiomeChunk::new(pos, -64, 128, BiomeId(0)).unwrap());
            }
        }
        let center = ChunkPos::new(0, 0);
        let biome_quarts = biomes[4].quart_range();
        let mut random = LegacyRandom::new(0);
        let outer = OuterStub {
            resident: ChunkPos::new(3, 0),
            stone: blocks.stone,
            air: blocks.air,
        };
        let region = TerrainRegion::new(chunks, WorldGenerationContext::new(-64, 128, 0))
            .unwrap()
            .with_block_bindings(&decoration.bindings)
            .unwrap();
        let biome_keys = Arc::new(mod_api::__private::BiomeKeys::new(0, []));
        let mut world = World {
            full: Vec::new(),
            dependencies: Some(&outer),
            block_entities: Vec::new(),
            entities: Vec::new(),
            level_random: &mut random,
            structure_mobs: None,
            starts: Vec::new(),
            biome_names: &[],
            biome_keys,
            blocks: None,
            region,
            biomes: BiomeRegion::new(biomes, obfuscate_seed(0)).unwrap(),
            zone: GenerationWriteZone::new(center, FEATURES_WRITE_RADIUS),
            center,
            uniform_biomes: Vec::new(),
            biome_quarts,
            decoration: &decoration,
            temperatures: &mut [],
            behaviours: None,
            build: (-64, 64),
            shape_ticks: std::cell::RefCell::new(crate::level::Ticks::new(0)),
            last_tag: None,
            floor_bounds: [None; 9],
            shape_capture: Default::default(),
        };

        // Distance 2 and distance 8 with no resident proto or full column:
        // empty STRUCTURE_STARTS proto chunks (air inside build height, void_air outside,
        // empty fluid, height min_y, biomes unavailable, writes refused).
        for pos in [[32, 0, 0], [128, 0, -128]] {
            assert_eq!(world.block(pos).unwrap(), blocks.air);
            assert_eq!(world.block([pos[0], 64, pos[2]]).unwrap(), void_air);
            assert_eq!(world.block([pos[0], -65, pos[2]]).unwrap(), void_air);
            assert_eq!(
                PlacementContext::fluid_state(&mut world, pos).unwrap(),
                FluidState::EMPTY
            );
            assert_eq!(
                SurvivalWorld::fluid_state(&mut world, pos).unwrap(),
                FluidState::EMPTY
            );
            assert!(!world.water_at(pos).unwrap());
            assert_eq!(world.layer_block(pos).unwrap(), LayerBlock::Empty);
            assert!(world.block_matches_tag(pos, "minecraft:air").unwrap());
            assert!(
                !world
                    .block_matches_tag(pos, "minecraft:base_stone_overworld")
                    .unwrap()
            );
            assert_eq!(
                world
                    .height(HeightmapType::WorldSurfaceWg, pos[0], pos[2])
                    .unwrap(),
                -64
            );
            assert_eq!(
                world.scan_column(pos, 1, 5, |s| s == blocks.air).unwrap(),
                5
            );
            assert!(world.biome_identity(pos).is_err());
            assert!(!world.ensure_can_write(pos).unwrap());
            assert!(!world.set_block(pos, blocks.stone, 2).unwrap());
        }

        // Distance 3 with a resident outer column: reads see its blocks and height,
        // out-of-build-height still reads void_air, and writes remain refused.
        let resident_stone = [48, 10, 0];
        assert_eq!(world.block(resident_stone).unwrap(), blocks.stone);
        assert_eq!(world.block([48, 12, 0]).unwrap(), blocks.air);
        assert_eq!(world.block([48, 64, 0]).unwrap(), void_air);
        assert!(
            world
                .block_matches_tag(resident_stone, "minecraft:base_stone_overworld")
                .unwrap()
        );
        assert_eq!(
            world.layer_block(resident_stone).unwrap(),
            LayerBlock::Ground
        );
        assert_eq!(
            world.height(HeightmapType::WorldSurfaceWg, 48, 0).unwrap(),
            12
        );
        assert!(!world.ensure_can_write(resident_stone).unwrap());
        assert!(
            !world
                .set_block(resident_stone, blocks.cobblestone, 2)
                .unwrap()
        );

        // Past radius 8 (distance 9 and extreme coordinates): out of WorldGenRegion bounds.
        for far in [[144, 0, 0], [0, 0, -144], [i32::MIN, 0, i32::MAX]] {
            assert!(matches!(
                world.block(far),
                Err(RegionError::MissingChunk(_))
            ));
            assert!(
                world
                    .height(HeightmapType::WorldSurfaceWg, far[0], far[2])
                    .is_err()
            );
            assert!(PlacementContext::fluid_state(&mut world, far).is_err());
            assert!(SurvivalWorld::fluid_state(&mut world, far).is_err());
            assert!(!world.ensure_can_write(far).unwrap());
        }
    }

    /// `RandomizableContainer.setBlockEntityLootTable` hands the table to the block
    /// entity of any `RandomizableContainer`, not only the chests the two worldgen
    /// callers place today.
    #[test]
    fn a_randomizable_container_takes_a_loot_table() {
        for name in [
            "minecraft:chest",
            "minecraft:dispenser",
            "minecraft:hopper",
            "minecraft:shulker_box",
            "minecraft:light_gray_shulker_box",
            "minecraft:crafter",
            "minecraft:decorated_pot",
        ] {
            let mut random = LegacyRandom::new(4);
            let nbt = loot_table_nbt(
                registry(),
                state(name),
                "minecraft:chests/simple_dungeon",
                &mut random,
            )
            .unwrap_or_else(|| panic!("no loot table for {name}"));
            assert_eq!(
                nbt.get(&"id".into()),
                Some(&sunrise_core::nbt::Tag::String(name.into()))
            );
            assert_eq!(
                nbt.get(&"LootTable".into()),
                Some(&sunrise_core::nbt::Tag::String(
                    "minecraft:chests/simple_dungeon".into()
                ))
            );
            // The seed is the first long of the random: same seed, same draw.
            assert_eq!(
                nbt.get(&"LootTableSeed".into()),
                Some(&sunrise_core::nbt::Tag::Long(
                    LegacyRandom::new(4).next_long()
                ))
            );
        }
    }

    /// A block whose block entity is not a `RandomizableContainer` keeps no table,
    /// and the seed is not drawn for it.
    #[test]
    fn a_plain_container_takes_no_loot_table() {
        for name in [
            "minecraft:furnace",
            "minecraft:ender_chest",
            "minecraft:air",
        ] {
            let mut random = LegacyRandom::new(4);
            assert!(
                loot_table_nbt(
                    registry(),
                    state(name),
                    "minecraft:chests/simple_dungeon",
                    &mut random
                )
                .is_none(),
                "{name}"
            );
            assert_eq!(
                random.next_long(),
                LegacyRandom::new(4).next_long(),
                "{name}"
            );
        }
    }
}
