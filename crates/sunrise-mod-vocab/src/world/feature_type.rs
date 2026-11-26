//! Feature types a trait mod registers: vanilla's `Feature` with its own `place`, as
//! [`Registrar::register_feature_type`](crate::Registrar::register_feature_type) hands it over
//! once at startup (the `FEATURE_TYPE` registry; a
//! configured feature is an entry of `minecraft:worldgen/feature`). A configured feature with
//! `"type": "modid:tree"` then places by the mod's code, wherever the game places features:
//! biome decoration, a nested feature (a random selector's choice), a sapling's growth, bone
//! meal, `/place feature` and `LevelRef::place_feature`.
//!
//! There is no `Host` here: decoration runs on the worldgen worker threads, so the mod sees only
//! the small context below, deterministic for the random it is given. A type keeps no mutable
//! state, because two workers can decorate neighbouring chunks at once. Everything around the
//! feature stays the game's own: the placed feature's modifiers, the biome's steps and the
//! decoration seeds.
//!
//! A type declares its config as a flat record ([`DatapackField`]s, the schema datapack
//! registries use); the game decodes each configured feature against it when the definition
//! loads, so a config that doesn't fit is refused then, naming the feature, never during
//! generation.
//!
//! Feature types run on the worldgen workers.
use crate::world::datapack_registry::{DatapackEntry, DatapackField};
use crate::world::structure_type::HeightmapType;
use crate::{
    BiomeKey, BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, RandomSource, TagKey,
};
use std::sync::Arc;
use sunrise_core::nbt::Compound;

/// A feature's placement failed. The game logs the error and its causes with the registering
/// mod, feature type and origin, then treats this invocation as unsuccessful. Completed writes
/// and draws stay, and later invocations still run. A feature that merely did not place (a tree
/// with no room) answers `Ok(false)` instead.
// no wire: retains the native source chain for this in-process callback.
#[derive(Debug)]
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct FeatureError {
    source: Box<dyn std::error::Error + Send + Sync>,
}
impl FeatureError {
    /// Preserve the original error for [`std::error::Error::source`].
    /// For a standalone message, use `std::io::Error::other(message)`.
    pub fn new(cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(cause),
        }
    }
}
impl std::fmt::Display for FeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, f)
    }
}
impl std::error::Error for FeatureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// A mod's feature type: vanilla's `Feature`, whose code answers for a configured feature with
/// `"type": "modid:tree"`. Implemented once, kept by the game; deterministic and stateless.
pub trait FeatureType: Send + Sync + 'static {
    /// `Feature.place(level, generator, random, origin)`: place the feature at `origin`,
    /// answering whether it placed. Writes take effect immediately and draws are never given
    /// back.
    ///
    /// Return `Ok(false)` when the feature simply did not place (for example, no room). Ordinary
    /// per-operation refusals from [`FeatureContext`] return `false` or `None` without failing the
    /// invocation or disabling the callback. Returning `Err` logs the registering mod, feature
    /// type, origin and cause chain each time and ends only this invocation without undoing
    /// completed writes or draws or disabling later invocations. A panic is logged and queued for
    /// an error toast naming the mod, and disables only this registered callback for the session
    /// without undoing its work. The configured feature's settings are
    /// [`context.config()`](FeatureContext::config).
    fn place(
        &self,
        context: &mut dyn FeatureContext,
        origin: [i32; 3],
    ) -> Result<bool, FeatureError>;
}

/// What a feature's [`place`](FeatureType::place) sees: vanilla's `WorldGenLevel` around the
/// origin (a decoration region on a worker, or the live level for growth and commands) and the
/// caller's random. Reads see the world's current state, including completed writes from this
/// call. A read the level can't answer is `None`. An individual refused write returns `false`
/// without failing the invocation, undoing earlier writes or disabling the callback. These
/// operations do not load chunks. A feature can't place another feature.
///
/// Features run on a worldgen worker for decoration and on the tick thread for growth, commands
/// and `LevelRef::place_feature`; the context holds no `Host`, so a worker never reaches mod
/// state through it.
pub trait FeatureContext {
    /// Immutable, load-owned block metadata, safe to borrow on this worker.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The configured feature's settings, decoded against the type's schema when the
    /// definition loaded: one value per declared field, defaults filled in.
    fn config(&self) -> &DatapackEntry;
    /// The caller's random: the same stream the placement modifiers before this feature drew
    /// from, so draws stay in vanilla's order.
    fn random(&mut self) -> &mut dyn RandomSource;
    /// The level's minimum block Y.
    fn min_y(&mut self) -> Option<i32>;
    /// The level's maximum block Y.
    fn max_y(&mut self) -> Option<i32>;
    /// The block's default state; resolve it once, not once per placed block.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `BlockState.isAir`.
    fn is_air(&self, state: BlockState) -> bool;
    /// The current block at `pos`, or `None` where the level can't be read. During `FEATURES`,
    /// reads reach the eight-chunk `STRUCTURE_STARTS` dependency radius around the center chunk;
    /// reads outside the one-chunk write zone log vanilla's unsafe-read warning and return
    /// resident blocks or empty `STRUCTURE_STARTS` air (`void_air` outside build height).
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// `BlockState.is(tag)` of the block at `pos`: whether it is in block tag `tag`
    /// (for example `minecraft:leaves`), or `None` where the level or tag is unavailable.
    fn block_in_tag(&mut self, pos: [i32; 3], tag: TagKey) -> Option<bool>;
    /// `WorldGenLevel.getBiome(pos)` (`LevelReader.getBiome`), as vanilla's
    /// `SnowAndFreezeFeature` reads it and NeoForge mods read it the same way: the biome the
    /// level holds at `pos`, the noise biome of the quart the biome zoom picks for that block,
    /// the same read the game's own features make. `None` where the level can't be read, as
    /// [`block`](Self::block) answers: in decoration, outside the 3×3 chunks around the one
    /// being decorated. The key equals what [`KeysHost::biome`](crate::KeysHost::biome) returns
    /// via `BiomeView::key()` for the same id.
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey>;
    /// `WorldGenLevel.ensureCanWrite(pos)`. During `FEATURES` it allows the center chunk and its
    /// eight neighbors, restricted throughout by the center's generation height if it is
    /// upgrading. This is an admission check, not a guarantee of a change: the live level
    /// allows the check and enforces its own bounds when writing, and read-only full chunks
    /// in the region can admit a write without applying it.
    fn ensure_can_write(&mut self, pos: [i32; 3]) -> bool;
    /// `LevelWriter.setBlock(pos, state, flags)` applies the write immediately and returns the
    /// world's result. A refused write (including an unregistered state or unwritable position)
    /// returns `false` without failing the invocation; an unchanged state can return `true`
    /// in a generation region and `false` on the live level. `Feature.setBlock` uses
    /// `BlockUpdateFlags::NEIGHBORS | CLIENTS`.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool;
    /// `WorldGenLevel.getHeight` of the heightmap kind at `(x, z)`, including completed writes.
    /// During `FEATURES`, reads within the eight-chunk `STRUCTURE_STARTS` dependency radius
    /// outside the one-chunk write zone log vanilla's unsafe-read warning and return the resident
    /// column's height or `min_y` for an empty `STRUCTURE_STARTS` holder.
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// Apply saved block-entity NBT (its `id` and fields) to the current block at `pos`, as
    /// `WorldGenRegion.getBlockEntity` and `BlockEntity.loadWithComponents` do; the block need
    /// not have been placed by this call. Returns `false` for invalid data (including an ID
    /// that does not fit the block), an unreadable or unwritable position, or a read-only full
    /// chunk.
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> bool;
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct FeatureTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) mod_id: &'static str,
    pub(crate) config: Vec<DatapackField>,
    pub(crate) ty: Arc<dyn FeatureType>,
}

/// Registered feature behaviours with their config schemas, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn feature_type_declarations() -> Vec<(
    String,
    &'static str,
    Vec<DatapackField>,
    Arc<dyn FeatureType>,
)> {
    crate::parts::installed_feature_type_list()
        .iter()
        .map(|info| {
            (
                info.id.to_string(),
                info.mod_id,
                info.config.clone(),
                info.ty.clone(),
            )
        })
        .collect()
}

/// A configured feature's JSON decoded against its type's schema. The dispatch `type` key is
/// allowed even if the schema does not declare it; errors name the offending key.
// hidden: the game decodes configs at load; mods read the decoded entry from their context.
#[doc(hidden)]
pub fn decode_feature_config(
    schema: &[DatapackField],
    json: &serde_json::Value,
) -> Result<DatapackEntry, String> {
    let serde_json::Value::Object(object) = json else {
        return Err("a configured feature is a JSON object".into());
    };
    use crate::world::datapack_registry::{RecordMetadata, decode_record};
    decode_record(schema, object, RecordMetadata::FeatureType)
        .map_err(|error| format!("config: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::FeatureError;
    use crate::{BlockStateError, StructureError};
    use std::error::Error;

    #[test]
    fn worldgen_errors_retain_typed_sources() {
        let feature = FeatureError::new(BlockStateError::UnknownProperty(2));
        let structure = StructureError::new(BlockStateError::InvalidValue(3));
        let cases: [(&dyn Error, BlockStateError); 2] = [
            (&feature, BlockStateError::UnknownProperty(2)),
            (&structure, BlockStateError::InvalidValue(3)),
        ];

        for (error, expected) in cases {
            let source = error.source().unwrap();
            assert_eq!(source.downcast_ref::<BlockStateError>(), Some(&expected));
        }
    }
}
