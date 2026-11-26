//! Chunk generators a native mod registers: vanilla's `ChunkGenerator` with
//! its own terrain, as
//! [`register_chunk_generator`](crate::Registrar::register_chunk_generator)
//! hands it over once at startup. A level whose stem names `modid:void`, one
//! [`register_dimension`](crate::Registrar::register_dimension) made or a data
//! pack's, then generates by the mod's code.
//!
//! There is no `Host` here: chunks fill on the worldgen worker threads, so the
//! mod sees only the [`ChunkAccess`] of the chunk it fills, and a generator
//! keeps no mutable state because two workers can fill neighbouring chunks at
//! once. The level gets the generator's terrain, then the
//! [`VanillaStages`] it asks for.
use crate::{BiomeKey, BlockKey, BlockMetadata, BlockState, ChunkPos, Compound, Identifier};
use std::sync::Arc;

/// The chunk a generator fills: vanilla's `ChunkAccess`, as far as terrain needs it. Positions
/// are world blocks.
pub trait ChunkAccess {
    /// Immutable block metadata from this worker's resource snapshot.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The id of the level this chunk belongs to (`mymod:pocket`), so one stateless generator
    /// can tell the levels it serves apart.
    fn level(&self) -> &str;
    /// The level's seed: the world's seed.
    fn seed(&self) -> i64;
    /// The lowest block Y the terrain fills: the level's (`getMinY`), or with
    /// [`VanillaStages`] its noise settings', as vanilla's noise fill reads them.
    fn min_y(&self) -> i32;
    /// How many blocks up from [`min_y`](Self::min_y) the terrain fills: the
    /// level's build height (`getHeight`), or with [`VanillaStages`] its noise
    /// settings' height inside it.
    fn height(&self) -> i32;
    /// The block's default state. Resolve before the terrain-writing loop.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    /// Resolve before the terrain-writing loop, never once per placed block.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `setBlockState`: write `state` at `pos`, answering whether it was written. A position
    /// outside the chunk or the range [`min_y`](Self::min_y) and [`height`](Self::height) give is
    /// skipped.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool;
    /// `ChunkAccess.setBlockEntityNbt`: keep `nbt` as the chunk's pending block-entity data, at
    /// the block its `x`, `y` and `z` name (`BlockEntity.getPosFromTag`: 0 for a missing one, and
    /// a position in another chunk moved into this one at the same place in its section),
    /// replacing what was pending there. Its `id` names the block entity type, as in a saved
    /// chunk: data without one, or with one that is not an identifier, is dropped, as
    /// `BlockEntity.loadStatic` skips it. When the chunk becomes a full chunk, before it first
    /// loads, the data becomes the block entity of the block there, as `LevelChunk` promotes
    /// `pendingBlockEntities`; if that block has no block entity then, the data is dropped. The
    /// tag holds what the block entity saves: a chest's `LootTable` and `LootTableSeed` wait to
    /// be rolled on first access, as a structure's chest does. Data for a block outside the
    /// level's height is dropped at once. With [`VanillaStages`], the stages after the terrain
    /// may change the block (a carver's cave, a structure piece), and a structure piece's own
    /// data at the same block replaces this. NeoForge mods call the same vanilla method.
    fn set_block_entity_nbt(&mut self, nbt: Compound);
}

/// The biome cells of the chunk a generator fills at the biome step: vanilla's `ChunkAccess` as
/// `ChunkGenerator.createBiomes` sees it, before the terrain. Positions are quarts, a block
/// position `>> 2`.
pub trait ChunkBiomes {
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The id of the level this chunk belongs to, as [`ChunkAccess::level`].
    fn level(&self) -> &str;
    /// The level's seed: the world's seed.
    fn seed(&self) -> i64;
    /// The level's lowest block Y (`getMinY`): the cells start at quart `min_y >> 2`.
    fn min_y(&self) -> i32;
    /// The level's build height (`getHeight`), which the cells cover.
    fn height(&self) -> i32;
    /// `ChunkAccess.fillBiomesFromNoise`: every 4×4×4 cell of the chunk takes the biome `resolver`
    /// names for its quart position (`BiomeResolver.getNoiseBiome`), asked once per cell,
    /// sections bottom to top and within each x, then y, then z, as
    /// `LevelChunkSection.fillBiomesFromNoise` asks. A biome the world does not have fails the
    /// chunk's biome step, as a biome source's failure does.
    fn fill_biomes_from_noise(&mut self, resolver: &mut dyn FnMut([i32; 3]) -> BiomeKey);
}

/// The vanilla stages a level runs around a mod's terrain, in vanilla's chunk
/// status order: the structure starts and references before the terrain;
/// `surface` and then `carvers` after it, as `NoiseBasedChunkGenerator.buildTerrain`
/// runs them after its noise fill; then the FEATURES status, where
/// `ChunkGenerator.applyBiomeDecoration` places the structure pieces and the
/// biomes' placed features. The default is none of them: the level is the
/// generator's terrain alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: a generator builds it with a literal as its vanilla_stages answer
#[derive(sunrise_mod_api_macros::Wire)]
pub struct VanillaStages {
    /// The level's material rule over the terrain: grass, sand, deepslate,
    /// bedrock and ore veins, from its `noise_settings`.
    pub surface: bool,
    /// The carvers of the level's biomes: caves and canyons, with the aquifer
    /// of its `noise_settings`.
    pub carvers: bool,
    /// The structures of the world's structure sets that may start in the
    /// level's biomes (`ChunkGenerator.createStructures`) and their pieces. A
    /// start is placed at the height of the level's `noise_settings` terrain
    /// (`getBaseHeight`), not of the generator's. None while the world's
    /// structures are turned off.
    pub structures: bool,
    /// The placed features of the level's biomes.
    pub features: bool,
}

/// A mod's chunk generator: vanilla's `ChunkGenerator`, whose code answers for
/// a level stem with `"generator": {"type": "modid:void"}`. Implemented once,
/// kept by the game; deterministic and stateless.
pub trait ChunkGenerator: Send + Sync + 'static {
    /// `ChunkGenerator.buildTerrain`: write the terrain of `chunk`, which
    /// starts as air. The same level id, seed and chunk give the same blocks.
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess);
    /// The vanilla stages the level runs after [`build_terrain`](Self::build_terrain),
    /// asked once, at registration. None by default.
    fn vanilla_stages(&self) -> VanillaStages {
        VanillaStages::default()
    }
    /// `ChunkGenerator.createBiomes`: the biome cells of `chunk`, at the biome step, before
    /// [`build_terrain`](Self::build_terrain) and every vanilla stage. The level's biome source
    /// has filled them; a generator that answers fills them again through
    /// [`ChunkBiomes::fill_biomes_from_noise`], as a NeoForge generator overriding `createBiomes`
    /// does. The chunk keeps these biomes: its features, its surface and what a player sees read
    /// them. Structure placement and carvers still ask the level's biome source, as vanilla's
    /// `ChunkGenerator` does. Deterministic and stateless, as `build_terrain`: a neighbour's biomes
    /// are read from its own cells. By default the source's biomes stay.
    fn create_biomes(&self, chunk: &mut dyn ChunkBiomes) {
        let _ = chunk;
    }
}

/// A registered API generator, before the game builds its worker adapter.
pub(crate) struct ChunkGeneratorInfo {
    pub(crate) id: Identifier,
    /// Its [`ChunkGenerator::vanilla_stages`], asked once at registration.
    pub(crate) stages: VanillaStages,
    pub(crate) generator: Arc<dyn ChunkGenerator>,
}

/// Registered generators for the hidden game integration boundary: each id, its
/// vanilla stages and the generator.
// hidden: the game's startup adapter reads the registered generators; mods declare them through the registrar.
#[doc(hidden)]
pub fn chunk_generator_declarations() -> Vec<(Identifier, VanillaStages, Arc<dyn ChunkGenerator>)> {
    crate::parts::installed_chunk_generator_list()
        .iter()
        .map(|info| (info.id.clone(), info.stages, info.generator.clone()))
        .collect()
}
