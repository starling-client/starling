//! Structure processors (`minecraft:worldgen/processor_list`). A trait mod
//! registers a structure processor type with
//! [`Registrar::register_structure_processor_type`](crate::Registrar::register_structure_processor_type)
//! (NeoForge's registry `STRUCTURE_PROCESSOR`, vanilla's
//! `StructureProcessorType`). A processor list entry with
//! `"processor_type": "modid:weathering"` is then decoded by the mod's code and
//! runs during template placement beside vanilla's processors.
//!
//! As with structure and pool element types, there is no `Host` here.
//! Processors are decoded once at load and run on the worldgen worker threads,
//! so a processor keeps no mutable state and the same random gives the same
//! results.
use super::structure_template::{StructureMirror, StructureRotation};
use super::structure_type::{HeightmapType, LiquidSettings, StructureError};
use crate::{BiomeKey, BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, RandomSource};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::{
    bounding_box::BoundingBox,
    nbt::{Compound, Tag},
};

/// A mod's structure processor type: the codec of vanilla's
/// `StructureProcessorType`. The game keeps one instance per registered type.
/// It must be deterministic and stateless.
pub trait StructureProcessorType: Send + Sync + 'static {
    /// Decode one processor. `config` is the processor list entry converted
    /// from the data pack's JSON to NBT, with `processor_type` included. A
    /// typed error fails the processor list's decode (reported, and the rest
    /// of the data pack loads on), matching vanilla's codec failure for an
    /// invalid processor.
    fn decode(
        &self,
        context: &dyn ProcessorDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn StructureProcessor>, StructureError>;
}

/// What [`StructureProcessorType::decode`] sees while a processor list loads.
pub trait ProcessorDecodeContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The block's default state; resolve once at decode rather than per block.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// Decode a nested processor of any type, vanilla's or a mod's, through
    /// `StructureProcessorType.SINGLE_CODEC`.
    fn processor(&self, config: &Tag) -> Result<StructureProcessorRef, StructureError>;
    /// Decode a processor list holder (a registry ID, `{"processors": [...]}`,
    /// or a list of processors) through `StructureProcessorType.LIST_CODEC`.
    fn processor_list(&self, config: &Tag) -> Result<StructureProcessorListRef, StructureError>;
}

/// A decoded structure processor of any type, vanilla's or a mod's: a handle
/// the game resolves with no lookup per block. Clone freely; clones share one
/// processor.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to a processor the game holds on the worldgen workers.
pub struct StructureProcessorRef {
    engine: Arc<dyn Any + Send + Sync>,
    mod_processor: Option<Arc<dyn StructureProcessor>>,
}
impl std::fmt::Debug for StructureProcessorRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructureProcessorRef")
            .field("mod_processor", &self.mod_processor)
            .finish_non_exhaustive()
    }
}
impl StructureProcessorRef {
    /// The processor as a mod's processor of type `T`, when it is one.
    /// Vanilla's processors and other mod types give `None`.
    pub fn downcast_ref<T: StructureProcessor>(&self) -> Option<&T> {
        let processor: &dyn Any = self.mod_processor.as_deref()?;
        processor.downcast_ref()
    }
}

/// Construct the handle the game answers nested processor decodes with.
// hidden: only the game's adapter builds a processor handle; mods receive one from decode.
#[doc(hidden)]
pub fn structure_processor_ref(
    engine: Arc<dyn Any + Send + Sync>,
    mod_processor: Option<Arc<dyn StructureProcessor>>,
) -> StructureProcessorRef {
    StructureProcessorRef {
        engine,
        mod_processor,
    }
}

/// The game's own processor behind a handle.
// hidden: only the game's adapter reads its processor back from a handle.
#[doc(hidden)]
pub fn structure_processor_payload(
    processor: &StructureProcessorRef,
) -> &Arc<dyn Any + Send + Sync> {
    &processor.engine
}

/// A decoded processor list: a handle the game resolves with no lookup per
/// block. Clone freely; clones share one list.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to a processor list the game holds on the worldgen workers.
pub struct StructureProcessorListRef {
    engine: Arc<dyn Any + Send + Sync>,
}
impl std::fmt::Debug for StructureProcessorListRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructureProcessorListRef")
            .finish_non_exhaustive()
    }
}

/// Construct the handle the game answers nested processor list decodes with.
// hidden: only the game's adapter builds a processor list handle; mods receive one from decode.
#[doc(hidden)]
pub fn structure_processor_list_ref(
    engine: Arc<dyn Any + Send + Sync>,
) -> StructureProcessorListRef {
    StructureProcessorListRef { engine }
}

/// The game's own processor list behind a handle.
// hidden: only the game's adapter reads its processor list back from a handle.
#[doc(hidden)]
pub fn structure_processor_list_payload(
    list: &StructureProcessorListRef,
) -> &Arc<dyn Any + Send + Sync> {
    &list.engine
}

/// `StructureTemplate.StructureBlockInfo`: one block of a template, either its
/// original entry (template-relative `pos`) or the processed entry (world
/// `pos`) passed through structure processors.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct StructureBlockInfo {
    /// Block position: template-relative for an original entry, world
    /// coordinates for a processed entry.
    pub pos: [i32; 3],
    /// The block state.
    pub state: BlockState,
    /// Optional block-entity NBT, without position coordinates.
    pub nbt: Option<Compound>,
}
impl StructureBlockInfo {
    /// Construct a block info entry.
    pub fn new(pos: [i32; 3], state: BlockState, nbt: Option<Compound>) -> Self {
        Self { pos, state, nbt }
    }
}

/// Read-only snapshot of vanilla's `StructurePlaceSettings` passed to
/// processors during template placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct StructurePlaceSettings {
    /// Mirror applied before rotation.
    pub mirror: StructureMirror,
    /// Rotation applied about `pivot`.
    pub rotation: StructureRotation,
    /// Local rotation pivot inside the template.
    pub pivot: [i32; 3],
    /// Optional clipping box (the chunk being generated, or `None` when
    /// unclipped).
    pub bounding_box: Option<BoundingBox>,
    /// Whether placed waterloggable blocks take on water already in the world.
    pub liquid: LiquidSettings,
    /// `StructurePlaceSettings.getKnownShape`: whether post-placement neighbor
    /// shape updates are skipped.
    pub known_shape: bool,
    /// `StructurePlaceSettings.isIgnoreEntities`: whether template entities
    /// are skipped.
    pub ignore_entities: bool,
    /// `StructurePlaceSettings.shouldFinalizeEntities`: whether placed mobs
    /// run `Mob.finalizeSpawn`.
    pub finalize_entities: bool,
}
impl StructurePlaceSettings {
    /// Default placement settings with `rotation`: no mirror, zero pivot, no
    /// bounding box, waterlogging applied, shape updates enabled, entities
    /// placed without finalization.
    pub fn new(rotation: StructureRotation) -> Self {
        Self {
            mirror: StructureMirror::None,
            rotation,
            pivot: [0; 3],
            bounding_box: None,
            liquid: LiquidSettings::ApplyWaterlogging,
            known_shape: false,
            ignore_entities: false,
            finalize_entities: false,
        }
    }
}

/// A decoded structure processor of a mod's type: vanilla's
/// `StructureProcessor`. Runs on worldgen worker threads, so it must be
/// immutable and deterministic.
pub trait StructureProcessor: Any + Send + Sync + std::fmt::Debug {
    /// `StructureProcessor.processBlock`: inspect or transform one block of a
    /// template. `original` is the template's untransformed entry (with
    /// template-relative `pos`); `current` is the entry produced by earlier
    /// processors (with world `pos`). Return `Ok(Some(info))` to keep or
    /// replace the block, or `Ok(None)` to discard it (vanilla's `null`).
    fn process_block(
        &self,
        context: &mut dyn ProcessorContext,
        original: &StructureBlockInfo,
        current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError> {
        let _ = (context, original);
        Ok(Some(current))
    }

    /// `StructureProcessor.finalizeProcessing`: run once after `process_block`
    /// has visited the piece's blocks. `original` holds the kept blocks'
    /// original template entries in matching index order; `processed` holds
    /// the current processed entries. Return the final block list.
    fn finalize_processing(
        &self,
        context: &mut dyn ProcessorFinalizeContext,
        original: &[StructureBlockInfo],
        processed: Vec<StructureBlockInfo>,
    ) -> Result<Vec<StructureBlockInfo>, StructureError> {
        let _ = (context, original);
        Ok(processed)
    }

    /// `StructureProcessor.evaluatesEntirePieceState`: when `true`, the engine
    /// runs `process_block` across all blocks of the template rather than
    /// skipping blocks outside `settings.bounding_box` before
    /// `finalize_processing` (matching vanilla's `CappedProcessor`).
    fn evaluates_entire_piece_state(&self) -> bool {
        false
    }
}

/// Level reader and placement settings available to
/// [`StructureProcessor::process_block`], mirroring vanilla's `LevelReader`,
/// placement offset, reference pivot, `StructurePlaceSettings`, and the random
/// `settings.getRandom(pos)` derives.
pub trait ProcessorContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The piece's placement position (`targetPosition` / `position`).
    fn offset(&self) -> [i32; 3];
    /// The structure's reference position (`referencePos`).
    fn pivot(&self) -> [i32; 3];
    /// Read-only snapshot of the template placement settings.
    fn settings(&self) -> StructurePlaceSettings;
    /// `StructurePlaceSettings.getRandom(pos)` for the current block (in
    /// `process_block`) or the piece offset (in `finalize_processing`): the
    /// settings' shared random when one was set, otherwise a position-seeded
    /// `LegacyRandomSource`.
    fn random(&mut self) -> &mut dyn RandomSource;
    /// `StructurePlaceSettings.getRandom(pos)` for an explicit position `pos`:
    /// the settings' shared random when one was set, otherwise a `LegacyRandom`
    /// seeded from `pos`.
    fn random_at(&mut self, pos: [i32; 3]) -> &mut dyn RandomSource;
    /// The block's default state.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `BlockState.isAir`.
    fn is_air(&self, state: BlockState) -> bool;
    /// World seed (`WorldGenLevel.getSeed`), or `None` when unavailable.
    fn world_seed(&mut self) -> Option<i64>;
    /// Minimum build height Y, or `None` when unavailable.
    fn min_y(&mut self) -> Option<i32>;
    /// Maximum build height Y, or `None` when unavailable.
    fn max_y(&mut self) -> Option<i32>;
    /// World sea level, or `None` when unavailable.
    fn sea_level(&mut self) -> Option<i32>;
    /// `LevelReader.getBlockState(pos)`, or `None` where unreadable.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// Whether the world block at `pos` is in block tag `tag`.
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool>;
    /// Whether `state` is in block tag `tag`.
    fn state_in_tag(&mut self, state: BlockState, tag: &str) -> Option<bool>;
    /// `LevelReader.getHeight(kind, x, z)`, or `None` where unavailable.
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// `LevelReader.getBiome(pos)`, or `None` where unavailable.
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey>;
    /// Whether the world fluid at `pos` is water (`getFluidState().is(WATER)`).
    fn is_water(&mut self, pos: [i32; 3]) -> Option<bool>;
    /// `BlockState.canOcclude()`.
    fn can_occlude(&mut self, state: BlockState) -> Option<bool>;
    /// Delegate `process_block` to a decoded nested processor.
    fn apply_processor(
        &mut self,
        processor: &StructureProcessorRef,
        original: &StructureBlockInfo,
        current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError>;
    /// Delegate `process_block` across every processor in a decoded nested
    /// processor list, stopping if one discards the block.
    fn apply_processor_list(
        &mut self,
        list: &StructureProcessorListRef,
        original: &StructureBlockInfo,
        current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError>;
}

/// Level access available to [`StructureProcessor::finalize_processing`]:
/// every [`ProcessorContext`] read plus block writes matching vanilla's
/// `ServerLevelAccessor`.
pub trait ProcessorFinalizeContext: ProcessorContext {
    /// `LevelWriter.setBlock(pos, state, flags)`: write a block directly into
    /// the level during finalization. Returns `false` if the write was refused
    /// or unchanged.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool;
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct StructureProcessorTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) ty: Arc<dyn StructureProcessorType>,
}

/// Registered structure processor types, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn structure_processor_type_declarations() -> Vec<(String, Arc<dyn StructureProcessorType>)> {
    crate::parts::installed_structure_processor_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.ty.clone()))
        .collect()
}
