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
use crate::{BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, RandomSource};
use std::sync::Arc;
use sunrise_core::nbt::Compound;

/// A feature's placement failed: a fault. The game discards that call's writes, logs it with
/// the feature's id and disables the type for the session. A feature that merely did not place
/// (a tree with no room) answers `Ok(false)` instead.
#[derive(Debug)]
// exhaustive: mods build it themselves as a feature's error answer
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FeatureError(pub String);
impl std::fmt::Display for FeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FeatureError {}
impl From<(crate::Identifier, String)> for FeatureError {
    fn from((source, reason): (crate::Identifier, String)) -> Self {
        Self(format!("{source}: {reason}"))
    }
}

/// A mod's feature type: vanilla's `Feature`, whose code answers for a configured feature with
/// `"type": "modid:tree"`. Implemented once, kept by the game; deterministic and stateless.
pub trait FeatureType: Send + Sync + 'static {
    /// `Feature.place(level, generator, random, origin)`: place the feature at `origin`,
    /// answering whether it placed. Its writes reach the level when it answers `Ok`, the ones
    /// before an `Ok(false)` included, as vanilla keeps them; draws are never given back. The
    /// configured feature's settings are [`context.config()`](FeatureContext::config).
    fn place(
        &self,
        context: &mut dyn FeatureContext,
        origin: [i32; 3],
    ) -> Result<bool, FeatureError>;
}

/// What a feature's [`place`](FeatureType::place) sees: vanilla's `WorldGenLevel` around the
/// origin (a decoration region on a worker, or the live level for growth and commands) and the
/// caller's random. Writes are staged: the context reads them back, and the level sees them, in
/// order, as soon as `place` answers `Ok(true)` or `Ok(false)`, so the steps after it see them
/// too. A read the level can't answer is `None`; a write onto the state already there is
/// `false`. Neither loads a chunk.
///
/// A fault discards the call's writes, logs it, and disables the feature type for the session
/// (in memory, never saved; a new load of the mod set enables it again): a later placement of it
/// answers `Ok(false)` and its root carries on, so chunks generated after the fault differ from
/// those before. A fault is a panic, an `Err`, a write the level refuses (outside the region
/// decoration may write, or outside the build height) and a work budget that ran out. Each world
/// query, block write or block-entity operation is one unit of the budget (65,536 for one root
/// placement); the budget counts work, never time. Random draws and the config, metadata and
/// state helpers are not charged. `height` reads the level as it was before the call; every
/// other read sees the call's own writes. A feature can't place another feature.
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
    /// The block at `pos` (this call's own write, if any), or `None` where the level can't be
    /// read.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// `BlockState.is(tag)` of the block at `pos`: whether it is in block tag `tag`
    /// (`minecraft:leaves`), or `None` where the level can't be read.
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool>;
    /// `LevelWriter.setBlock(pos, state, flags)`, as `Feature.setBlock` (flags
    /// `BlockUpdateFlags::NEIGHBORS | CLIENTS`) and the tree features (`| KNOWN_SHAPE`) call it:
    /// whether it will be written.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool;
    /// `WorldGenLevel.getHeight` of the heightmap kind at (x, z), as the level has it before
    /// this call's writes.
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// The block entity's saved NBT (its `id` and fields) for the block this call placed at
    /// `pos`, as `BlockEntity.loadWithComponents`: false for a position this call did not place
    /// a block at, and for data whose `id` is not that block's block entity.
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> bool;
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct FeatureTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) config: Vec<DatapackField>,
    pub(crate) ty: Arc<dyn FeatureType>,
}

/// Registered feature behaviours with their config schemas, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn feature_type_declarations() -> Vec<(String, Vec<DatapackField>, Arc<dyn FeatureType>)> {
    crate::parts::installed_feature_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.config.clone(), info.ty.clone()))
        .collect()
}

/// A configured feature's JSON decoded against its type's schema, `type` aside; the error names
/// the offending key.
// hidden: the game decodes configs at load; mods read the decoded entry from their context.
#[doc(hidden)]
pub fn decode_feature_config(
    schema: &[DatapackField],
    json: &serde_json::Value,
) -> Result<DatapackEntry, String> {
    let serde_json::Value::Object(object) = json else {
        return Err("a configured feature is a JSON object".into());
    };
    crate::world::datapack_registry::decode_record(schema, object, &["type"])
        .map_err(|error| format!("config: {error:?}"))
}
