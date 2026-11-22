//! The `level_signals` area: redstone power at a position, and whether a block conducts it. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_signals` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSignalsBackend {
    /// Side: server.
    /// `BlockState.isRedstoneConductor(level, pos)` of the block at `pos`: whether it passes
    /// redstone power through it (a full block that conducts: stone yes; glass, slabs and
    /// pistons' heads no). It takes the level and position as vanilla's call does. None for a
    /// stale handle, a client or an unloaded `pos`; a read never loads a chunk.
    fn level_is_redstone_conductor(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// How strongly the block at `pos` powers the block on its `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`, as a block reads its
    /// neighbour `pos` that lies `direction` from it), 0 to 15. None for another level, a stale
    /// handle or a client. Takes a shared host and opens no transaction.
    fn level_signal(&self, level: LevelRef, pos: [i32; 3], direction: Direction) -> Option<i32>;
    /// Side: server.
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15. None as for `signal`.
    fn level_direct_signal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        direction: Direction,
    ) -> Option<i32>;
    /// Side: server.
    /// The strongest strong power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getDirectSignalTo`), 0 to 15. None as for `signal`.
    fn level_direct_signal_to(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// The strongest power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getBestNeighborSignal`, what a dispenser or a piston reads), 0 to 15.
    /// None as for `signal`.
    fn level_best_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does. False where `signal` is None.
    fn level_has_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> bool;
}
