//! Engine-side decoding and read-only evaluation of vanilla codec values (`IntProvider`,
//! `BlockStateProvider`, `BlockPredicate`, `HolderSet` for `minecraft:block`) inside mods'
//! custom datapack registries (#5368 part 2).
//!
//! Each load/reload (and each synced client snapshot) builds a fresh [`DatapackVanillaValues`]
//! table with a monotonically increasing non-zero generation number. Handles copied out of
//! `level.datapack_read` carry `(generation, index)`; after `/reload` or a client snapshot swap,
//! any older handle fails with [`mod_api::DatapackValueError::Stale`] rather than panicking.
use super::api_values::TryIntoApi;
use super::{Dim, Game};
use mod_api::{
    __private::VanillaValueDecoder, BlockKey, BlockPredicateValue, BlockState,
    BlockStateProviderValue, DatapackValueError, HolderSetValue, IntProviderValue, LevelRef,
    RandomSource as ModRandomSource,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_client::world::World;
use sunrise_core::dimension::Dimension as LevelDimension;
use sunrise_core::geometry::{Aabb, Direction, Vec3};
use sunrise_core::identifier::Identifier;
use sunrise_core::position::BlockPos;
use sunrise_core::random::{
    InvalidBound, LegacyRandom, PositionalRandom, RandomSource as EngineRandomSource,
};
use sunrise_world::fluid::FluidKind;
use sunrise_world::heightmap::HeightmapType;
use sunrise_world::holder_set::{self, HolderLookup};
use sunrise_world::level::{Behaviours, BlockEntityNeighbors, LevelAccess, Ticks};
use sunrise_world::registry::{BlockRegistry, BlockStateId};
use sunrise_world::worldgen::anchor::WorldGenerationContext;
use sunrise_world::worldgen::biome::BiomeId;
use sunrise_world::worldgen::biome_predicate::BiomePredicateBindings;
use sunrise_world::worldgen::block_feature::integer_properties;
use sunrise_world::worldgen::block_predicate::BlockPredicate;
use sunrise_world::worldgen::feature::FeatureWorld;
use sunrise_world::worldgen::placement::{PlacementContext, PlacementQueryError};
use sunrise_world::worldgen::state_provider::{
    BlockStateProvider, StateProviderBindings, StateProviderCatalog,
};
use sunrise_world::worldgen::value_provider::IntProvider;

/// Monotonic non-zero generation counter shared across loads; generation `0` is reserved for
/// unbound handles and never matches any live table.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// One load's decoded vanilla codec values, swapped atomically on `/reload` and client snapshot
/// replacement.
pub(in crate::game) struct DatapackVanillaValues {
    generation: u64,
    int_providers: Vec<IntProvider>,
    block_state_providers: Vec<BlockStateProvider>,
    block_predicates: Vec<BlockPredicate>,
    block_holder_sets: Vec<BTreeSet<BlockStateId>>,
    biome_ids: BTreeMap<String, BiomeId>,
}

impl DatapackVanillaValues {
    fn int_provider(&self, value: IntProviderValue) -> Result<&IntProvider, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.int_providers
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_state_provider(
        &self,
        value: BlockStateProviderValue,
    ) -> Result<&BlockStateProvider, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_state_providers
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_predicate(
        &self,
        value: BlockPredicateValue,
    ) -> Result<&BlockPredicate, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_predicates
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }

    fn block_holder_set(
        &self,
        value: HolderSetValue,
    ) -> Result<&BTreeSet<BlockStateId>, DatapackValueError> {
        if self.generation != value.generation() {
            return Err(DatapackValueError::Stale);
        }
        self.block_holder_sets
            .get(value.index() as usize)
            .ok_or(DatapackValueError::Stale)
    }
}

/// Decodes vanilla codec JSON during `DatapackCandidate::build` or client snapshot
/// re-binding, accumulating live objects into one [`DatapackVanillaValues`] table.
pub(in crate::game) struct VanillaValueTableBuilder<'a> {
    generation: u64,
    bindings: StateProviderBindings<'static>,
    catalog: Option<&'a StateProviderCatalog>,
    block_tags: BTreeMap<String, Vec<String>>,
    biome_ids: BTreeMap<String, BiomeId>,
    int_providers: Vec<IntProvider>,
    block_state_providers: Vec<BlockStateProvider>,
    block_predicates: Vec<BlockPredicate>,
    block_holder_sets: Vec<BTreeSet<BlockStateId>>,
}

impl<'a> VanillaValueTableBuilder<'a> {
    pub(in crate::game) fn new(
        registry: &'static BlockRegistry,
        behaviours: &Behaviours,
        block_tags: &HashMap<String, HashSet<String>>,
        fluid_tags: &HashMap<String, HashSet<String>>,
        biome_names: &[&str],
        biome_tags: &HashMap<String, HashSet<String>>,
        catalog: Option<&'a StateProviderCatalog>,
    ) -> Result<Self, String> {
        let resolved_block_tags = resolve_block_tags(registry, block_tags);
        let resolved_fluid_tags = resolve_fluid_tags(fluid_tags);
        let biome_ids: BTreeMap<String, BiomeId> = biome_names
            .iter()
            .enumerate()
            .map(|(i, name)| ((*name).to_owned(), BiomeId(i as u32)))
            .collect();
        let resolved_biome_tags = resolve_biome_tags(&biome_ids, biome_tags);
        let biomes = Arc::new(
            BiomePredicateBindings::new(&biome_ids, &resolved_biome_tags)
                .map_err(|e| format!("biome predicate bindings: {e}"))?,
        );
        let bindings = StateProviderBindings::new(
            registry,
            integer_properties(registry),
            &resolved_block_tags,
        )
        .map_err(|e| format!("state provider bindings: {e}"))?
        .with_bound_properties(behaviours.bound_physical_properties())
        .with_fluid_tags(&resolved_fluid_tags)
        .map_err(|e| format!("fluid predicate bindings: {e}"))?
        .with_biomes(biomes);
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            generation,
            bindings,
            catalog,
            block_tags: resolved_block_tags,
            biome_ids,
            int_providers: Vec::new(),
            block_state_providers: Vec::new(),
            block_predicates: Vec::new(),
            block_holder_sets: Vec::new(),
        })
    }

    pub(in crate::game) fn finish(self) -> DatapackVanillaValues {
        DatapackVanillaValues {
            generation: self.generation,
            int_providers: self.int_providers,
            block_state_providers: self.block_state_providers,
            block_predicates: self.block_predicates,
            block_holder_sets: self.block_holder_sets,
            biome_ids: self.biome_ids,
        }
    }
}

fn resolve_block_tags(
    registry: &BlockRegistry,
    tags: &HashMap<String, HashSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    registry.default_state(&canonical).map(|_| canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

fn resolve_fluid_tags(tags: &HashMap<String, HashSet<String>>) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    FluidKind::from_name(&canonical).map(|_| canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

fn resolve_biome_tags(
    biomes: &BTreeMap<String, BiomeId>,
    tags: &HashMap<String, HashSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    tags.iter()
        .filter_map(|(tag, members)| {
            let key = Identifier::parse(tag).ok()?.to_string();
            let mut entries: Vec<String> = members
                .iter()
                .filter_map(|member| {
                    let canonical = Identifier::parse(member).ok()?.to_string();
                    biomes.contains_key(&canonical).then_some(canonical)
                })
                .collect();
            entries.sort_unstable();
            entries.dedup();
            Some((key, entries))
        })
        .collect()
}

struct BlockHolderLookup<'a> {
    registry: &'a BlockRegistry,
    tags: &'a BTreeMap<String, Vec<String>>,
}

impl HolderLookup for BlockHolderLookup<'_> {
    type Id = BlockStateId;

    fn what(&self) -> &str {
        "block"
    }

    fn entry(&self, id: &str) -> Result<Vec<BlockStateId>, String> {
        let canonical = Identifier::parse(id)
            .map_err(|_| format!("invalid block identifier {id:?}"))?
            .to_string();
        let state = self
            .registry
            .default_state(&canonical)
            .ok_or_else(|| format!("unknown block {canonical}"))?;
        Ok(vec![state])
    }

    fn tag(&self, tag: &str) -> Result<Vec<BlockStateId>, String> {
        let canonical = Identifier::parse(tag)
            .map_err(|_| format!("invalid block tag {tag:?}"))?
            .to_string();
        let members = self
            .tags
            .get(&canonical)
            .ok_or_else(|| format!("missing block tag #{canonical}"))?;
        members
            .iter()
            .map(|name| {
                self.registry
                    .default_state(name)
                    .ok_or_else(|| format!("unknown block {name} in tag #{canonical}"))
            })
            .collect()
    }

    fn all(&self) -> Result<Vec<BlockStateId>, String> {
        let mut defaults = BTreeSet::new();
        for index in 0..self.registry.len() {
            if let Some(state) = self.registry.state(BlockStateId(index as u32))
                && let Some(default) = self.registry.default_state(&state.name)
            {
                defaults.insert(default);
            }
        }
        Ok(defaults.into_iter().collect())
    }
}

impl VanillaValueDecoder for VanillaValueTableBuilder<'_> {
    fn decode_int_provider(
        &mut self,
        json: &str,
        min: i32,
        max: i32,
    ) -> Result<IntProviderValue, String> {
        let provider = IntProvider::from_json(json).map_err(|e| e.to_string())?;
        // Matches vanilla `IntProviders.validate(minValue, maxValue, value)`.
        let [lo, hi] = provider.bounds();
        if lo < min || hi > max {
            return Err(format!(
                "int provider bounds [{lo}, {hi}] outside [{min}, {max}]"
            ));
        }
        let index = u32::try_from(self.int_providers.len())
            .map_err(|_| "int provider table overflow".to_owned())?;
        self.int_providers.push(provider);
        Ok(IntProviderValue::from_raw(self.generation, index))
    }

    fn decode_block_state_provider(
        &mut self,
        json: &str,
    ) -> Result<BlockStateProviderValue, String> {
        let provider = match self.catalog {
            Some(catalog) => catalog.provider(json, &self.bindings),
            None => BlockStateProvider::from_json(json, &self.bindings),
        }
        .map_err(|e| e.to_string())?;
        let index = u32::try_from(self.block_state_providers.len())
            .map_err(|_| "block state provider table overflow".to_owned())?;
        self.block_state_providers.push(provider);
        Ok(BlockStateProviderValue::from_raw(self.generation, index))
    }

    fn decode_block_predicate(&mut self, json: &str) -> Result<BlockPredicateValue, String> {
        let predicate = BlockPredicate::from_json_with_blocks(json, &self.bindings)
            .map_err(|e| e.to_string())?;
        let index = u32::try_from(self.block_predicates.len())
            .map_err(|_| "block predicate table overflow".to_owned())?;
        self.block_predicates.push(predicate);
        Ok(BlockPredicateValue::from_raw(self.generation, index))
    }

    fn decode_holder_set(&mut self, registry: &str, json: &str) -> Result<HolderSetValue, String> {
        if registry != "minecraft:block" {
            return Err(format!("unsupported holder set registry {registry}"));
        }
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("invalid holder set json: {e}"))?;
        let lookup = BlockHolderLookup {
            registry: self.bindings.registry(),
            tags: &self.block_tags,
        };
        let states: BTreeSet<BlockStateId> =
            holder_set::resolve(&value, &lookup)?.into_iter().collect();
        let index = u32::try_from(self.block_holder_sets.len())
            .map_err(|_| "holder set table overflow".to_owned())?;
        self.block_holder_sets.push(states);
        Ok(HolderSetValue::from_raw(self.generation, index))
    }
}

/// Adapts a mod's `&mut dyn mod_api::RandomSource` to `sunrise_core::random::RandomSource`,
/// validating bounds so a custom mod RNG returning out-of-range values never panics engine
/// indexing.
struct ModRandom<'a>(&'a mut dyn ModRandomSource);

/// Unreachable positional factory satisfying `EngineRandomSource::Positional`; neither
/// `IntProvider::sample` nor `BlockStateProvider::get_state` forks a positional random.
struct ModPositional<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> PositionalRandom for ModPositional<'a> {
    type Source = ModRandom<'a>;
    fn at(&self, _: BlockPos) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_seed(&self, _: i64) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_hash_of(&self, _: &str) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
    fn from_hash_of_utf16(&self, _: &[u16]) -> Self::Source {
        unreachable!("IntProvider and BlockStateProvider never fork a positional RandomSource")
    }
}

impl<'a> EngineRandomSource for ModRandom<'a> {
    type Positional = ModPositional<'a>;

    fn set_seed(&mut self, _seed: i64) {}

    fn next_int(&mut self) -> i32 {
        self.0.next_int()
    }

    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if bound <= 0 {
            return Err(InvalidBound);
        }
        let value = self.0.next_int_bound(bound);
        if !(0..bound).contains(&value) {
            return Err(InvalidBound);
        }
        Ok(value)
    }

    fn next_long(&mut self) -> i64 {
        self.0.next_long()
    }

    fn next_boolean(&mut self) -> bool {
        self.0.next_boolean()
    }

    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }

    fn next_double(&mut self) -> f64 {
        self.0.next_double()
    }

    fn next_gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }

    fn fork_positional(&mut self) -> Self::Positional {
        ModPositional(std::marker::PhantomData)
    }

    fn fork(&mut self) -> Self {
        unreachable!("IntProvider and BlockStateProvider never fork a RandomSource")
    }
}

/// Read-only `LevelAccess` over one loaded `World` column, used only by `BlockPredicate::WouldSurvive`
/// (`Behaviours::can_survive`). Never writes.
struct ReadOnlyLevel<'a> {
    world: &'a World,
    facts: &'a sunrise_client::blocks::BlockFacts,
    biomes: &'a sunrise_client::biome::Biomes,
    dimension: LevelDimension,
    sea_level: i32,
    /// Only `LevelAccess::random` asks for it. `WouldSurvivePredicate.test` calls
    /// `BlockState.canSurvive(LevelReader, BlockPos)`, which takes no random and draws none, and
    /// `can_survive` never reads this stream, so its seed is not observable.
    random: LegacyRandom,
}

impl BlockEntityNeighbors for ReadOnlyLevel<'_> {}

impl LevelAccess for ReadOnlyLevel<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }

    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }

    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn game_time(&self) -> i64 {
        self.world.game_time
    }

    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }

    fn max_y(&self) -> i32 {
        self.world.shape.max_y()
    }

    fn sea_level(&self) -> i32 {
        self.sea_level
    }

    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }

    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 0x0f)
    }

    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }

    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }

    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
}

/// Read-only `FeatureWorld + PlacementContext` view over a live level (server active/parked or
/// client level) for evaluating `BlockPredicate::test` and `BlockStateProvider::get_state`.
struct DatapackReadWorld<'a, 'g> {
    game: &'a Game<'g>,
    level: LevelRef,
    world: &'a World,
    behaviours: &'a Behaviours,
    registry: &'static BlockRegistry,
    facts: &'a sunrise_client::blocks::BlockFacts,
    biomes: &'a sunrise_client::biome::Biomes,
    biome_ids: &'a BTreeMap<String, BiomeId>,
    block_tags: &'a HashMap<String, HashSet<String>>,
    dimension: LevelDimension,
    sea_level: i32,
}

impl FeatureWorld for DatapackReadWorld<'_, '_> {
    type Error = PlacementQueryError;

    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.world.block(position[0], position[1], position[2]))
    }

    fn set_block(
        &mut self,
        _position: [i32; 3],
        _state: BlockStateId,
        _flags: i32,
    ) -> Result<bool, Self::Error> {
        Err(PlacementQueryError::Unavailable)
    }
}

impl PlacementContext for DatapackReadWorld<'_, '_> {
    fn block_registry(&self) -> Option<&'static BlockRegistry> {
        Some(self.registry)
    }

    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self.world.block(position[0], position[1], position[2]))
    }

    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let mut level = ReadOnlyLevel {
            world: self.world,
            facts: self.facts,
            biomes: self.biomes,
            dimension: self.dimension,
            sea_level: self.sea_level,
            random: LegacyRandom::new(0),
        };
        let mut ticks = Ticks::default();
        Ok(self.behaviours.can_survive(
            &mut level,
            &mut ticks,
            BlockPos::new(position[0], position[1], position[2]),
            state,
        ))
    }

    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.world.block(position[0], position[1], position[2]);
        Ok(self.behaviours.is_face_sturdy(state, direction))
    }

    fn unobstructed_block(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.game
            .is_datapack_block_unobstructed(self.level, self.world, position)
    }

    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        let chunk = self
            .world
            .chunks
            .get(&(position[0] >> 4, position[2] >> 4))
            .ok_or(PlacementQueryError::Unavailable)?;
        let name = &self
            .biomes
            .get(chunk.biome(position[0], position[1], position[2]))
            .name;
        self.biome_ids
            .get(name)
            .copied()
            .ok_or(PlacementQueryError::Unavailable)
    }

    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.world.block(position[0], position[1], position[2]);
        let Some(info) = self.registry.state(state) else {
            return Ok(false);
        };
        let key = tag.trim_start_matches('#');
        let canonical = Identifier::parse(key)
            .map(|id| id.to_string())
            .unwrap_or_else(|_| key.to_owned());
        Ok(self
            .block_tags
            .get(&canonical)
            .is_some_and(|members| members.contains(&info.name)))
    }

    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.world.shape.max_y() - 1)
    }

    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea_level)
    }

    fn generation(&self) -> WorldGenerationContext {
        let min_y = self.world.shape.min_y;
        WorldGenerationContext::new(min_y, self.world.shape.max_y() - min_y, self.sea_level)
    }

    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.world.shape.min_y)
    }

    fn height(
        &mut self,
        _kind: HeightmapType,
        _x: i32,
        _z: i32,
    ) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}

fn valid_resource_segment(s: &str, allow_slash: bool) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("//")
        && s.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && s.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'_' | b'-' | b'.')
                || (allow_slash && b == b'/')
        })
}

fn read_builtin_state_provider(
    resources: &std::path::Path,
    name: &str,
    bundle: &mut Option<Option<serde_json::Map<String, serde_json::Value>>>,
) -> Option<String> {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    if !valid_resource_segment(ns, false) || !valid_resource_segment(path, true) {
        return None;
    }
    let file = resources
        .join("data")
        .join(ns)
        .join("worldgen/block_state_provider")
        .join(format!("{path}.json"));
    if let Ok(text) = std::fs::read_to_string(&file) {
        return Some(text);
    }
    if ns == "minecraft" {
        let map = bundle
            .get_or_insert_with(|| {
                let bytes = std::fs::read(resources.join("worldgen/state_providers.json")).ok()?;
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
            })
            .as_ref()?;
        if let Some(value) = map.get(name).or_else(|| map.get(path)) {
            return Some(
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    None
}

impl<'g> Game<'g> {
    /// Active decoded vanilla-value table for this side (server or client).
    fn active_datapack_vanilla_values(&self) -> Option<&DatapackVanillaValues> {
        if self.is_logical_server() {
            self.mods.datapack_vanilla_values.as_deref()
        } else {
            self.remote
                .as_ref()
                .and_then(|remote| remote.datapack_vanilla_values.as_deref())
        }
    }

    /// Re-decode every vanilla value in `received` against the client's current registries and
    /// tags, returning the bound set and its live value table without mutating `self.remote`.
    fn bind_client_datapack_set(
        &self,
        received: &mod_api::DatapackRegistrySet,
    ) -> sunrise_client::Result<(
        Arc<mod_api::DatapackRegistrySet>,
        Option<Arc<DatapackVanillaValues>>,
    )> {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if !mod_api::__private::declarations_use_vanilla_values(declarations) {
            return Ok((Arc::new(received.clone()), None));
        }
        let (block_tags, fluid_tags, biome_tags) = self.client_datapack_tags();
        let catalog = self.client_state_provider_catalog()?;
        let biomes = self
            .world_biomes
            .as_deref()
            .unwrap_or_else(|| self.res.biomes());
        let biome_names = biomes.registry_order();
        let mut decoder = VanillaValueTableBuilder::new(
            self.pack.assets.registry,
            self.res.behaviours(),
            &block_tags,
            &fluid_tags,
            &biome_names,
            &biome_tags,
            catalog.as_ref(),
        )
        .map_err(|e| format!("client datapack vanilla values: {e}"))?;
        let bound = mod_api::__private::rebind_datapack_vanilla_values(
            received,
            declarations,
            &mut decoder,
        )
        .map_err(|error| {
            format!("The server's mod registries contain an invalid vanilla value: {error:?}")
        })?;
        Ok((Arc::new(bound), Some(Arc::new(decoder.finish()))))
    }

    /// Atomically bind `set` and swap both `remote.datapacks` and `remote.datapack_vanilla_values`
    /// only on success.
    pub(in crate::game) fn install_client_datapack_set(
        &mut self,
        set: &mod_api::DatapackRegistrySet,
    ) -> sunrise_client::Result<()> {
        let (bound, values) = self.bind_client_datapack_set(set)?;
        if let Some(remote) = self.remote.as_mut() {
            remote.datapacks.replace_received(bound);
            remote.datapack_vanilla_values = values;
        }
        Ok(())
    }

    /// Atomically install a configuration-phase `datapacks` receiver: if it carries a set, bind
    /// its vanilla values before swapping `remote.datapacks` and `remote.datapack_vanilla_values`.
    pub(in crate::game) fn install_client_datapack_receiver(
        &mut self,
        mut datapacks: sunrise_client::datapack_net::Receiver,
    ) -> sunrise_client::Result<()> {
        let bound = match datapacks.received() {
            Some(received) => Some(self.bind_client_datapack_set(received)?),
            None => None,
        };
        if let Some((set, values)) = bound {
            datapacks.replace_received(set);
            if let Some(remote) = self.remote.as_mut() {
                remote.datapacks = datapacks;
                remote.datapack_vanilla_values = values;
            }
        } else if let Some(remote) = self.remote.as_mut() {
            remote.datapacks = datapacks;
            remote.datapack_vanilla_values = None;
        }
        Ok(())
    }

    /// Build a `StateProviderCatalog` from the remote server's synced
    /// `minecraft:worldgen/block_state_provider` registry (converting `Some(tag)` via `tag_to_json`
    /// and falling back to the client's built-in `worldgen/block_state_provider` JSON when `data`
    /// is `None` from the known-pack optimization). Never falls back to a local world's
    /// `live_features()`.
    fn client_state_provider_catalog(
        &self,
    ) -> sunrise_client::Result<Option<StateProviderCatalog>> {
        let Some(entries) = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.registries
                    .registries
                    .get("minecraft:worldgen/block_state_provider")
            })
            .filter(|entries| !entries.is_empty())
        else {
            return Ok(None);
        };
        let mut resources = BTreeMap::new();
        let mut builtin_bundle: Option<Option<serde_json::Map<String, serde_json::Value>>> = None;
        for (name, data) in entries {
            if let Some(tag) = data {
                resources.insert(
                    name.clone(),
                    sunrise_client::scoreboard::tag_to_json(tag).to_string(),
                );
                continue;
            }
            if let Some(json) =
                read_builtin_state_provider(&self.options.resources, name, &mut builtin_bundle)
            {
                resources.insert(name.clone(), json);
            }
        }
        if resources.is_empty() {
            return Ok(None);
        }
        StateProviderCatalog::from_resources(&resources)
            .map(Some)
            .map_err(|error| {
                format!("client worldgen/block_state_provider catalog: {error}").into()
            })
    }

    fn client_datapack_tags(
        &self,
    ) -> (
        HashMap<String, HashSet<String>>,
        HashMap<String, HashSet<String>>,
        HashMap<String, HashSet<String>>,
    ) {
        let fallback_blocks = &self.items().block_tags;
        let fallback_fluids = &self.items().fluid_tags;
        let Some(remote) = self.remote.as_ref() else {
            return (
                fallback_blocks.clone(),
                fallback_fluids.clone(),
                HashMap::new(),
            );
        };
        let names = sunrise_protocol::registry::SyncedNames {
            builtin: &remote.builtin,
            dynamic: &remote.registries,
        };
        let decode_tags = |registry: &str| -> Option<HashMap<String, HashSet<String>>> {
            let tags = remote.registries.tags.get(registry)?;
            Some(
                tags.iter()
                    .map(|(tag, ids)| {
                        let members = ids
                            .iter()
                            .filter_map(|&id| {
                                sunrise_protocol::item::Names::name(&names, registry, id)
                            })
                            .collect();
                        (tag.clone(), members)
                    })
                    .collect(),
            )
        };
        let block_tags = decode_tags("minecraft:block")
            .or_else(|| remote.tags.as_ref().map(|t| t.block_tags.clone()))
            .unwrap_or_else(|| fallback_blocks.clone());
        let fluid_tags = decode_tags("minecraft:fluid")
            .or_else(|| remote.tags.as_ref().map(|t| t.fluid_tags.clone()))
            .unwrap_or_else(|| fallback_fluids.clone());
        let biome_tags = decode_tags("minecraft:worldgen/biome").unwrap_or_default();
        (block_tags, fluid_tags, biome_tags)
    }

    fn datapack_read_world<'a>(
        &'a self,
        level: LevelRef,
        table: &'a DatapackVanillaValues,
    ) -> Result<DatapackReadWorld<'a, 'g>, DatapackValueError> {
        let world = self
            .native_level_read_world(level)
            .ok_or(DatapackValueError::Unavailable)?;
        let dimension = Dim::from_id(level.dimension_id()).level();
        let sea_level = Dim::sea_level_of_id(level.dimension_id());
        let biomes = self
            .world_biomes
            .as_deref()
            .unwrap_or_else(|| self.res.biomes());
        let block_tags = if self.is_logical_server() {
            &self.items().block_tags
        } else {
            self.remote
                .as_ref()
                .and_then(|r| r.tags.as_ref())
                .map_or(&self.items().block_tags, |tags| &tags.block_tags)
        };
        Ok(DatapackReadWorld {
            game: self,
            level,
            world,
            behaviours: self.res.behaviours(),
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            biomes,
            biome_ids: &table.biome_ids,
            block_tags,
            dimension,
            sea_level,
        })
    }

    /// `EntityGetter.isUnobstructed(null, Shapes.block().move(pos))`: checks every live
    /// non-spectator player (including LAN guests and dedicated-server players) and every
    /// `blocksBuilding` entity in `level` (active or parked). Mobs come from the level's entity
    /// lookup by the cell's box, as `EntityGetter.getEntities(source, shape.bounds())` does,
    /// not from a walk over every mob.
    fn is_datapack_block_unobstructed(
        &self,
        level: LevelRef,
        world: &World,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let cell = Aabb::new(
            Vec3::new(
                f64::from(position[0]),
                f64::from(position[1]),
                f64::from(position[2]),
            ),
            Vec3::new(
                f64::from(position[0]) + 1.0,
                f64::from(position[1]) + 1.0,
                f64::from(position[2]) + 1.0,
            ),
        );
        let mut cell_shape: Option<sunrise_core::geometry::VoxelShape> = None;
        let mut obstructs = |blocker: Aabb| {
            if !blocker.intersects(cell) {
                return false;
            }
            let shape = cell_shape.get_or_insert_with(|| {
                sunrise_core::geometry::VoxelShape::block().moved(Vec3::new(
                    f64::from(position[0]),
                    f64::from(position[1]),
                    f64::from(position[2]),
                ))
            });
            sunrise_client::mobs::spawn_selection::blocks_spawn_shape(shape, cell, blocker)
        };

        let dim_id = level.dimension_id();
        let dim = Dim::from_id(dim_id);
        let is_active = std::ptr::eq(world, &self.world);

        if is_active
            && !self.lan_acting_fake
            && !self.spectator()
            && self.player.as_ref().is_some_and(|p| obstructs(p.bounds()))
        {
            return Ok(false);
        }
        if self.is_logical_server() {
            if self.lan_acting.is_some() {
                if self.lan_building_boxes.iter(dim).any(&mut obstructs) {
                    return Ok(false);
                }
            } else if let Some(host) = self.lan.as_ref()
                && host
                    .hub
                    .guests
                    .simulation_iter()
                    .filter(|g| g.dimension == dim_id && g.loaded && !g.dead && !g.spectator())
                    .any(|g| obstructs(g.bounds()))
            {
                return Ok(false);
            }
        }

        if is_active {
            let remote = (!self.is_logical_server())
                .then_some(self.remote.as_ref())
                .flatten();
            let mut mob_obstructs = false;
            let _ = self
                .level_visit_entities_in(
                    level,
                    cell.min_position().components(),
                    cell.max_position().components(),
                    &mut |entity| {
                        let Some(index) = self.mob_index.find_ref(&self.mobs, entity.id()) else {
                            return std::ops::ControlFlow::Continue(());
                        };
                        let mob = &self.mobs[index];
                        if !mob.removed
                            && !(&*mob.kind.id == "minecraft:player"
                                && remote.is_some_and(|r| {
                                    r.remote_player_info_game_mode(mob.id as i32) == Some(3)
                                }))
                            && obstructs(mob.bounds())
                        {
                            mob_obstructs = true;
                            return std::ops::ControlFlow::Break(());
                        }
                        std::ops::ControlFlow::Continue(())
                    },
                )
                .map_err(|_| PlacementQueryError::Unavailable)?;
            if mob_obstructs
                || self
                    .dragon
                    .as_ref()
                    .is_some_and(|dragon| obstructs(dragon.body_bounds()))
            {
                return Ok(false);
            }
            let entities = crate::game::entity::entity_lists_of!(self);
            return Ok(!entities
                .non_mob_building_blocker_boxes(
                    self.carts
                        .iter()
                        .map(crate::game::minecarts::Minecart::bounds),
                    &self.crystals,
                )
                .any(obstructs));
        }

        if self.is_logical_server()
            && let Some(parked) = self.levels.parked_of(dim_id)
        {
            return Ok(!parked.any_building_blocker(obstructs));
        }

        Ok(true)
    }

    pub(super) fn datapack_int_provider_sample(
        &self,
        value: IntProviderValue,
        random: &mut dyn ModRandomSource,
    ) -> Result<i32, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let provider = table.int_provider(value)?;
        provider
            .sample(&mut ModRandom(random))
            .map_err(|_| DatapackValueError::EvaluationFailed)
    }

    pub(super) fn datapack_block_state_provider_state(
        &self,
        value: BlockStateProviderValue,
        level: LevelRef,
        random: &mut dyn ModRandomSource,
        pos: [i32; 3],
    ) -> Result<BlockState, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let provider = table.block_state_provider(value)?;
        let mut view = self.datapack_read_world(level, table)?;
        let state = provider
            .get_state(&mut view, &mut ModRandom(random), pos)
            .map_err(|err| match err {
                sunrise_world::worldgen::state_provider::StateProviderError::Query(_)
                | sunrise_world::worldgen::state_provider::StateProviderError::World(_) => {
                    DatapackValueError::Unavailable
                }
                _ => DatapackValueError::EvaluationFailed,
            })?;
        state
            .try_into_api()
            .map_err(|_| DatapackValueError::EvaluationFailed)
    }

    pub(super) fn datapack_block_predicate_test(
        &self,
        value: BlockPredicateValue,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Result<bool, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let predicate = table.block_predicate(value)?;
        let mut view = self.datapack_read_world(level, table)?;
        predicate
            .test(&mut view, pos)
            .map_err(|_| DatapackValueError::Unavailable)
    }

    pub(super) fn datapack_holder_set_contains(
        &self,
        value: HolderSetValue,
        block: BlockKey,
    ) -> Result<bool, DatapackValueError> {
        let table = self
            .active_datapack_vanilla_values()
            .ok_or(DatapackValueError::Stale)?;
        let set = table.block_holder_set(value)?;
        let default = self
            .block_state_default(block)
            .ok_or(DatapackValueError::UnknownBlock)?;
        Ok(set.contains(&default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_client_session::tests::logged_client;
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use crate::game::remote::RemoteDimensionEntry;
    use crate::game::world_loading::WorldRequest;
    use mod_api::{
        DatapackField, DatapackFieldKind, DatapackRegistrySync, DatapackValue, DatapackValueError,
        DatapackVariant, Hooks, LegacyRandomSource, ModEntry, RandomSource, Registrar,
        RegistrationContext,
    };
    use sunrise_client::datapack_net;
    use sunrise_world::worldgen::block_predicate::BlockPredicate;
    use sunrise_world::worldgen::state_provider::BlockStateProvider;
    use sunrise_world::worldgen::value_provider::IntProvider;

    fn request(root: &std::path::Path) -> WorldRequest {
        let mut request = WorldRequest::new(
            "test",
            Some("test"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = root.to_owned();
        request
    }

    struct VanillaValuesMod;
    #[mod_api::__subscriptions]
    impl Hooks for VanillaValuesMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.declare_datapack_registry(
                "bone_meal_rule",
                DatapackRegistrySync::Required,
                vec![
                    DatapackField::required(
                        "blocks_to_grow",
                        DatapackFieldKind::IntProvider { min: 0, max: 128 },
                    ),
                    DatapackField::required(
                        "increase",
                        DatapackFieldKind::IntProvider { min: 0, max: 16 },
                    ),
                    DatapackField::required("vegetation", DatapackFieldKind::BlockStateProvider),
                    DatapackField::required("ground_state", DatapackFieldKind::BlockStateProvider),
                    DatapackField::required("predicate", DatapackFieldKind::BlockPredicate),
                    DatapackField::required(
                        "spread_sources",
                        DatapackFieldKind::HolderSet {
                            registry: "minecraft:block".into(),
                        },
                    ),
                    DatapackField::required(
                        "tag_targets",
                        DatapackFieldKind::HolderSet {
                            registry: "minecraft:block".into(),
                        },
                    ),
                    DatapackField::required(
                        "single_target",
                        DatapackFieldKind::HolderSet {
                            registry: "minecraft:block".into(),
                        },
                    ),
                    DatapackField::required(
                        "variant",
                        DatapackFieldKind::Dispatch {
                            key: "type".into(),
                            variants: vec![DatapackVariant::new(
                                "ubm_test:random_tick",
                                vec![DatapackField::with_default(
                                    "growth_attempts",
                                    DatapackFieldKind::IntProvider { min: 1, max: 64 },
                                    DatapackValue::int_provider("1"),
                                )],
                            )],
                        },
                    ),
                ],
            )
            .unwrap();
        }
    }

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    /// A hostile mod RNG that returns an out-of-range value from `next_int_bound`: must return
    /// `EvaluationFailed` rather than panicking inside engine indexing.
    struct HostileRng;
    impl RandomSource for HostileRng {
        fn next_int(&mut self) -> i32 {
            0
        }
        fn next_int_bound(&mut self, bound: i32) -> i32 {
            bound
        }
        fn next_long(&mut self) -> i64 {
            0
        }
        fn next_boolean(&mut self) -> bool {
            false
        }
        fn next_float(&mut self) -> f32 {
            0.0
        }
        fn next_double(&mut self) -> f64 {
            0.0
        }
        fn next_gaussian(&mut self) -> f64 {
            0.0
        }
    }

    #[test]
    fn datapack_vanilla_values_table_rejects_stale_and_hostile_rng() {
        let table = DatapackVanillaValues {
            generation: 2,
            int_providers: vec![
                IntProvider::from_json(
                    r#"{"type":"minecraft:uniform","min_inclusive":1,"max_inclusive":4}"#,
                )
                .unwrap(),
            ],
            block_state_providers: Vec::new(),
            block_predicates: vec![
                BlockPredicate::from_json(r#"{"type":"minecraft:true"}"#).unwrap(),
            ],
            block_holder_sets: vec![BTreeSet::from([BlockStateId(7)])],
            biome_ids: BTreeMap::new(),
        };
        // Earlier generation and out-of-bounds index both return Stale.
        assert_eq!(
            table.int_provider(IntProviderValue::from_raw(1, 0)).err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table.int_provider(IntProviderValue::from_raw(2, 1)).err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table
                .block_state_provider(BlockStateProviderValue::from_raw(1, 0))
                .err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table
                .block_predicate(BlockPredicateValue::from_raw(1, 0))
                .err(),
            Some(DatapackValueError::Stale)
        );
        assert_eq!(
            table.block_holder_set(HolderSetValue::from_raw(1, 0)).err(),
            Some(DatapackValueError::Stale)
        );
        // Live generation succeeds, and a hostile RNG returns Err without panicking.
        let provider = table
            .int_provider(IntProviderValue::from_raw(2, 0))
            .unwrap();
        let mut hostile = HostileRng;
        assert!(provider.sample(&mut ModRandom(&mut hostile)).is_err());
        let mut ok_rng = LegacyRandomSource::new(42);
        let value = provider.sample(&mut ModRandom(&mut ok_rng)).unwrap();
        assert!((1..=4).contains(&value));
    }

    #[test]
    #[ignore = "daily"]
    fn datapack_vanilla_values_agree_with_worldgen_and_invalidate_on_reload() {
        if run_fresh_cases(
            "game::native_datapack_values::tests::datapack_vanilla_values_agree_with_worldgen_and_invalidate_on_reload",
            "SUNRISE_DATAPACK_VANILLA_VALUES_CHILD",
            &["all"],
        ) {
            return;
        }
        let _publication = datapack_net::isolate_server_publication();
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let pack = root.join("datapacks/ubm_fixture");
        write(
            &pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"ubm vanilla values fixture","min_format":94,"max_format":94}}"#,
        );
        let tag_file = pack.join("data/ubm_test/tags/block/bonemeal_targets.json");
        write(
            &tag_file,
            r#"{"values":["minecraft:dirt","minecraft:sand"]}"#,
        );
        let uniform_json = r#"{"type":"minecraft:uniform","min_inclusive":1,"max_inclusive":2}"#;
        let weighted_int_json = r#"{"type":"minecraft:weighted_list","distribution":[{"data":0,"weight":1},{"data":1,"weight":3}]}"#;
        let cactus_provider_json = r#"{"type":"minecraft:weighted","entries":[{"data":"minecraft:cactus","weight":9},{"data":"minecraft:cactus_flower","weight":1}]}"#;
        let podzol_provider_json = r#"{"type":"minecraft:weighted","entries":[{"data":{"id":"minecraft:sweet_berry_bush","properties":{"age":"3"}},"weight":2},{"data":"minecraft:fern","weight":120}]}"#;
        let air_above_json =
            r#"{"type":"minecraft:matching_block_tag","offset":[0,1,0],"tag":"minecraft:air"}"#;
        let rule_file = pack.join("data/ubm_test/ubm_test/bone_meal_rule/cactus.json");
        let rule_v1 = format!(
            r##"{{
                "blocks_to_grow": {uniform_json},
                "increase": {weighted_int_json},
                "vegetation": {cactus_provider_json},
                "ground_state": {podzol_provider_json},
                "predicate": {air_above_json},
                "spread_sources": ["minecraft:grass_block", "minecraft:mycelium"],
                "tag_targets": "#ubm_test:bonemeal_targets",
                "single_target": "minecraft:fern",
                "variant": {{"type": "ubm_test:random_tick"}}
            }}"##
        );
        write(&rule_file, &rule_v1);
        write(
            &root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/ubm_fixture"],"disabled":[]}"#,
        );

        mod_platform::testing::with_mods(
            vec![ModEntry::new("ubm_test", Box::new(VanillaValuesMod))].into(),
            || {
                with_game(|game| {
                    game.load_world(request(root)).unwrap();
                    let level = game.level(game.dim.id()).unwrap();
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let air = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:air")
                        .unwrap();
                    if !game.world.chunks.contains_key(&(0, 0)) {
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                    }
                    game.world.set(0, 64, 0, stone).unwrap();
                    game.world.set(0, 65, 0, air).unwrap();
                    game.world.set(0, 66, 0, stone).unwrap();

                    let mut blocks_to_grow = None;
                    let mut increase = None;
                    let mut vegetation = None;
                    let mut ground_state = None;
                    let mut predicate = None;
                    let mut spread_sources = None;
                    let mut tag_targets = None;
                    let mut single_target = None;
                    let mut growth_attempts = None;
                    level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:cactus",
                            &mut |entry| {
                                blocks_to_grow = entry
                                    .get("blocks_to_grow")
                                    .and_then(DatapackValue::as_int_provider);
                                increase = entry
                                    .get("increase")
                                    .and_then(DatapackValue::as_int_provider);
                                vegetation = entry
                                    .get("vegetation")
                                    .and_then(DatapackValue::as_block_state_provider);
                                ground_state = entry
                                    .get("ground_state")
                                    .and_then(DatapackValue::as_block_state_provider);
                                predicate = entry
                                    .get("predicate")
                                    .and_then(DatapackValue::as_block_predicate);
                                spread_sources = entry
                                    .get("spread_sources")
                                    .and_then(DatapackValue::as_holder_set);
                                tag_targets = entry
                                    .get("tag_targets")
                                    .and_then(DatapackValue::as_holder_set);
                                single_target = entry
                                    .get("single_target")
                                    .and_then(DatapackValue::as_holder_set);
                                growth_attempts = entry
                                    .get("variant")
                                    .and_then(|v| v.get("growth_attempts"))
                                    .and_then(DatapackValue::as_int_provider);
                            },
                        )
                        .unwrap();
                    let blocks_to_grow = blocks_to_grow.unwrap();
                    let increase = increase.unwrap();
                    let vegetation = vegetation.unwrap();
                    let ground_state = ground_state.unwrap();
                    let predicate = predicate.unwrap();
                    let spread_sources = spread_sources.unwrap();
                    let tag_targets = tag_targets.unwrap();
                    let single_target = single_target.unwrap();
                    let growth_attempts = growth_attempts.unwrap();

                    // 1. IntProvider agrees with sunrise-world's evaluator on the same JSON and seed.
                    for (handle, json) in [
                        (blocks_to_grow, uniform_json),
                        (increase, weighted_int_json),
                        (growth_attempts, "1"),
                    ] {
                        let direct = IntProvider::from_json(json).unwrap();
                        let mut mod_rng = LegacyRandomSource::new(0x5368_1234);
                        let mut engine_rng = LegacyRandom::new(0x5368_1234);
                        for _ in 0..32 {
                            assert_eq!(
                                handle.sample(game, &mut mod_rng).unwrap(),
                                direct.sample(&mut engine_rng).unwrap()
                            );
                        }
                    }
                    // Hostile RNG returning an out-of-range draw fails cleanly without panicking.
                    assert_eq!(
                        blocks_to_grow.sample(game, &mut HostileRng),
                        Err(DatapackValueError::EvaluationFailed)
                    );

                    // 2. BlockStateProvider agrees with sunrise-world's evaluator on the same JSON and seed.
                    let resolved_tags =
                        resolve_block_tags(game.pack.assets.registry, &game.items().block_tags);
                    let bindings = StateProviderBindings::new(
                        game.pack.assets.registry,
                        integer_properties(game.pack.assets.registry),
                        &resolved_tags,
                    )
                    .unwrap()
                    .with_bound_properties(game.res.behaviours().bound_physical_properties());
                    let table = game.active_datapack_vanilla_values().unwrap();
                    for (handle, json) in [
                        (vegetation, cactus_provider_json),
                        (ground_state, podzol_provider_json),
                    ] {
                        let direct = BlockStateProvider::from_json(json, &bindings).unwrap();
                        let mut mod_rng = LegacyRandomSource::new(0x5368_5678);
                        let mut engine_rng = LegacyRandom::new(0x5368_5678);
                        for _ in 0..16 {
                            let got = handle.state(game, level, &mut mod_rng, [0, 65, 0]).unwrap();
                            let mut view = game.datapack_read_world(level, table).unwrap();
                            let expected = direct
                                .get_state(&mut view, &mut engine_rng, [0, 65, 0])
                                .unwrap()
                                .try_into_api()
                                .unwrap();
                            assert_eq!(got, expected);
                        }
                    }

                    // 3. BlockPredicate agrees with sunrise-world's evaluator on live world positions.
                    let direct_pred =
                        BlockPredicate::from_json_with_blocks(air_above_json, &bindings).unwrap();
                    for (pos, expect_true) in [([0, 64, 0], true), ([0, 65, 0], false)] {
                        let got = predicate.test(game, level, pos).unwrap();
                        let mut view = game.datapack_read_world(level, table).unwrap();
                        let expected = direct_pred.test(&mut view, pos).unwrap();
                        assert_eq!(got, expected);
                        assert_eq!(got, expect_true);
                    }

                    // 4. HolderSet membership for list, custom tag, and single block.
                    let key = |game: &Game<'_>, id: &str| {
                        <dyn mod_api::Host>::block_key(game, id).unwrap()
                    };
                    assert_eq!(
                        spread_sources.contains(game, key(game, "minecraft:grass_block")),
                        Ok(true)
                    );
                    assert_eq!(
                        spread_sources.contains(game, key(game, "minecraft:mycelium")),
                        Ok(true)
                    );
                    assert_eq!(
                        spread_sources.contains(game, key(game, "minecraft:stone")),
                        Ok(false)
                    );
                    assert_eq!(
                        tag_targets.contains(game, key(game, "minecraft:dirt")),
                        Ok(true)
                    );
                    assert_eq!(
                        tag_targets.contains(game, key(game, "minecraft:sand")),
                        Ok(true)
                    );
                    assert_eq!(
                        tag_targets.contains(game, key(game, "minecraft:gravel")),
                        Ok(false)
                    );
                    assert_eq!(
                        single_target.contains(game, key(game, "minecraft:fern")),
                        Ok(true)
                    );
                    assert_eq!(
                        single_target.contains(game, key(game, "minecraft:stone")),
                        Ok(false)
                    );

                    // 5. Invalid vanilla value on reload refuses with the field path and keeps gen 1 live.
                    write(
                        &rule_file,
                        r##"{
                            "blocks_to_grow": {"type":"minecraft:uniform","min_inclusive":0,"max_inclusive":999},
                            "increase": 1,
                            "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                            "ground_state": {"id":"minecraft:fern"},
                            "predicate": {"type":"minecraft:true"},
                            "spread_sources": "minecraft:dirt",
                            "tag_targets": "#ubm_test:bonemeal_targets",
                            "single_target": "minecraft:fern",
                            "variant": {"type":"ubm_test:random_tick"}
                        }"##,
                    );
                    let features = game.level.enabled_features.clone();
                    let err = game
                        .reload_datapacks(true, &features)
                        .err()
                        .expect("out-of-bounds int provider must refuse reload")
                        .to_string();
                    assert!(err.contains("blocks_to_grow"), "{err}");
                    assert_eq!(
                        tag_targets.contains(game, key(game, "minecraft:dirt")),
                        Ok(true)
                    );

                    // 6. Successful /reload swaps generation: old handles error as Stale, new handles read new data.
                    write(&tag_file, r#"{"values":["minecraft:gravel"]}"#);
                    write(
                        &rule_file,
                        r##"{
                            "blocks_to_grow": 5,
                            "increase": 2,
                            "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                            "ground_state": {"id":"minecraft:fern"},
                            "predicate": {"type":"minecraft:solid"},
                            "spread_sources": "minecraft:stone",
                            "tag_targets": "#ubm_test:bonemeal_targets",
                            "single_target": "minecraft:cactus",
                            "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                        }"##,
                    );
                    assert!(game.run_command("reload"));
                    let mut rng = LegacyRandomSource::new(1);
                    assert_eq!(
                        blocks_to_grow.sample(game, &mut rng),
                        Err(DatapackValueError::Stale)
                    );
                    assert_eq!(
                        vegetation.state(game, level, &mut rng, [0, 64, 0]),
                        Err(DatapackValueError::Stale)
                    );
                    assert_eq!(
                        predicate.test(game, level, [0, 64, 0]),
                        Err(DatapackValueError::Stale)
                    );
                    assert_eq!(
                        tag_targets.contains(game, key(game, "minecraft:gravel")),
                        Err(DatapackValueError::Stale)
                    );

                    let mut new_int = None;
                    let mut new_state = None;
                    let mut new_pred = None;
                    let mut new_tag = None;
                    level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:cactus",
                            &mut |entry| {
                                new_int = entry
                                    .get("blocks_to_grow")
                                    .and_then(DatapackValue::as_int_provider);
                                new_state = entry
                                    .get("vegetation")
                                    .and_then(DatapackValue::as_block_state_provider);
                                new_pred = entry
                                    .get("predicate")
                                    .and_then(DatapackValue::as_block_predicate);
                                new_tag = entry
                                    .get("tag_targets")
                                    .and_then(DatapackValue::as_holder_set);
                            },
                        )
                        .unwrap();
                    assert_eq!(new_int.unwrap().sample(game, &mut rng), Ok(5));
                    let sugar_cane = new_state
                        .unwrap()
                        .state(game, level, &mut rng, [0, 64, 0])
                        .unwrap();
                    assert_eq!(
                        <dyn mod_api::Host>::block_state_string(game, sugar_cane).as_deref(),
                        Some("minecraft:sugar_cane[age=0]")
                    );
                    assert_eq!(new_pred.unwrap().test(game, level, [0, 64, 0]), Ok(true));
                    assert_eq!(new_pred.unwrap().test(game, level, [0, 65, 0]), Ok(false));
                    assert_eq!(
                        new_tag
                            .unwrap()
                            .contains(game, key(game, "minecraft:gravel")),
                        Ok(true)
                    );
                    assert_eq!(
                        new_tag.unwrap().contains(game, key(game, "minecraft:dirt")),
                        Ok(false)
                    );

                    // 7. /reload with native mods defers datapack snapshots until after UpdateTags
                    //    (run_native_advancement_grants), so a connected client rebinds a newly
                    //    added block tag referenced by a synced HolderSet cleanly.
                    let offer = datapack_net::offer().unwrap().unwrap();
                    let rx = crate::game::lan::world_spawn_sync_tests::host_for(game);
                    {
                        let host = game.lan.as_mut().unwrap();
                        host.hub.mods_loaded = true;
                        host.hub.guests.update(0, |guest| {
                            guest.brand = "sunrise".into();
                            guest.sender.datapack_offer = Some(offer.clone().into());
                        });
                    }
                    let drain_guest_packets = || {
                        rx.try_iter()
                            .flat_map(|out| match out {
                                sunrise_client::lan::Out::Packet(body) => vec![body],
                                sunrise_client::lan::Out::Packets(bodies) => bodies,
                                sunrise_client::lan::Out::Disconnect(reason) => {
                                    panic!("unexpected guest disconnect: {reason:?}")
                                }
                            })
                            .map(|body| {
                                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                                let id = reader.varint().unwrap();
                                sunrise_protocol::packets::play::Clientbound::decode(
                                    id,
                                    reader.rest(),
                                )
                                .unwrap()
                            })
                            .collect::<Vec<_>>()
                    };
                    let _ = drain_guest_packets();

                    let added_tag_file = pack.join("data/ubm_test/tags/block/reload_added.json");
                    write(&added_tag_file, r#"{"values":["minecraft:clay"]}"#);
                    write(
                        &rule_file,
                        r##"{
                            "blocks_to_grow": 5,
                            "increase": 2,
                            "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                            "ground_state": {"id":"minecraft:fern"},
                            "predicate": {"type":"minecraft:solid"},
                            "spread_sources": "minecraft:stone",
                            "tag_targets": "#ubm_test:reload_added",
                            "single_target": "minecraft:cactus",
                            "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                        }"##,
                    );
                    let resources = game.options.resources.clone();
                    let stack = crate::game::functions::pack_stack(
                        root,
                        &resources,
                        &game.pack.data_source,
                        &game.pack.data_changes,
                        false,
                        &[],
                        &[],
                    );
                    let prepared = game.prepare_world_reload(&stack, &resources).unwrap();
                    game.install_world_tables(prepared);
                    assert!(
                        game.lan.as_ref().unwrap().hub.native_reload_pending,
                        "native mods must defer finish_reload_resources"
                    );
                    let before_finish = drain_guest_packets();
                    assert!(
                        !before_finish.iter().any(|packet| matches!(
                            packet,
                            sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                        )),
                        "UpdateTags must wait for finish_reload_resources"
                    );
                    assert!(
                        !before_finish.iter().any(|packet| matches!(
                            packet,
                            sunrise_protocol::packets::play::Clientbound::CustomPayload(p)
                                if p.channel == datapack_net::CHANNEL
                        )),
                        "datapack snapshot must not precede UpdateTags during deferred reload"
                    );

                    game.run_native_advancement_grants();
                    let reload_packets = drain_guest_packets();
                    let tags_idx = reload_packets
                        .iter()
                        .position(|packet| {
                            matches!(
                                packet,
                                sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                            )
                        })
                        .expect("UpdateTags must be sent on finish_reload_resources");
                    let snapshot_idx = reload_packets
                        .iter()
                        .position(|packet| {
                            matches!(
                                packet,
                                sunrise_protocol::packets::play::Clientbound::CustomPayload(p)
                                    if p.channel == datapack_net::CHANNEL
                            )
                        })
                        .expect("datapack snapshot must be sent after finish_reload_resources");
                    assert!(
                        tags_idx < snapshot_idx,
                        "UpdateTags ({tags_idx}) must arrive before datapack snapshot ({snapshot_idx})"
                    );
                    // 7b. UnobstructedPredicate checks non-mob blocksBuilding entities, LAN/dedicated
                    //     players, and parked dimensions lazily.
                    let unobstructed_file =
                        pack.join("data/ubm_test/ubm_test/bone_meal_rule/unobstructed.json");
                    write(
                        &unobstructed_file,
                        r##"{
                            "blocks_to_grow": 1,
                            "increase": 1,
                            "vegetation": {"id":"minecraft:stone"},
                            "ground_state": {"id":"minecraft:stone"},
                            "predicate": {"type":"minecraft:unobstructed"},
                            "spread_sources": "minecraft:stone",
                            "tag_targets": "#ubm_test:reload_added",
                            "single_target": "minecraft:cactus",
                            "variant": {"type":"ubm_test:random_tick"}
                        }"##,
                    );
                    assert!(game.run_command("reload"));
                    let mut unobstructed = None;
                    level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:unobstructed",
                            &mut |entry| {
                                unobstructed = entry
                                    .get("predicate")
                                    .and_then(DatapackValue::as_block_predicate);
                            },
                        )
                        .unwrap();
                    let unobstructed = unobstructed.unwrap();
                    assert_eq!(unobstructed.test(game, level, [10, 65, 10]), Ok(true));
                    // Non-marker armor stand blocks building; marker armor stand does not.
                    game.armor_stands
                        .push(sunrise_client::entities::ArmorStand::new(
                            9001,
                            Vec3::new(10.5, 65.0, 10.5),
                            0.0,
                        ));
                    assert_eq!(unobstructed.test(game, level, [10, 65, 10]), Ok(false));
                    let mut marker = sunrise_client::entities::ArmorStand::new(
                        9002,
                        Vec3::new(12.5, 65.0, 12.5),
                        0.0,
                    );
                    marker.marker = true;
                    game.armor_stands.push(marker);
                    assert_eq!(unobstructed.test(game, level, [12, 65, 12]), Ok(true));
                    // LAN guest blocks building even when self.player is None (dedicated server).
                    let saved_player = game.player.take();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.dimension = game.dim.id().to_owned();
                        guest.loaded = true;
                        guest.dead = false;
                        guest.position = [14.5, 65.0, 14.5];
                        guest.game_mode = 0;
                    });
                    assert_eq!(unobstructed.test(game, level, [14, 65, 14]), Ok(false));
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.game_mode = 3;
                    });
                    assert_eq!(unobstructed.test(game, level, [14, 65, 14]), Ok(true));
                    game.player = saved_player;
                    // Parked server dimension entities are checked lazily too.
                    let back = game.dim;
                    game.enter_level(Dim::Nether).unwrap();
                    game.armor_stands
                        .push(sunrise_client::entities::ArmorStand::new(
                            9003,
                            Vec3::new(16.5, 65.0, 16.5),
                            0.0,
                        ));
                    game.enter_level(back).unwrap();
                    let parked_nether = LevelRef::new(level.epoch(), Dim::Nether.key());
                    assert_eq!(
                        unobstructed.test(game, parked_nether, [16, 65, 16]),
                        Ok(false)
                    );
                    assert_eq!(
                        unobstructed.test(game, parked_nether, [18, 65, 18]),
                        Ok(true)
                    );

                    // Clean up LAN host before switching this fixture Game to client mode.
                    game.lan = None;

                    // 8. Client sync round-trip: replaying the reload's wire packets in order
                    //    installs the newly added block tag before binding the snapshot.
                    let _server = crate::game::remote::tests::connect_menu_test(game);
                    game.drain_test_remote_join();
                    {
                        let remote = game.remote.as_mut().unwrap();
                        remote.logged_in = true;
                        remote.entity_id = 7;
                        remote.profile = 42;
                    }
                    game.enter_remote_dimension(
                        &sunrise_protocol::packets::play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:overworld".into(),
                            seed: 0,
                            game_type: 1,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        },
                        RemoteDimensionEntry::Login,
                        sunrise_client::level_loading::Reason::Other,
                    );
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                    game.world.set(0, 64, 0, stone).unwrap();
                    game.world.set(0, 65, 0, air).unwrap();
                    for packet in reload_packets {
                        if matches!(
                            packet,
                            sunrise_protocol::packets::play::Clientbound::UpdateTags(_)
                                | sunrise_protocol::packets::play::Clientbound::CustomPayload(_)
                        ) {
                            game.apply_packet(packet).unwrap();
                        }
                    }
                    let client_level = game.level("minecraft:overworld").unwrap();
                    let mut c_int = None;
                    let mut c_state = None;
                    let mut c_pred = None;
                    let mut c_tag = None;
                    client_level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:cactus",
                            &mut |entry| {
                                c_int = entry
                                    .get("blocks_to_grow")
                                    .and_then(DatapackValue::as_int_provider);
                                c_state = entry
                                    .get("vegetation")
                                    .and_then(DatapackValue::as_block_state_provider);
                                c_pred = entry
                                    .get("predicate")
                                    .and_then(DatapackValue::as_block_predicate);
                                c_tag = entry
                                    .get("tag_targets")
                                    .and_then(DatapackValue::as_holder_set);
                            },
                        )
                        .unwrap();
                    assert_eq!(c_int.unwrap().sample(game, &mut rng), Ok(5));
                    let c_sugar = c_state
                        .unwrap()
                        .state(game, client_level, &mut rng, [0, 64, 0])
                        .unwrap();
                    assert_eq!(
                        <dyn mod_api::Host>::block_state_string(game, c_sugar).as_deref(),
                        Some("minecraft:sugar_cane[age=0]")
                    );
                    assert_eq!(
                        c_pred.unwrap().test(game, client_level, [0, 64, 0]),
                        Ok(true)
                    );
                    assert_eq!(
                        c_pred.unwrap().test(game, client_level, [0, 65, 0]),
                        Ok(false)
                    );
                    assert_eq!(
                        c_tag.unwrap().contains(game, key(game, "minecraft:clay")),
                        Ok(true)
                    );
                    assert_eq!(
                        c_tag.unwrap().contains(game, key(game, "minecraft:gravel")),
                        Ok(false)
                    );
                    // Re-binding a new client snapshot invalidates the previous client handles.
                    let current_set = game
                        .remote
                        .as_ref()
                        .unwrap()
                        .datapacks
                        .received()
                        .unwrap()
                        .clone();
                    game.install_client_datapack_set(&current_set).unwrap();
                    assert_eq!(
                        c_int.unwrap().sample(game, &mut rng),
                        Err(DatapackValueError::Stale)
                    );

                    // 9. Atomic client binding + clean disconnect on invalid vanilla value.
                    //    Read fresh handles from the current live client generation first.
                    let mut live_int = None;
                    let mut live_tag = None;
                    client_level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:cactus",
                            &mut |entry| {
                                live_int = entry
                                    .get("blocks_to_grow")
                                    .and_then(DatapackValue::as_int_provider);
                                live_tag = entry
                                    .get("tag_targets")
                                    .and_then(DatapackValue::as_holder_set);
                            },
                        )
                        .unwrap();
                    let live_int = live_int.unwrap();
                    let live_tag = live_tag.unwrap();

                    // Build a wire-valid snapshot whose HolderSet references a tag the client lacks.
                    let declarations = mod_api::__private::datapack_registry_declarations();
                    let bad_files = vec![(
                        "data/ubm_test/ubm_test/bone_meal_rule/cactus.json".to_owned(),
                        vec![(
                            "bad".to_owned(),
                            br##"{
                                "blocks_to_grow": 5,
                                "increase": 2,
                                "vegetation": {"id":"minecraft:sugar_cane","properties":{"age":"0"}},
                                "ground_state": {"id":"minecraft:fern"},
                                "predicate": {"type":"minecraft:solid"},
                                "spread_sources": "minecraft:stone",
                                "tag_targets": "#ubm_test:missing_client_tag",
                                "single_target": "minecraft:cactus",
                                "variant": {"type":"ubm_test:random_tick","growth_attempts":4}
                            }"##
                            .to_vec(),
                        )],
                    )];
                    struct AcceptAllVanilla;
                    impl VanillaValueDecoder for AcceptAllVanilla {
                        fn decode_int_provider(
                            &mut self,
                            _: &str,
                            _: i32,
                            _: i32,
                        ) -> Result<IntProviderValue, String> {
                            Ok(IntProviderValue::from_raw(99, 0))
                        }
                        fn decode_block_state_provider(
                            &mut self,
                            _: &str,
                        ) -> Result<BlockStateProviderValue, String> {
                            Ok(BlockStateProviderValue::from_raw(99, 0))
                        }
                        fn decode_block_predicate(
                            &mut self,
                            _: &str,
                        ) -> Result<BlockPredicateValue, String> {
                            Ok(BlockPredicateValue::from_raw(99, 0))
                        }
                        fn decode_holder_set(
                            &mut self,
                            _: &str,
                            _: &str,
                        ) -> Result<HolderSetValue, String> {
                            Ok(HolderSetValue::from_raw(99, 0))
                        }
                    }
                    let bad_candidate = mod_api::DatapackCandidate::build(
                        declarations,
                        &bad_files,
                        &mut AcceptAllVanilla,
                    )
                    .unwrap();
                    let bad_set = mod_api::DatapackRegistrySet::from_candidate(bad_candidate);
                    let offered = mod_api::DatapackSnapshot::decode_offer(&offer).unwrap();
                    let bad_bytes = bad_set
                        .snapshot_for(declarations, &offered)
                        .unwrap()
                        .encode()
                        .unwrap();
                    let bad_chunks: Vec<Vec<u8>> =
                        datapack_net::chunks(&bad_bytes).unwrap().collect();
                    assert_eq!(bad_chunks.len(), 1);
                    let bad_packet = sunrise_protocol::packets::play::Clientbound::CustomPayload(
                        sunrise_protocol::packets::configuration::CustomPayload {
                            channel: datapack_net::CHANNEL.into(),
                            data: bad_chunks[0].clone(),
                        },
                    );
                    // Direct apply_packet fails with PacketError and keeps the previous generation live.
                    let err = game.apply_packet(bad_packet.clone()).expect_err(
                        "invalid vanilla value on Play reload must fail with PacketError",
                    );
                    assert!(
                        err.is::<crate::game::remote::PacketError>(),
                        "expected PacketError, got: {err}"
                    );
                    assert_eq!(live_int.sample(game, &mut rng), Ok(5));
                    assert_eq!(
                        live_tag.contains(game, key(game, "minecraft:clay")),
                        Ok(true)
                    );

                    // Dispatching the bad Play packet through poll_remote_events disconnects cleanly.
                    game.dispatch_test_remote_event(sunrise_client::net::Event::Packet(Box::new(
                        bad_packet,
                    )))
                    .unwrap();
                    assert!(
                        game.remote.is_none(),
                        "invalid Play datapack vanilla value must disconnect the client"
                    );

                    // Event::Joined with an invalid vanilla value also disconnects cleanly.
                    let _server2 = logged_client(game);
                    let mut bad_receiver = datapack_net::Receiver::default();
                    for chunk in &bad_chunks {
                        bad_receiver.accept_chunk(chunk).unwrap();
                    }
                    let joined_registries = game.remote.as_ref().unwrap().registries.clone();
                    game.dispatch_test_remote_event(sunrise_client::net::Event::Joined {
                        registries: joined_registries,
                        datapacks: bad_receiver,
                        name: "tester".into(),
                        uuid: 1,
                    })
                    .unwrap();
                    assert!(
                        game.remote.is_none(),
                        "invalid Joined datapack vanilla value must disconnect the client"
                    );
                    // 10. Client-side named BlockStateProvider holders come from synced
                    //     remote.registries["minecraft:worldgen/block_state_provider"] (both
                    //     inline NBT tags and known-pack None fallback to built-in JSON).
                    let _server3 = logged_client(game);
                    game.enter_remote_dimension(
                        &sunrise_protocol::packets::play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:overworld".into(),
                            seed: 0,
                            game_type: 1,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        },
                        RemoteDimensionEntry::Login,
                        sunrise_client::level_loading::Reason::Other,
                    );
                    let synced_tag = sunrise_client::lan::data::json_to_nbt(&serde_json::json!({
                        "type": "minecraft:weighted",
                        "entries": [
                            { "data": "minecraft:emerald_block", "weight": 1 }
                        ]
                    }));
                    // Pick a real built-in provider name from the prepared resources if present.
                    let builtin_provider_name =
                        std::fs::read(game.options.resources.join("worldgen/state_providers.json"))
                            .ok()
                            .and_then(|bytes| {
                                serde_json::from_slice::<serde_json::Value>(&bytes).ok()
                            })
                            .and_then(|value| {
                                value.as_object().and_then(|map| map.keys().next().cloned())
                            })
                            .map(|key| {
                                if key.contains(':') {
                                    key
                                } else {
                                    format!("minecraft:{key}")
                                }
                            });
                    {
                        let remote = game.remote.as_mut().unwrap();
                        let dyn_reg = Arc::make_mut(&mut remote.registries);
                        let mut provider_entries =
                            vec![("ubm_test:synced_emerald".to_owned(), Some(synced_tag))];
                        if let Some(ref builtin_name) = builtin_provider_name {
                            provider_entries.push((builtin_name.clone(), None));
                        }
                        dyn_reg.registries.insert(
                            "minecraft:worldgen/block_state_provider".into(),
                            provider_entries,
                        );
                    }
                    let ground_json = builtin_provider_name
                        .as_deref()
                        .map(|name| format!(r#""{name}""#))
                        .unwrap_or_else(|| r#"{"id":"minecraft:stone"}"#.to_owned());
                    let named_files = vec![(
                        "data/ubm_test/ubm_test/bone_meal_rule/named.json".to_owned(),
                        vec![(
                            "synced".to_owned(),
                            format!(
                                r##"{{
                                    "blocks_to_grow": 1,
                                    "increase": 1,
                                    "vegetation": "ubm_test:synced_emerald",
                                    "ground_state": {ground_json},
                                    "predicate": {{"type":"minecraft:true"}},
                                    "spread_sources": "minecraft:stone",
                                    "tag_targets": "minecraft:stone",
                                    "single_target": "minecraft:stone",
                                    "variant": {{"type":"ubm_test:random_tick"}}
                                }}"##
                            )
                            .into_bytes(),
                        )],
                    )];
                    let named_candidate = mod_api::DatapackCandidate::build(
                        declarations,
                        &named_files,
                        &mut AcceptAllVanilla,
                    )
                    .unwrap();
                    let named_set = mod_api::DatapackRegistrySet::from_candidate(named_candidate);
                    let named_bytes = named_set
                        .snapshot_for(declarations, &offered)
                        .unwrap()
                        .encode()
                        .unwrap();
                    for chunk in datapack_net::chunks(&named_bytes).unwrap() {
                        game.apply_packet(
                            sunrise_protocol::packets::play::Clientbound::CustomPayload(
                                sunrise_protocol::packets::configuration::CustomPayload {
                                    channel: datapack_net::CHANNEL.into(),
                                    data: chunk,
                                },
                            ),
                        )
                        .unwrap();
                    }
                    let named_level = game.level("minecraft:overworld").unwrap();
                    let mut synced_veg = None;
                    let mut synced_ground = None;
                    named_level
                        .datapack_read(
                            game,
                            "ubm_test:bone_meal_rule",
                            "ubm_test:named",
                            &mut |entry| {
                                synced_veg = entry
                                    .get("vegetation")
                                    .and_then(DatapackValue::as_block_state_provider);
                                synced_ground = entry
                                    .get("ground_state")
                                    .and_then(DatapackValue::as_block_state_provider);
                            },
                        )
                        .unwrap();
                    let emerald = synced_veg
                        .unwrap()
                        .state(game, named_level, &mut rng, [0, 64, 0])
                        .unwrap();
                    assert_eq!(
                        <dyn mod_api::Host>::block_state_string(game, emerald).as_deref(),
                        Some("minecraft:emerald_block")
                    );
                    assert!(
                        synced_ground
                            .unwrap()
                            .state(game, named_level, &mut rng, [0, 64, 0])
                            .is_ok(),
                        "known-pack None built-in provider holder must resolve on client"
                    );
                });
            },
        );
    }
}
