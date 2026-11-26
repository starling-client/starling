//! Chunk generators a mod registers (vanilla's `ChunkGenerator`), handed over
//! once at startup and looked up when a level's stem names one. Runs on the
//! worldgen worker threads, so a generator keeps no mutable state and its
//! chunk carries no `Host`.
use crate::registry::BlockStateId;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use sunrise_core::identifier::Identifier;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::position::ChunkPos;

/// The chunk a generator fills: vanilla's `ChunkAccess`, as far as terrain
/// needs it. Positions are world blocks.
pub trait ChunkAccess {
    /// The immutable registry whose numeric states this chunk stores.
    fn registry(&self) -> &'static crate::registry::BlockRegistry;
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The id of the level this chunk belongs to.
    fn level(&self) -> &str;
    /// The level's seed: the world's seed.
    fn seed(&self) -> i64;
    /// The lowest block Y the terrain fills: the level's, or with
    /// [`VanillaStages`] its noise settings'.
    fn min_y(&self) -> i32;
    /// The blocks up from `min_y` the terrain fills: the level's build height,
    /// or with [`VanillaStages`] its noise settings' height.
    fn height(&self) -> i32;
    /// A block state by block name (`stone`, `minecraft:oak_log`), with its
    /// default properties; air for a block the game doesn't have.
    fn state(&self, name: &str) -> BlockStateId;
    /// A block state by block name with properties (`oak_log` with `axis=y`);
    /// missing properties keep defaults.
    fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId;
    /// `setBlockState`: write `state` at `pos`, answering whether it was
    /// written. A position outside the chunk or that range is skipped.
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool;
    /// `ChunkAccess.setBlockEntityNbt`: keep `nbt` as the chunk's pending block-entity data at
    /// the position [`pending_block_entity_pos`] reads from it, replacing what was pending there.
    /// It becomes a block entity when the chunk becomes a full chunk, and is dropped then if the
    /// block there has none (`LevelChunk.promotePendingBlockEntity`).
    fn set_block_entity_nbt(&mut self, nbt: Compound);
}

/// Where a generator's pending block-entity data goes in `chunk`: [`block_entity_pos`], or None
/// for data whose `id` is not a block entity type id, which vanilla's `BlockEntity.loadStatic`
/// skips when the chunk promotes it, or for a block outside the level's `heights`, where no
/// block entity can be.
pub fn pending_block_entity_pos(
    chunk: ChunkPos,
    nbt: &Compound,
    heights: std::ops::Range<i32>,
) -> Option<[i32; 3]> {
    let id = match nbt.get(&"id".into()) {
        Some(Tag::String(id)) => Identifier::parse(&id.to_string_lossy()).ok(),
        _ => None,
    };
    if id.is_none() {
        log::error!(
            "Skipping block entity with invalid type: {:?}",
            nbt.get(&"id".into())
        );
        return None;
    }
    Some(block_entity_pos(chunk, nbt)).filter(|pos| heights.contains(&pos[1]))
}

/// `BlockEntity.getPosFromTag`: the block `nbt`'s `x`, `y` and `z` name (`getIntOr`, 0 for a
/// missing one). A position in another chunk is moved into `chunk` at the same place within
/// its section, as vanilla does after warning.
pub fn block_entity_pos(chunk: ChunkPos, nbt: &Compound) -> [i32; 3] {
    use crate::worldgen::structure::saved_legacy::int_value;
    let [x, y, z] = ["x", "y", "z"].map(|key| int_value(nbt.get(&key.into())));
    if x >> 4 == chunk.x && z >> 4 == chunk.z {
        return [x, y, z];
    }
    log::warn!(
        "Block entity {:?} found in a wrong chunk, expected position from chunk {chunk:?}",
        Tag::Compound(nbt.clone())
    );
    [
        chunk.x.wrapping_shl(4).wrapping_add(x & 15),
        y,
        chunk.z.wrapping_shl(4).wrapping_add(z & 15),
    ]
}

/// The biome cells of the chunk a generator fills at the biome step: vanilla's `ChunkAccess`
/// as `ChunkGenerator.createBiomes` sees it. Positions are quarts (a block position `>> 2`).
pub trait ChunkBiomes {
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The id of the level this chunk belongs to.
    fn level(&self) -> &str;
    /// The level's seed: the world's seed.
    fn seed(&self) -> i64;
    /// The level's lowest block Y (`getMinY`).
    fn min_y(&self) -> i32;
    /// The level's build height (`getHeight`).
    fn height(&self) -> i32;
    /// `ChunkAccess.fillBiomesFromNoise`: every 4×4×4 cell of the chunk takes the biome
    /// `resolver` names for its quart position, asked once per cell, sections bottom to top and
    /// within each x, then y, then z (`LevelChunkSection.fillBiomesFromNoise`). A biome the
    /// world does not have fails the chunk's biome step, naming it.
    fn fill_biomes_from_noise(&mut self, resolver: &mut dyn FnMut([i32; 3]) -> Identifier);
}

/// The vanilla stages a level runs around a mod's terrain
/// ([`OverworldGenerator::for_mod`](super::generator::OverworldGenerator::for_mod)).
/// None of them is the generator's terrain alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VanillaStages {
    /// The noise settings' material rule over the terrain.
    pub surface: bool,
    /// The biomes' carvers, with the noise settings' aquifer.
    pub carvers: bool,
    /// The structure starts and references, and the pieces in FEATURES.
    pub structures: bool,
    /// The biomes' placed features in FEATURES.
    pub features: bool,
}

impl VanillaStages {
    /// Whether the level runs any of them.
    pub fn any(self) -> bool {
        self.surface || self.carvers || self.structures || self.features
    }
}

/// A chunk generator's engine side, as the executable's adapter sees it.
/// `Send + Sync` because neighbouring chunks fill at once on worker threads;
/// the same seed gives the same terrain.
pub trait ModChunkGenerator: Send + Sync {
    /// `ChunkGenerator.buildTerrain`: the chunk's terrain.
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess);
    /// The vanilla stages its levels run, as the mod answered at registration.
    fn vanilla_stages(&self) -> VanillaStages {
        VanillaStages::default()
    }
    /// `ChunkGenerator.createBiomes`: the chunk's biome cells, after the level's biome source
    /// filled them. None by default: the source's biomes stay.
    fn create_biomes(&self, _chunk: &mut dyn ChunkBiomes) {}
}

/// The installed generators, `modid:void` → generator. Set once, before any
/// world opens (the executable hands the mods' registrations over).
pub type ModChunkGenerators = BTreeMap<Identifier, Arc<dyn ModChunkGenerator>>;

static MOD_GENERATORS: OnceLock<ModChunkGenerators> = OnceLock::new();
static EMPTY: ModChunkGenerators = BTreeMap::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_chunk_generators(
    generators: ModChunkGenerators,
) -> Result<(), ModChunkGenerators> {
    MOD_GENERATORS.set(generators)
}

/// The installed generators: empty until the executable installs the mods'.
pub fn mod_chunk_generators() -> &'static ModChunkGenerators {
    MOD_GENERATORS.get().unwrap_or(&EMPTY)
}
