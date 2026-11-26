//! Feature types a mod registers: vanilla's `Feature` with its own `place`, as a configured
//! feature with `"type": "modid:tree"` names it. Handed over once at startup and looked up by
//! `FeatureDefinition::decode` before its built-in fallback, so a mod feature runs through every
//! route a built-in one does: biome decoration on the worldgen workers, nested features,
//! growth, bone meal, `/place feature` and `LevelRef::place_feature`. Runs on worker threads,
//! so a type keeps no mutable state and its target carries no `Host`.
//!
//! Placements borrow the caller's world and random directly. A failed invocation keeps its
//! completed writes and draws; only a callback panic disables that registration (in the adapter).
//!
//! A mod feature runs on the thread its placement does: a worldgen worker for decoration, the
//! tick thread for growth, `/place feature` and `LevelRef::place_feature`. It sees only its
//! target, its config and the caller's random, with no `Host`, no `Game` and no hook dispatch,
//! so a worker never reaches mod state through it (#1532).
use super::{
    feature::FeatureWorld,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::heightmap::HeightmapType;
use crate::registry::{BlockRegistry, BlockStateId};
use mod_api::__private::BiomeKeys;
use mod_api::BiomeKey;
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};
use sunrise_core::nbt::Compound;
use sunrise_core::random::RandomSource;

/// A configured feature's config, as its type's adapter decoded it at load.
pub type ModFeatureConfig = Arc<dyn Any + Send + Sync>;

/// A feature type's engine side, as the executable's adapter sees it. `Send + Sync` because
/// neighbouring chunks decorate at once on worker threads; the same random gives the same
/// blocks.
pub trait ModFeatureType: Send + Sync {
    /// Decode a configured feature's JSON once at load (the settings sit inline next to
    /// `type`) against the type's declared schema; an error refuses the definition, naming it.
    fn prepare_config(&self, config: &serde_json::Value) -> Result<ModFeatureConfig, String>;
    /// `Feature.place`: true when the feature says it placed. An `Err` is the mod's failure,
    /// already rendered by the adapter inside its callback boundary (the mod's error value
    /// never crosses it).
    fn place(
        &self,
        target: &mut dyn ModFeatureTarget,
        random: &mut dyn ModFeatureRandom,
        origin: [i32; 3],
        // Decoded once and retained by the loaded definition.
        config: &(dyn Any + Send + Sync),
    ) -> Result<bool, String>;
}

/// The level a mod feature places in, object-safe: the caller's own world (a decoration
/// region or live level), never a second block store. A read the world can't answer is `None`.
/// Writes take effect immediately and return the owning world's result.
pub trait ModFeatureTarget {
    /// The numeric block registry the states are of, for the adapter's state index.
    fn registry(&self) -> Option<&'static BlockRegistry>;
    fn block(&mut self, position: [i32; 3]) -> Option<BlockStateId>;
    /// `WorldGenLevel.ensureCanWrite`: the world's generation write policy.
    fn ensure_can_write(&mut self, position: [i32; 3]) -> bool;
    /// `LevelWriter.setBlock` with vanilla's update `flags`. A refusal is just this operation's
    /// result.
    fn set_block(&mut self, position: [i32; 3], state: BlockStateId, flags: i32) -> bool;
    /// Whether the block at `position` is in block tag `tag`.
    fn block_in_tag(&mut self, position: [i32; 3], tag: &str) -> Option<bool>;
    /// `WorldGenLevel.getBiome`: the key of the biome at `position` in the world's published
    /// [`ModBiomeKeys`].
    fn biome(&mut self, position: [i32; 3]) -> Option<BiomeKey>;
    fn min_y(&mut self) -> Option<i32>;
    fn max_y(&mut self) -> Option<i32>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// Set validated block-entity data for the current block at `position`. A worldgen region
    /// cannot change block entities in a read-only full chunk.
    fn set_block_entity(&mut self, position: [i32; 3], nbt: Compound) -> bool;
}

/// The biome keys a world's mod features answer [`ModFeatureTarget::biome`] with: the snapshot
/// of the world's current data load, which the game publishes on every load and reload, so
/// that a feature's key equals the one `Host::biome_key` gives. Shared by a generator's workers
/// and by the live level: a decoration takes the snapshot current when its region opens, the live
/// level reads the current one, and before the game publishes one no biome has a key.
#[derive(Clone, Default)]
pub struct ModBiomeKeys(Arc<RwLock<Arc<BiomeKeys>>>);
impl ModBiomeKeys {
    pub fn publish(&self, keys: Arc<BiomeKeys>) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = keys;
    }
    pub fn current(&self) -> Arc<BiomeKeys> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// The caller's random, object-safe: the same stream the placement chain and the built-in
/// features draw from, in vanilla's order.
pub trait ModFeatureRandom {
    /// `nextInt(bound)`; 0 without a draw for a bound below 1.
    fn int(&mut self, bound: i32) -> i32;
    /// `nextInt()`: any `i32`.
    fn int_unbounded(&mut self) -> i32;
    fn long(&mut self) -> i64;
    fn float(&mut self) -> f32;
    fn double(&mut self) -> f64;
    fn boolean(&mut self) -> bool;
    /// `nextGaussian()`.
    fn gaussian(&mut self) -> f64;
}

/// Any `RandomSource` as a [`ModFeatureRandom`].
pub(crate) struct Draws<'a, R>(pub(crate) &'a mut R);
impl<R: RandomSource> ModFeatureRandom for Draws<'_, R> {
    fn int(&mut self, bound: i32) -> i32 {
        if bound < 1 {
            return 0;
        }
        self.0.next_int_bound(bound).unwrap_or(0)
    }
    fn int_unbounded(&mut self) -> i32 {
        self.0.next_int()
    }
    fn long(&mut self) -> i64 {
        self.0.next_long()
    }
    fn float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
    fn gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }
}

/// The stack-only view of the world loaned to one feature invocation.
struct PlacingWorld<'a, W>(&'a mut W);

impl<W: FeatureWorld + PlacementContext> ModFeatureTarget for PlacingWorld<'_, W> {
    fn registry(&self) -> Option<&'static BlockRegistry> {
        self.0.block_registry()
    }
    fn block(&mut self, position: [i32; 3]) -> Option<BlockStateId> {
        self.0.block(position).ok()
    }
    fn ensure_can_write(&mut self, position: [i32; 3]) -> bool {
        self.0.ensure_can_write(position).unwrap_or_else(|error| {
            log::error!("checking worldgen write at {position:?} failed: {error}");
            false
        })
    }
    fn set_block(&mut self, position: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        self.0
            .set_block(position, state, flags)
            .unwrap_or_else(|error| {
                log::error!("setting worldgen block at {position:?} failed: {error:?}");
                false
            })
    }
    fn block_in_tag(&mut self, position: [i32; 3], tag: &str) -> Option<bool> {
        self.0.block_matches_tag(position, tag).ok()
    }
    fn biome(&mut self, position: [i32; 3]) -> Option<BiomeKey> {
        self.0.biome_key(position).ok()
    }
    fn min_y(&mut self) -> Option<i32> {
        self.0.min_y().ok()
    }
    fn max_y(&mut self) -> Option<i32> {
        self.0.max_y().ok()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        self.0.height(kind, x, z).ok()
    }
    fn set_block_entity(&mut self, position: [i32; 3], nbt: Compound) -> bool {
        let write = || -> Result<bool, PlacementQueryError> {
            if !self.0.ensure_can_write(position)?
                || !self.0.inside_build_height(position)?
                || self.0.full_chunk_at(position)?
            {
                return Ok(false);
            }
            self.0.set_block_entity(position, nbt)?;
            Ok(true)
        };
        write().unwrap_or_else(|error| {
            log::error!("setting worldgen block entity at {position:?} failed: {error}");
            false
        })
    }
}

/// A registered feature type and the id of the mod that registered it.
pub struct ModFeatureEntry {
    pub mod_id: &'static str,
    pub ty: Arc<dyn ModFeatureType>,
}

/// A configured feature of a mod's type: the type and its config, decoded once at load.
#[derive(Clone)]
pub struct ModFeature {
    pub id: String,
    pub entry: Arc<ModFeatureEntry>,
    pub config: ModFeatureConfig,
}
impl std::fmt::Debug for ModFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModFeature")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ModFeature {
    /// One call at `origin`. A returned error fails this invocation and is reported with its
    /// source chain; the world and random keep the operations already performed.
    pub(crate) fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> bool {
        let mut target = PlacingWorld(world);
        let mut draws = Draws(random);
        match self
            .entry
            .ty
            .place(&mut target, &mut draws, origin, &*self.config)
        {
            Ok(placed) => placed,
            Err(causes) => {
                log::error!(
                    "mod {}: feature type {} failed at {origin:?}: {causes}",
                    self.entry.mod_id,
                    self.id,
                );
                false
            }
        }
    }
}

/// The installed types, `modid:tree` → type. Set once, before any world opens (the executable
/// hands the mods' registrations over), and read while decoding features.
pub type ModFeatureTypes = BTreeMap<String, Arc<ModFeatureEntry>>;

static MOD_TYPES: OnceLock<ModFeatureTypes> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::Cell<Option<&'static ModFeatureTypes>> =
        const { std::cell::Cell::new(None) };
}

/// Install the process-wide table once; a second installation keeps the first and hands its
/// own types back.
pub fn install_mod_feature_types(types: ModFeatureTypes) -> Result<(), ModFeatureTypes> {
    let mut offered = Some(types);
    MOD_TYPES.get_or_init(|| offered.take().unwrap_or_default());
    offered.map_or(Ok(()), Err)
}

/// The installed feature type `id`: none until the executable installs the mods', and a test's
/// own while one is installed.
pub fn mod_feature_type(id: &str) -> Option<&'static Arc<ModFeatureEntry>> {
    #[cfg(test)]
    if let Some(types) = TEST_TYPES.get() {
        return types.get(id);
    }
    MOD_TYPES.get()?.get(id)
}

#[cfg(test)]
/// Runs `with` over a test's own feature types, as decoding reads them.
pub fn with_test_feature_types<T>(types: &ModFeatureTypes, with: impl FnOnce() -> T) -> T {
    let leaked: &'static ModFeatureTypes = Box::leak(Box::new(types.clone()));
    let prev = TEST_TYPES.replace(Some(leaked));
    let out = with();
    TEST_TYPES.set(prev);
    out
}

#[cfg(test)]
#[path = "mod_features/tests.rs"]
mod tests;
