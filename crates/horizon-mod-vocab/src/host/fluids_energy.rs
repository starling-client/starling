//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `fluids_energy` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait FluidsEnergyBackend {
    /// Side: server.
    /// The energy the buffer of the mod block at `at` in the level in place holds, for the
    /// game's own energy capability: none where no buffer is, in another level, a chunk that
    /// isn't loaded, or the block entity ticking now, or where `at`'s side reaches neither a
    /// fill nor a drain.
    #[access(WorldOnly)]
    fn energy_amount(&self, at: BlockSide) -> Option<u32>;
    /// Side: server.
    /// The capacity of the buffer at `at` (`native_energy_amount`), 0 where none is.
    #[access(WorldOnly)]
    fn energy_capacity(&self, at: BlockSide) -> u32;
    /// Side: server.
    /// How many tanks of the block at `at` in the level in place its side reaches
    /// (none: every tank), for the game's own fluid capability: a mod block's by its table, a
    /// cauldron's one. None where no tanks are.
    #[access(WorldOnly)]
    fn fluid_tank_count(&self, at: BlockSide) -> Option<usize>;
    /// Side: server.
    /// What tank `tank` of those `at`'s side reaches holds (`native_fluid_tank_count`).
    #[access(WorldOnly)]
    fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack>;
    /// Side: server.
    /// How much of `fluid` tank `tank` of those `at`'s side reaches holds when empty: a mod
    /// block's fillable tank its capacity, a cauldron a bucket; none for another tank, a
    /// drain-only one, or a fluid the tanks there don't take: other than water and lava, or
    /// with components into a mod block's tanks.
    #[access(WorldOnly)]
    fn fluid_tank_capacity(&self, at: BlockSide, tank: usize, fluid: &FluidStack) -> u32;
}
/// The handles' writes of the `fluids_energy` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait FluidsEnergyBackendMut {
    /// Side: server.
    /// `SimpleEnergyHandler.insert`: puts up to `amount` into the buffer at `at`
    /// (`native_energy_amount`) through its side, no more than its room and its limit per call,
    /// kept by transaction `tx`. How much went in.
    #[access(WorldOnly)]
    fn energy_insert(&mut self, at: BlockSide, amount: u32, tx: &Transaction) -> u32;
    /// Side: server.
    /// `SimpleEnergyHandler.extract`: takes up to `amount` out of the buffer at `at`
    /// (`native_energy_amount`) by its side, no more than it holds and its limit per call, kept
    /// by transaction `tx`. How much came out.
    #[access(WorldOnly)]
    fn energy_extract(&mut self, at: BlockSide, amount: u32, tx: &Transaction) -> u32;
    /// Side: server.
    /// Puts as much of `fluid` as tank `tank` of those `at`'s side reaches takes: a mod
    /// block's fillable tank, empty or holding that fluid, up to its capacity; a cauldron by
    /// whole thirds, lava only whole into an empty one, discarding the fluid's components.
    /// What didn't fit. The write is kept by
    /// transaction `tx`.
    #[access(WorldOnly)]
    fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        tx: &Transaction,
    ) -> Option<FluidStack>;
    /// Side: server.
    /// Takes up to `max` out of tank `tank` of those `at`'s side reaches: a mod block's
    /// drainable tank, a cauldron by whole thirds and lava only whole. What came out. The
    /// write is kept by transaction `tx`.
    #[access(WorldOnly)]
    fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        tx: &Transaction,
    ) -> Option<FluidStack>;
}
