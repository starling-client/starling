//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `fluids_energy` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait FluidsEnergyBackend {
    /// Side: server.
    /// The energy the buffer of the mod block at `at` in the level in place holds, for the
    /// game's own energy capability: none where no buffer is, in another level, a chunk that
    /// isn't loaded, or the block entity ticking now, or where `at`'s side reaches neither a
    /// fill nor a drain.
    fn energy_amount(&self, at: BlockSide) -> Option<u32>;
    /// Side: server.
    /// The capacity of the buffer at `at` (`native_energy_amount`), 0 where none is.
    fn energy_capacity(&self, at: BlockSide) -> u32;
    /// Side: server.
    /// How many tanks of the block at `at` in the level in place its side reaches
    /// (none: every tank), for the game's own fluid capability: a mod block's by its table, a
    /// cauldron's one. None where no tanks are.
    fn fluid_tank_count(&self, at: BlockSide) -> Option<usize>;
    /// Side: server.
    /// What tank `tank` of those `at`'s side reaches holds (`native_fluid_tank_count`).
    fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack>;
    /// Side: server.
    /// How much of `fluid` tank `tank` of those `at`'s side reaches holds when empty: a mod
    /// block's fillable tank its capacity, a cauldron a bucket; none for another tank, a
    /// drain-only one, or a fluid the tanks there don't take: other than water and lava, or
    /// with components into a mod block's tanks. `None` queries the general capacity,
    /// represented by `u32::MAX` where no fluid-independent limit is known.
    fn fluid_tank_capacity(&self, at: BlockSide, tank: usize, fluid: Option<&FluidStack>) -> u32;
}
/// The handles' writes of the `fluids_energy` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait FluidsEnergyBackendMut {
    /// Side: server.
    /// `SimpleEnergyHandler.insert`: puts up to `amount` into the buffer at `at`
    /// (`native_energy_amount`) through its side, no more than its room and its limit per call,
    /// kept by transaction `tx`. How much went in.
    fn energy_insert(&mut self, at: BlockSide, amount: u32, tx: &TransactionContext) -> u32;
    /// Side: server.
    /// `SimpleEnergyHandler.extract`: takes up to `amount` out of the buffer at `at`
    /// (`native_energy_amount`) by its side, no more than it holds and its limit per call, kept
    /// by transaction `tx`. How much came out.
    fn energy_extract(&mut self, at: BlockSide, amount: u32, tx: &TransactionContext) -> u32;
    /// Side: server.
    /// Puts as much of `fluid` as tank `tank` of those `at`'s side reaches takes: a mod
    /// block's fillable tank, empty or holding that fluid, up to its capacity; a cauldron by
    /// whole thirds, lava only whole into an empty one, discarding the fluid's components.
    /// What didn't fit. The write is kept by
    /// transaction `tx`.
    fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        tx: &TransactionContext,
    ) -> Option<FluidStack>;
    /// Side: server.
    /// Takes up to `max` out of tank `tank` of those `at`'s side reaches: a mod block's
    /// drainable tank, a cauldron by whole thirds and lava only whole. What came out. The
    /// write is kept by transaction `tx`.
    fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        tx: &TransactionContext,
    ) -> Option<FluidStack>;
    /// Side: server.
    /// `BucketPickup.pickupBlock` for a mod's fluid container (NeoForge's
    /// `FluidUtil.tryPickupFluid`): takes the fluid source at `pos` in the level in place, a
    /// waterlogged block's water or a bubble column's, as an empty bucket would, `player`
    /// picking it up when given (only a creative player drains a barrier). What came out, a
    /// bucket of it. `Ok(None)`, with nothing done, where the block gives no fluid (a flowing
    /// fluid, powder snow, any other block), in another level, or in a chunk that isn't loaded.
    /// `Err(TransactionError::InvalidContext)`, with nothing done, for a context other than this
    /// host's innermost open transaction.
    ///
    /// Kept by transaction `tx`: the block is decided and written at the call, so later reads
    /// in the transaction see it drained and a second pickup there gets nothing; its updates, a
    /// drained block's drops when it no longer survives, and the bucket's fill sound and
    /// `minecraft:fluid_pickup` (from `player`) follow at the outermost commit. An abort leaves
    /// the world as it was, with no drop, sound or event, where NeoForge's may have changed it.
    fn level_pickup_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        player: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<Option<FluidStack>, TransactionError<()>>;
    /// Side: server.
    /// `BucketItem.emptyContents` of a bucket of `fluid` for a mod's fluid container (NeoForge's
    /// `FluidUtil.tryPlaceFluid`), with no hit result: at `pos` in the level in place, its source
    /// into air or a block it replaces (broken with its drops), into an accepting container
    /// (a part can accept nonwater), or hissing away where water evaporates (the Nether).
    /// The acceptance query lends `player` when supplied. With `validate`, a container's
    /// false placement answer fails instead of emptying the bucket (NeoForge's `validatePlaced`).
    /// `Ok(true)` when it was placed or hissed away; `Ok(false)`, with nothing done, for a fluid
    /// with no world form, where it can't be placed, in another level, or in a chunk that isn't
    /// loaded. `Err(TransactionError::InvalidContext)`, with nothing done, for a context other
    /// than this host's innermost open transaction.
    ///
    /// Kept by transaction `tx` as [`level_pickup_fluid`](Self::level_pickup_fluid) is: the
    /// block written at the call, its updates, the fluid tick, a replaced block's drops, a doused
    /// block's sound and the bucket's empty sound and `minecraft:fluid_place` (from `player`) at
    /// the outermost commit, nothing of it after an abort.
    fn level_place_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        player: Option<PlayerRef>,
        validate: bool,
        tx: &TransactionContext,
    ) -> Result<bool, TransactionError<()>>;
    /// Side: server.
    /// The bucket's sound and game event for `fluid` at the centre of `pos` in the level in
    /// place, as `FluidUtil.triggerSoundAndGameEvent` plays them (`pickup`: the fill sound and
    /// `minecraft:fluid_pickup`, else the empty sound and `minecraft:fluid_place`; a tank-only
    /// fluid has no sound), from `player` when given, at the outermost commit of transaction
    /// `tx`: nothing if it aborts. `Ok(false)`, with nothing queued, in another level;
    /// `Err(TransactionError::InvalidContext)` for a context other than this host's innermost open
    /// transaction.
    fn level_play_fluid_effects(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        pickup: bool,
        player: Option<PlayerRef>,
        tx: &TransactionContext,
    ) -> Result<bool, TransactionError<()>>;
}
