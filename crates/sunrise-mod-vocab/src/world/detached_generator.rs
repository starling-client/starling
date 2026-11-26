use crate::game::TerrainColumn;
use crate::world::chunk_snapshot::ChunkSnapshot;
use std::fmt;
use std::sync::Arc;

/// Generation stage requested from [`DetachedChunkGenerator::generate_chunk`].
///
/// Follows vanilla 26.3's `net.minecraft.world.level.chunk.status.ChunkStatus` progression
/// (`BIOMES -> TERRAIN -> FEATURES -> INITIALIZE_LIGHT -> LIGHT -> SPAWN -> FULL`), exposing the
/// three sub-phases of `ChunkStatus.TERRAIN` ([`Self::Noise`], [`Self::Surface`], and
/// [`Self::Carvers`]) so callers can stop before surface painting or cave carving when they only
/// need coarse terrain.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, sunrise_mod_api_macros::Wire,
)]
#[non_exhaustive]
pub enum GenerationStage {
    /// `ChunkStatus.BIOMES`: biome layout only (3D quart biomes, no terrain blocks).
    /// Deterministic and bit-exact with live worldgen. `has_light() == false`.
    Biomes,
    /// First sub-phase of `ChunkStatus.TERRAIN`: noise terrain and aquifers (default
    /// stone/fluid/air; flat layers for superflat), before surface rules or carvers.
    /// Deterministic and bit-exact with live worldgen. `has_light() == false`.
    Noise,
    /// Second sub-phase of `ChunkStatus.TERRAIN`: noise plus surface rules (grass, sand, snow,
    /// badlands strata, bedrock), before cave/ravine carvers.
    /// Deterministic and bit-exact with live worldgen. `has_light() == false`.
    Surface,
    /// Complete `ChunkStatus.TERRAIN`: surface plus caves and ravines.
    /// Deterministic and bit-exact with live worldgen. `has_light() == false`.
    Carvers,
    /// `ChunkStatus.FEATURES`: decorated as vanilla would (trees, lakes, ores, structures) over a
    /// 3x3 detached scratch neighbourhood. Not guaranteed identical to the saved world (cross-chunk
    /// placements from neighbours' decoration passes and generation order are not replicated).
    /// `has_light() == false`.
    Features,
    /// `ChunkStatus.LIGHT`: [`Self::Features`] followed by the engine's real light engine over the
    /// detached 3x3 neighbourhood. Produces real sky and block light (`has_light() == true`). Not
    /// guaranteed identical to the saved world for the same reasons as [`Self::Features`].
    Light,
}

/// Why a detached generation or surface sample call failed.
// no wire: returned only to the caller of the in-process DetachedChunkGenerator.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GenerationError {
    /// Level closed, client side, or dimension has no detached generator.
    Unavailable,
    /// Out-of-bounds chunk or column coordinate (`!ChunkPos.isValid`).
    InvalidPos,
    /// Generator returned an error or panicked (contained; does not crash the server).
    Failed(String),
}

impl fmt::Display for GenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("detached chunk generator is unavailable"),
            Self::InvalidPos => f.write_str("chunk or column position is out of bounds"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for GenerationError {}

// hidden: engine-only backend trait; mods interact with `DetachedChunkGenerator`.
#[doc(hidden)]
pub trait DetachedGeneratorBackend: Send + Sync + 'static {
    fn generate_chunk(
        &self,
        pos: [i32; 2],
        stage: GenerationStage,
    ) -> Result<ChunkSnapshot, GenerationError>;

    fn sample_surface_columns(
        &self,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, GenerationError>;
}

/// Thread-safe, read-only handle to a server level's world generator.
///
/// Obtained from [`crate::LevelRef::detached_generator`]. Safe to clone and invoke on background
/// worker threads (`Send + Sync + 'static`). Executes purely in memory without mutating the live
/// world, streaming tickets, or save directory. Once the level closes, every call returns
/// `Err(GenerationError::Unavailable)` and idle worker state is released immediately.
// no wire: in-process thread-safe handle to the level's world generator for worker threads.
// exhaustive: opaque handle with private backend field; constructed only by the engine.
#[derive(Clone)]
pub struct DetachedChunkGenerator {
    backend: Arc<dyn DetachedGeneratorBackend>,
}

impl DetachedChunkGenerator {
    // hidden: engine-only constructor; mods obtain generators from `LevelRef::detached_generator`.
    #[doc(hidden)]
    pub fn from_backend(backend: Arc<dyn DetachedGeneratorBackend>) -> Self {
        Self { backend }
    }

    /// Generates one chunk up to `stage` purely in memory on the caller's thread and returns it
    /// as an immutable [`ChunkSnapshot`].
    ///
    /// - Never adds the chunk to the live world or save directory.
    /// - Never fires chunk-load or chunk-save hooks.
    /// - Bit-exact with live worldgen for [`GenerationStage::Biomes`], [`GenerationStage::Noise`],
    ///   [`GenerationStage::Surface`], and [`GenerationStage::Carvers`].
    /// - [`GenerationStage::Features`] and [`GenerationStage::Light`] run decoration over a 3x3
    ///   detached scratch neighbourhood and are not guaranteed identical to the saved world.
    /// - Only [`GenerationStage::Light`] computes light (`has_light() == true`); earlier stages
    ///   never fabricate light (`has_light() == false`).
    /// - Returns `Err(GenerationError::Unavailable)` once the level closes.
    pub fn generate_chunk(
        &self,
        pos: [i32; 2],
        stage: GenerationStage,
    ) -> Result<ChunkSnapshot, GenerationError> {
        self.backend.generate_chunk(pos, stage)
    }

    /// Fast undecorated column surface sample, callable off the server tick thread without
    /// creating or holding chunks.
    ///
    /// Returns one optional [`TerrainColumn`] per input `(x, z)` in input order (`None` for void
    /// columns or custom generators without a noise surface).
    pub fn sample_surface_columns(
        &self,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, GenerationError> {
        self.backend.sample_surface_columns(columns)
    }
}

impl fmt::Debug for DetachedChunkGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DetachedChunkGenerator")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + 'static>() {}

    #[test]
    fn detached_chunk_generator_is_send_sync() {
        assert_send_sync::<DetachedChunkGenerator>();
        assert_send_sync::<GenerationStage>();
        assert_send_sync::<GenerationError>();
    }
}
