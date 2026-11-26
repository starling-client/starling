//! Structure processor types a mod registers (vanilla's
//! `StructureProcessorType` with its own `StructureProcessor`), handed over
//! once at startup and looked up while a processor list decodes, after the
//! vanilla processor types. A decoded processor is
//! [`Processor::Mod`](super::processor::Processor::Mod): template placement
//! runs [`ModStructureProcessor::process_block`] per block and
//! [`ModStructureProcessor::finalize_processing`] once after the block loop.
//! Runs on worldgen worker threads, so a processor keeps no mutable state and
//! its contexts carry no `Host`.
use super::processor::{PlaceSettings, Processor, ProcessorList, ProcessorRandom};
use super::{BlockInfo, Mirror, Rotation, StructureResources};
use crate::heightmap::HeightmapType;
use crate::registry::BlockStateId;
use crate::worldgen::block_feature::FeatureBlockBindings;
use crate::worldgen::feature::FeatureWorld;
use crate::worldgen::placement::{PlacementContext, PlacementQueryError};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use sunrise_core::bounding_box::BoundingBox;
use sunrise_core::position::BlockPos;
use sunrise_core::random::{LegacyRandom, RandomSource, position_seed};

/// Read-only snapshot of [`PlaceSettings`] handed to a mod processor.
#[derive(Clone, Copy, Debug)]
pub struct PlaceSettingsSnapshot {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: [i32; 3],
    pub bounding_box: Option<BoundingBox>,
    pub waterlogging: bool,
    pub known_shape: bool,
    pub ignore_entities: bool,
    pub finalize_entities: bool,
}
impl PlaceSettingsSnapshot {
    pub(super) fn from_settings(settings: &PlaceSettings<'_>) -> Self {
        Self {
            mirror: settings.mirror,
            rotation: settings.rotation,
            pivot: settings.pivot,
            bounding_box: settings.bounding_box,
            waterlogging: settings.waterlogging,
            known_shape: settings.known_shape,
            ignore_entities: settings.ignore_entities,
            finalize_entities: settings.finalize_entities,
        }
    }
}

/// A structure processor type's engine side: the codec of vanilla's
/// `StructureProcessorType`, as the executable's adapter sees it.
pub trait ModStructureProcessorType: Send + Sync {
    /// Decode one processor from its processor list entry (its
    /// `processor_type` included). An error fails the processor list's decode.
    fn decode(
        &self,
        config: &serde_json::Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Arc<dyn ModStructureProcessor>, String>;
}

/// A decoded structure processor of a mod's type: vanilla's
/// `StructureProcessor`. `Send + Sync` because neighbouring chunks build at
/// once on worker threads; the same random gives the same answers.
pub trait ModStructureProcessor: Send + Sync + std::fmt::Debug + std::any::Any {
    /// `StructureProcessor.processBlock`: inspect or transform one block.
    fn process_block(
        &self,
        context: &mut dyn ModProcessorContext,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError>;

    /// `StructureProcessor.finalizeProcessing`: run once after `process_block`
    /// has visited the piece's blocks.
    fn finalize_processing(
        &self,
        context: &mut dyn ModProcessorFinalizeContext,
        original: &[BlockInfo],
        processed: Vec<BlockInfo>,
    ) -> Result<Vec<BlockInfo>, PlacementQueryError>;

    /// `StructureProcessor.evaluatesEntirePieceState`.
    fn evaluates_entire_piece_state(&self) -> bool {
        false
    }
}

/// Read-only world and placement context available to
/// [`ModStructureProcessor::process_block`].
pub trait ModProcessorContext {
    fn resources(&self) -> &StructureResources;
    fn offset(&self) -> [i32; 3];
    fn pivot(&self) -> [i32; 3];
    fn settings(&self) -> PlaceSettingsSnapshot;
    /// `settings.getRandom(current_pos)`: the settings' shared random when
    /// set, else a position-seeded `LegacyRandom` for the current block (or
    /// piece offset in `finalize_processing`).
    fn random(&mut self) -> &mut dyn mod_api::RandomSource;
    /// `settings.getRandom(pos)`: the settings' shared random when set, else
    /// a fresh `LegacyRandom` seeded from `pos`.
    fn random_at(&mut self, pos: [i32; 3]) -> &mut dyn mod_api::RandomSource;
    fn world_seed(&mut self) -> Option<i64>;
    fn min_y(&mut self) -> Option<i32>;
    fn max_y(&mut self) -> Option<i32>;
    fn sea_level(&mut self) -> Option<i32>;
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool>;
    fn state_in_tag(&mut self, state: BlockStateId, tag: &str) -> Option<bool>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    fn biome(&mut self, pos: [i32; 3]) -> Option<mod_api::BiomeKey>;
    fn is_water(&mut self, pos: [i32; 3]) -> Option<bool>;
    fn can_occlude(&mut self, state: BlockStateId) -> Option<bool>;
    fn apply_processor(
        &mut self,
        processor: &Processor,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError>;
    fn apply_processor_list(
        &mut self,
        list: &ProcessorList,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError>;
}

/// Level-writing extension available to
/// [`ModStructureProcessor::finalize_processing`].
pub trait ModProcessorFinalizeContext: ModProcessorContext {
    /// `LevelWriter.setBlock(pos, state, flags)`.
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool;
}

struct SharedRandom<'a>(&'a mut dyn ProcessorRandom);
impl mod_api::RandomSource for SharedRandom<'_> {
    fn next_int(&mut self) -> i32 {
        self.0.int_unbounded()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        if bound <= 0 { 0 } else { self.0.int(bound) }
    }
    fn next_long(&mut self) -> i64 {
        self.0.long()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.boolean()
    }
    fn next_float(&mut self) -> f32 {
        self.0.float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.double()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.gaussian()
    }
}

pub(super) struct SeededRandom(LegacyRandom);
impl SeededRandom {
    fn at(pos: [i32; 3]) -> Self {
        Self(LegacyRandom::new(position_seed(BlockPos::new(
            pos[0], pos[1], pos[2],
        ))))
    }
}
impl mod_api::RandomSource for SeededRandom {
    fn next_int(&mut self) -> i32 {
        self.0.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            0
        } else {
            self.0.next_int_bound(bound).unwrap_or(0)
        }
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
}

pub(super) struct ProcessorRunContext<'a, 'b, W> {
    world: &'a mut W,
    resources: &'a StructureResources,
    position: [i32; 3],
    reference: [i32; 3],
    settings: &'a mut PlaceSettings<'b>,
    random_pos: [i32; 3],
    shared_random: Option<SharedRandom<'b>>,
    default_random: Option<SeededRandom>,
    at_random: Option<SeededRandom>,
}
impl<'a, 'b, W> ProcessorRunContext<'a, 'b, W> {
    pub(super) fn new(
        world: &'a mut W,
        resources: &'a StructureResources,
        position: [i32; 3],
        reference: [i32; 3],
        settings: &'a mut PlaceSettings<'b>,
        random_pos: [i32; 3],
    ) -> Self {
        let shared_random = settings.random.take().map(SharedRandom);
        Self {
            world,
            resources,
            position,
            reference,
            settings,
            random_pos,
            shared_random,
            default_random: None,
            at_random: None,
        }
    }
}
impl<W> Drop for ProcessorRunContext<'_, '_, W> {
    fn drop(&mut self) {
        if let Some(SharedRandom(random)) = self.shared_random.take() {
            self.settings.random = Some(random);
        }
    }
}
impl<W: PlacementContext> ModProcessorContext for ProcessorRunContext<'_, '_, W> {
    fn resources(&self) -> &StructureResources {
        self.resources
    }
    fn offset(&self) -> [i32; 3] {
        self.position
    }
    fn pivot(&self) -> [i32; 3] {
        self.reference
    }
    fn settings(&self) -> PlaceSettingsSnapshot {
        PlaceSettingsSnapshot::from_settings(self.settings)
    }
    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        if let Some(random) = self.shared_random.as_mut() {
            random
        } else {
            let pos = self.random_pos;
            self.default_random
                .get_or_insert_with(|| SeededRandom::at(pos))
        }
    }
    fn random_at(&mut self, pos: [i32; 3]) -> &mut dyn mod_api::RandomSource {
        if let Some(random) = self.shared_random.as_mut() {
            random
        } else {
            self.at_random.insert(SeededRandom::at(pos))
        }
    }
    fn world_seed(&mut self) -> Option<i64> {
        self.world.world_seed().ok()
    }
    fn min_y(&mut self) -> Option<i32> {
        self.world.min_y().ok()
    }
    fn max_y(&mut self) -> Option<i32> {
        self.world.max_y().ok()
    }
    fn sea_level(&mut self) -> Option<i32> {
        self.world.sea_level().ok()
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockStateId> {
        self.world.block_state(pos).ok()
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        self.world.block_matches_tag(pos, tag).ok()
    }
    fn state_in_tag(&mut self, state: BlockStateId, tag: &str) -> Option<bool> {
        self.world.state_matches_tag(state, tag).ok()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        self.world.height(kind, x, z).ok()
    }
    fn biome(&mut self, pos: [i32; 3]) -> Option<mod_api::BiomeKey> {
        self.world.biome_key(pos).ok()
    }
    fn is_water(&mut self, pos: [i32; 3]) -> Option<bool> {
        self.world.water_at(pos).ok()
    }
    fn can_occlude(&mut self, state: BlockStateId) -> Option<bool> {
        self.world.can_occlude(state).ok()
    }
    fn apply_processor(
        &mut self,
        processor: &Processor,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError> {
        if let Some(SharedRandom(random)) = self.shared_random.take() {
            self.settings.random = Some(random);
        }
        let result = super::processor::process_block(
            processor,
            self.world,
            self.resources,
            self.position,
            self.reference,
            original,
            current,
            self.settings,
        );
        self.shared_random = self.settings.random.take().map(SharedRandom);
        result
    }
    fn apply_processor_list(
        &mut self,
        list: &ProcessorList,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError> {
        if let Some(SharedRandom(random)) = self.shared_random.take() {
            self.settings.random = Some(random);
        }
        let result = super::processor::process_list(
            list,
            self.world,
            self.resources,
            self.position,
            self.reference,
            original,
            current,
            self.settings,
        );
        self.shared_random = self.settings.random.take().map(SharedRandom);
        result
    }
}
impl<W: FeatureWorld + PlacementContext> ModProcessorFinalizeContext
    for ProcessorRunContext<'_, '_, W>
{
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        self.world.set_block(pos, state, flags).unwrap_or(false)
    }
}

/// The installed types, `modid:weathering` → type. Set once, before any world
/// opens (the executable hands the mods' registrations over), and read while
/// processor lists decode.
pub type ModStructureProcessorTypes = BTreeMap<String, Arc<dyn ModStructureProcessorType>>;

static MOD_TYPES: OnceLock<ModStructureProcessorTypes> = OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::RefCell<Option<ModStructureProcessorTypes>> =
        const { std::cell::RefCell::new(None) };
}

/// Install the process-wide table once; a second installation keeps the first
/// and hands its own types back.
pub fn install_mod_structure_processor_types(
    types: ModStructureProcessorTypes,
) -> Result<(), ModStructureProcessorTypes> {
    MOD_TYPES.set(types)
}

/// The installed processor type `id`: none until the executable installs the
/// mods', and a test's own while one is installed.
pub fn mod_structure_processor_type(id: &str) -> Option<Arc<dyn ModStructureProcessorType>> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(found) =
        TEST_TYPES.with(|cell| cell.borrow().as_ref().map(|types| types.get(id).cloned()))
    {
        return found;
    }
    MOD_TYPES.get()?.get(id).cloned()
}

/// Runs `with` over a test's own structure processor types, as decoding reads
/// them. Restores the previous scoped table even if `with` panics.
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_structure_processor_types<T>(
    types: &ModStructureProcessorTypes,
    with: impl FnOnce() -> T,
) -> T {
    struct Guard(Option<ModStructureProcessorTypes>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_TYPES.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let prev = TEST_TYPES.with(|cell| cell.borrow_mut().replace(types.clone()));
    let _guard = Guard(prev);
    with()
}
