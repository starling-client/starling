//! Placement modifiers (`minecraft:worldgen/placed_feature`). A trait mod
//! registers a placement modifier type with
//! [`Registrar::register_placement_modifier_type`](crate::Registrar::register_placement_modifier_type)
//! (NeoForge's registry `PLACEMENT_MODIFIER_TYPE`, vanilla's
//! `PlacementModifierType`). A placed feature's `placement` entry with
//! `"type": "modid:below_sea_level"` is then decoded by the mod's code and
//! runs in the placement chain beside vanilla's modifiers.
//!
//! As with feature and structure types, there is no `Host` here. Modifiers are
//! decoded once at load and run on the worldgen worker threads, so a modifier
//! keeps no mutable state and the same random gives the same positions.
use super::feature_type::FeatureError;
use super::structure_type::HeightmapType;
use crate::{BiomeKey, BlockKey, BlockMetadata, BlockState, RandomSource};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::nbt::Tag;

/// A mod's placement modifier type: the codec of vanilla's
/// `PlacementModifierType`. The game keeps one instance per registered type.
/// It must be deterministic and stateless.
pub trait PlacementModifierType: Send + Sync + 'static {
    /// Decode one placement modifier. `config` is the placement entry converted
    /// from the data pack's JSON to NBT, with `type` included. A typed error
    /// fails the placed feature's decode (reported, and the rest of the data
    /// pack loads on), matching vanilla's codec failure for an invalid
    /// placement modifier.
    fn decode(
        &self,
        context: &dyn PlacementModifierDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn PlacementModifier>, FeatureError>;
}

/// What [`PlacementModifierType::decode`] sees while a placed feature loads.
pub trait PlacementModifierDecodeContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The block's default state; resolve once at decode rather than per call.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// Decode a nested placement modifier of any type, vanilla's or a mod's,
    /// through `PlacementModifier.CODEC`. This is how a modifier that wraps
    /// another holds it.
    fn modifier(&self, config: &Tag) -> Result<PlacementModifierRef, FeatureError>;
}

/// A decoded placement modifier of any type, vanilla's or a mod's: a handle
/// the game resolves with no lookup per call. Clone freely; clones share one
/// modifier.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to a placement modifier the game holds on the worldgen workers.
pub struct PlacementModifierRef {
    engine: Arc<dyn Any + Send + Sync>,
    mod_modifier: Option<Arc<dyn PlacementModifier>>,
}
impl std::fmt::Debug for PlacementModifierRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlacementModifierRef")
            .field("mod_modifier", &self.mod_modifier)
            .finish_non_exhaustive()
    }
}
impl PlacementModifierRef {
    /// The modifier as a mod's modifier of type `T`, when it is one. Vanilla's
    /// modifiers and other mod types give `None`.
    pub fn downcast_ref<T: PlacementModifier>(&self) -> Option<&T> {
        let modifier: &dyn Any = self.mod_modifier.as_deref()?;
        modifier.downcast_ref()
    }
}

/// Construct the handle the game answers nested placement modifier decodes with.
// hidden: only the game's adapter builds a placement modifier handle; mods receive one from decode.
#[doc(hidden)]
pub fn placement_modifier_ref(
    engine: Arc<dyn Any + Send + Sync>,
    mod_modifier: Option<Arc<dyn PlacementModifier>>,
) -> PlacementModifierRef {
    PlacementModifierRef {
        engine,
        mod_modifier,
    }
}

/// The game's own placement modifier behind a handle.
// hidden: only the game's adapter reads its placement modifier back from a handle.
#[doc(hidden)]
pub fn placement_modifier_payload(modifier: &PlacementModifierRef) -> &Arc<dyn Any + Send + Sync> {
    &modifier.engine
}

/// A decoded placement modifier of a mod's type: vanilla's
/// `PlacementModifier`. Runs on worldgen worker threads, so it must be
/// immutable and deterministic.
pub trait PlacementModifier: Any + Send + Sync + std::fmt::Debug {
    /// `PlacementModifier.modify(context, random, origin, output)`: pass zero
    /// or more positions, in order, to `output`, which hands them to the next
    /// modifier in the placed feature's chain (or to the configured feature
    /// itself). An error from `output` means placement stops: return it, as
    /// `output(pos)?` does.
    fn modify(
        &self,
        context: &mut dyn PlacementModifierContext,
        random: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), FeatureError>,
    ) -> Result<(), FeatureError>;
}

/// Level and generation queries available to
/// [`PlacementModifier::modify`], mirroring vanilla's `PlacementContext`.
pub trait PlacementModifierContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The block's default state.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `BlockState.isAir`.
    fn is_air(&self, state: BlockState) -> bool;
    /// World seed (`WorldGenLevel.getSeed`), or `None` when unavailable.
    fn world_seed(&mut self) -> Option<i64>;
    /// `PlacementContext.getMinY`, or `None` when unavailable.
    fn min_y(&mut self) -> Option<i32>;
    /// Maximum build height Y, or `None` when unavailable.
    fn max_y(&mut self) -> Option<i32>;
    /// World sea level, or `None` when unavailable.
    fn sea_level(&mut self) -> Option<i32>;
    /// `PlacementContext.getBlockState(pos)`, or `None` where unreadable.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// Whether the world block at `pos` is in block tag `tag`.
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool>;
    /// `PlacementContext.getHeight(kind, x, z)`, or `None` where unavailable.
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// `WorldGenLevel.getBiome(pos)`, or `None` where unavailable.
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey>;
    /// Run a decoded nested placement modifier on `origin`, sharing `random`
    /// and this placement context, passing each position it produces to
    /// `output`.
    fn apply_modifier(
        &mut self,
        modifier: &PlacementModifierRef,
        random: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), FeatureError>,
    ) -> Result<(), FeatureError>;
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct PlacementModifierTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) ty: Arc<dyn PlacementModifierType>,
}

/// Registered placement modifier types, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn placement_modifier_type_declarations() -> Vec<(String, Arc<dyn PlacementModifierType>)> {
    crate::parts::installed_placement_modifier_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.ty.clone()))
        .collect()
}
