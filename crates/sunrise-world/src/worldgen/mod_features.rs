//! Feature types a mod registers: vanilla's `Feature` with its own `place`, as a configured
//! feature with `"type": "modid:tree"` names it. Handed over once at startup and looked up by
//! `FeatureDefinition::decode` before its built-in fallback, so a mod feature runs through every
//! route a built-in one does: biome decoration on the worldgen workers, nested features,
//! growth, bone meal, `/place feature` and `LevelRef::place_feature`. Runs on worker threads,
//! so a type keeps no mutable state and its target carries no `Host`.
//!
//! A call's writes are staged ([`Staging`]): the feature reads its own writes, and when it
//! answers `Ok(true)` or `Ok(false)` they reach the level at once, in order, so the steps after
//! it (built-in or a mod's) see them. An `Ok(false)` publishes what it wrote, as vanilla keeps
//! the blocks of a partly placed tree. Only a fault discards the call's writes: a panic, an
//! `Err`, a work budget that ran out, or a write the level refuses (outside the region
//! decoration may write, or the build height). A fault also disables the type for the session,
//! in memory only: a later placement of it answers `Ok(false)` and its root carries on, and a
//! new load of the mod set enables it again. The call that faults and the root around it are
//! not rolled back beyond that call: vanilla has no root rollback either. The budget counts
//! work units, never time, so a run's output does not depend on how fast a machine is.
//!
//! A mod feature runs on the thread its placement does: a worldgen worker for decoration, the
//! tick thread for growth, `/place feature` and `LevelRef::place_feature`. It sees only its
//! target, its config and the caller's random, with no `Host`, no `Game` and no hook dispatch,
//! so a worker never reaches mod state through it (#1532).
use crate::heightmap::HeightmapType;
use crate::registry::{BlockRegistry, BlockStateId};
use std::any::Any;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use sunrise_core::nbt::Compound;
use sunrise_core::random::RandomSource;

/// The work units one root placement lends its mod features, together: every query, write and
/// block entity a callback asks for is one unit. Separate from the graph's own budget, so the
/// built-in features keep their capacity.
pub const CUSTOM_WORK_BUDGET: u32 = 65_536;

/// A configured feature's config, as its type's adapter decoded it at load.
pub type ModFeatureConfig = Arc<dyn Any + Send + Sync>;

/// A feature type's engine side, as the executable's adapter sees it. `Send + Sync` because
/// neighbouring chunks decorate at once on worker threads; the same random gives the same
/// blocks.
pub trait ModFeatureType: Send + Sync {
    /// Decode a configured feature's JSON once at load (the settings sit inline next to
    /// `type`) against the type's declared schema; an error refuses the definition, naming it.
    fn prepare_config(&self, config: &serde_json::Value) -> Result<ModFeatureConfig, String>;
    /// `Feature.place`: true when the feature says it placed. An `Err` is the mod's failure.
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
/// region, the live level), never a second block store. A read the world can't answer is
/// `None`, a write onto the state already there is `false`, and a write it refuses faults the
/// call; neither loads a chunk.
pub trait ModFeatureTarget {
    /// The numeric block registry the states are of, for the adapter's state index.
    fn registry(&self) -> Option<&'static BlockRegistry>;
    fn block(&mut self, position: [i32; 3]) -> Option<BlockStateId>;
    /// `LevelWriter.setBlock` with vanilla's update `flags`: false for the state already there.
    /// A write outside the writable region or the build height answers false and faults the
    /// call.
    fn set_block(&mut self, position: [i32; 3], state: BlockStateId, flags: i32) -> bool;
    /// Whether the block at `position` is in block tag `tag`.
    fn block_in_tag(&mut self, position: [i32; 3], tag: &str) -> Option<bool>;
    fn min_y(&mut self) -> Option<i32>;
    fn max_y(&mut self) -> Option<i32>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32>;
    /// Block-entity data for the block this call placed at `position`; false anywhere else, and
    /// in a full chunk the world only reads, where the block was never placed.
    fn set_block_entity(&mut self, position: [i32; 3], nbt: Compound) -> bool;
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

/// Staged block writes and block entities, in order, with the latest state at each position
/// kept apart so a read is one lookup however many writes came before it.
#[derive(Default)]
pub(crate) struct Pending {
    writes: Vec<([i32; 3], BlockStateId, i32)>,
    latest: HashMap<[i32; 3], BlockStateId>,
    /// Block entities in the order given; a replaced one leaves a hole.
    entities: Vec<Option<([i32; 3], Compound)>>,
    entity_at: HashMap<[i32; 3], usize>,
}
impl Pending {
    fn write(&mut self, position: [i32; 3], state: BlockStateId, flags: i32) {
        self.writes.push((position, state, flags));
        self.latest.insert(position, state);
    }
    fn state(&self, position: [i32; 3]) -> Option<BlockStateId> {
        self.latest.get(&position).copied()
    }
    fn set_entity(&mut self, position: [i32; 3], nbt: Compound) {
        self.drop_entity(position);
        self.entity_at.insert(position, self.entities.len());
        self.entities.push(Some((position, nbt)));
    }
    fn drop_entity(&mut self, position: [i32; 3]) {
        if let Some(at) = self.entity_at.remove(&position) {
            self.entities[at] = None;
        }
    }
    /// The call answered `Ok`: its writes, in order, then the block entities of the blocks among
    /// them the level took. A write the level refuses now drops its block entity with it.
    pub(crate) fn commit<W>(self, world: &mut W)
    where
        W: super::feature::FeatureWorld + super::placement::PlacementContext,
    {
        let mut written: HashSet<[i32; 3]> = HashSet::new();
        for (position, state, flags) in self.writes {
            if world.set_block(position, state, flags).unwrap_or(false) {
                written.insert(position);
            } else {
                written.remove(&position);
            }
        }
        for (position, nbt) in self.entities.into_iter().flatten() {
            if written.contains(&position) {
                let _ = world.set_block_entity(position, nbt);
            }
        }
    }
}

/// Why a call faulted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fault {
    /// The root's custom work budget ran out.
    Budget,
    /// A write the level refuses: outside the region decoration may write, or the build height.
    Refused([i32; 3]),
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget => write!(f, "the custom work budget of {CUSTOM_WORK_BUDGET} ran out"),
            Self::Refused([x, y, z]) => write!(
                f,
                "a write at [{x}, {y}, {z}] is outside the region or the build height"
            ),
        }
    }
}

/// One call's view of a placement world: reads see the call's own staged writes, writes and
/// block entities wait for [`publish`](Self::publish), and every operation is charged to the
/// root's custom budget. An operation past the budget answers `None` or `false` and faults the
/// call, and so does a write the level refuses. Reads of the heightmap see the level as it was
/// before the call.
pub(crate) struct Staging<'a, W> {
    world: &'a mut W,
    budget: &'a Cell<u32>,
    own: Pending,
    fault: Option<Fault>,
}
impl<'a, W> Staging<'a, W>
where
    W: super::feature::FeatureWorld + super::placement::PlacementContext,
{
    pub(crate) fn new(world: &'a mut W, budget: &'a Cell<u32>) -> Self {
        Self {
            world,
            budget,
            own: Pending::default(),
            fault: None,
        }
    }
    fn charge(&mut self) -> bool {
        match self.budget.get().checked_sub(1) {
            Some(left) => {
                self.budget.set(left);
                true
            }
            None => {
                self.fault.get_or_insert(Fault::Budget);
                false
            }
        }
    }
    fn current(&mut self, position: [i32; 3]) -> Option<BlockStateId> {
        match self.own.state(position) {
            Some(state) => Some(state),
            None => self.world.block(position).ok(),
        }
    }
    /// The call answered `Ok`: its writes reach the level, in order.
    fn publish(self) {
        self.own.commit(self.world);
    }
}
impl<W> ModFeatureTarget for Staging<'_, W>
where
    W: super::feature::FeatureWorld + super::placement::PlacementContext,
{
    fn registry(&self) -> Option<&'static BlockRegistry> {
        self.world.block_registry()
    }
    fn block(&mut self, position: [i32; 3]) -> Option<BlockStateId> {
        if !self.charge() {
            return None;
        }
        self.current(position)
    }
    fn set_block(&mut self, position: [i32; 3], state: BlockStateId, flags: i32) -> bool {
        if !self.charge() {
            return false;
        }
        let writable = self.world.ensure_can_write(position) == Ok(true)
            && self.world.inside_build_height(position) == Ok(true);
        if !writable {
            self.fault.get_or_insert(Fault::Refused(position));
            return false;
        }
        // `Level.setBlock` answers false for the state already there.
        if self
            .current(position)
            .is_none_or(|current| current == state)
        {
            return false;
        }
        self.own.write(position, state, flags);
        true
    }
    fn block_in_tag(&mut self, position: [i32; 3], tag: &str) -> Option<bool> {
        if !self.charge() {
            return None;
        }
        match self.own.state(position) {
            Some(state) => self.world.state_matches_tag(state, tag).ok(),
            None => self.world.block_matches_tag(position, tag).ok(),
        }
    }
    fn min_y(&mut self) -> Option<i32> {
        if !self.charge() {
            return None;
        }
        self.world.min_y().ok()
    }
    fn max_y(&mut self) -> Option<i32> {
        if !self.charge() {
            return None;
        }
        self.world.max_y().ok()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        if !self.charge() {
            return None;
        }
        self.world.height(kind, x, z).ok()
    }
    fn set_block_entity(&mut self, position: [i32; 3], nbt: Compound) -> bool {
        if !self.charge()
            || self.own.state(position).is_none()
            || self.world.full_chunk_at(position).unwrap_or(false)
        {
            return false;
        }
        self.own.set_entity(position, nbt);
        true
    }
}

/// A registered type and whether a fault has disabled it for the session.
pub struct ModFeatureEntry {
    pub ty: Arc<dyn ModFeatureType>,
    disabled: AtomicBool,
}
impl ModFeatureEntry {
    pub fn new(ty: Arc<dyn ModFeatureType>) -> Self {
        Self {
            ty,
            disabled: AtomicBool::new(false),
        }
    }
    pub fn disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }
    /// Disables the type; true for the call that did, so the fault is logged once.
    fn disable(&self) -> bool {
        !self.disabled.swap(true, Ordering::Relaxed)
    }
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
    /// One call of the type at `origin` over `world`, charged to `budget`. Its writes reach the
    /// world when it answers `Ok`, `Ok(false)` included, and the answer is the type's. A fault
    /// discards them, disables the type and answers `false`, as does a type already disabled.
    pub(crate) fn place<W>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        budget: &Cell<u32>,
    ) -> bool
    where
        W: super::feature::FeatureWorld + super::placement::PlacementContext,
    {
        if self.entry.disabled() {
            return false;
        }
        let mut staging = Staging::new(world, budget);
        let mut draws = Draws(random);
        let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.entry
                .ty
                .place(&mut staging, &mut draws, origin, &*self.config)
        }));
        let reason = match (called, staging.fault.take()) {
            (_, Some(fault)) => fault.to_string(),
            (Err(_), None) => "it panicked".to_owned(),
            (Ok(Err(reason)), None) => format!("it failed: {reason}"),
            (Ok(Ok(placed)), None) => {
                staging.publish();
                return placed;
            }
        };
        if self.entry.disable() {
            let [x, y, z] = origin;
            let mod_id = self
                .id
                .split_once(':')
                .map_or(&*self.id, |(namespace, _)| namespace);
            log::error!(
                "mod {mod_id}: feature type {} faulted at [{x}, {y}, {z}] ({reason}); its writes \
                 of that call are discarded and the type is off until the mods load again, so \
                 chunks generated from now on differ",
                self.id
            );
        }
        false
    }
}

/// The installed types, `modid:tree` → type. Set once, before any world opens (the executable
/// hands the mods' registrations over), and read while decoding features.
pub type ModFeatureTypes = BTreeMap<String, Arc<dyn ModFeatureType>>;
type Entries = BTreeMap<String, Arc<ModFeatureEntry>>;

static MOD_TYPES: OnceLock<Entries> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::Cell<Option<&'static Entries>> =
        const { std::cell::Cell::new(None) };
}

fn entries(types: ModFeatureTypes) -> Entries {
    types
        .into_iter()
        .map(|(id, ty)| (id, Arc::new(ModFeatureEntry::new(ty))))
        .collect()
}

/// Install the process-wide table once; a second installation keeps the first and hands its
/// own types back.
pub fn install_mod_feature_types(types: ModFeatureTypes) -> Result<(), ModFeatureTypes> {
    let mut offered = Some(types);
    MOD_TYPES.get_or_init(|| entries(offered.take().unwrap_or_default()));
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
    let leaked: &'static Entries = Box::leak(Box::new(entries(types.clone())));
    let prev = TEST_TYPES.replace(Some(leaked));
    let out = with();
    TEST_TYPES.set(prev);
    out
}

#[cfg(test)]
#[path = "mod_features/tests.rs"]
mod tests;
