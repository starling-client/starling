//! The `level_block_updates` area: telling neighbours, clients and capability caches that a block changed. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_block_updates` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelBlockUpdatesBackend {
    /// Side: server.
    /// The capability generation at `pos`: a number that changes when a provider says its
    /// answer there changed ([`LevelRef::invalidate_capabilities`]) and when the chunk of a
    /// position a provider invalidated unloads, and never comes back to a number it was while
    /// the level is open. [`BlockCapabilityCache`] compares it with the block entity and the
    /// block state there. Only the change means anything, not the number. Reads parked
    /// levels. None for a stale handle, a client, an unloaded position and one outside build
    /// height. Constant time, no allocation.
    fn level_capability_generation(&self, level: LevelRef, pos: [i32; 3]) -> Option<u64>;
}
/// The handles' writes of the `level_block_updates` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelBlockUpdatesBackendMut {
    /// Side: server.
    /// Tells kept capabilities at `pos` that a provider's answer there changed with no change
    /// of block or block entity (NeoForge's `invalidateCapabilities`), such as a machine whose
    /// side configuration changed: the capability generation there moves on
    /// ([`LevelRef::capability_generation`]), so a [`BlockCapabilityCache`] there looks again
    /// at its next `get`. Inside a transaction it happens at the root commit, and an aborted
    /// one changes nothing. Block and block entity changes, chunk unloads and mod switches
    /// need no call: the cache sees them itself. False, with nothing changed, off the level in
    /// place, for a stale handle, on a client and for a position that isn't loaded.
    fn level_invalidate_capabilities(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Send `ServerLevel.destroyBlockProgress` in this level: stages 0..9 show a crack,
    /// any other stage removes it. A machine uses a stable negative breaker id.
    /// Inside a transaction, notices wait for the root commit in call order; abort publishes
    /// no notice. Outside a transaction this executes immediately. True means accepted or
    /// queued, not already delivered.
    fn level_set_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool;
    /// Side: server.
    /// Tell the clients the block at `pos` changed (`Level.sendBlockUpdated`): its state and
    /// block entity data are sent again at the end of the tick, without writing anything, for
    /// a mod that changed what a block shows by other means than a block write. False, with
    /// nothing queued, for a stale or parked handle, a client, and an unloaded or out-of-height
    /// position. True with no player to notify (no one is in the level) when the position is
    /// valid: the chunk is marked changed and nothing is sent.
    /// Inside a transaction this waits for the root commit; abort publishes no notice.
    /// Outside a transaction this executes immediately. True means accepted or queued,
    /// not already delivered.
    fn level_send_block_updated(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Queues block event `a`, `b` for `block` at `pos`, as `ServerLevel.blockEvent(pos, block,
    /// a, b)` does: after this tick's scheduled ticks the level runs the block's
    /// `triggerEvent` with them (a powered piston's 0 extends it, a note block's plays its
    /// note, a bell's 1 rings it) if `pos` still holds `block`, and a handled event reaches the
    /// players' copy of the block. The same event queued twice runs once; one in a chunk
    /// that isn't ticking waits for it. True means queued, not handled. False for a block
    /// key no block has, a position outside the level's build height, a parked level, a stale
    /// handle or a client.
    fn level_block_event(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        a: i32,
        b: i32,
    ) -> bool;
    /// Side: server.
    /// Tell the neighbours of `pos` that a block changed there, as vanilla's own writes do:
    /// `Neighbours` (`Level.updateNeighborsAt`: a lever's or a button's own block, and the one
    /// it is attached to, when the signal it gives changes), `NeighboursExcept(direction)`
    /// (`updateNeighborsAtExceptFromFacing`: a diode's front) or `OutputSignal`
    /// (`updateNeighbourForOutputSignal`: the comparators beside it, when its comparator output
    /// changes without its state changing). `source` is the state whose block vanilla passes as
    /// the one that changed. Typed errors refuse stale/unavailable levels or invalid states
    /// without mutation; Ok(false) means an unloaded or out-of-height position.
    fn level_update_neighbors(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockState,
        update: NeighbourUpdate,
    ) -> Result<bool, LevelWriteError>;
    /// Side: server.
    /// Tell the block at `pos` that `source`'s block beside it changed (`Level.neighborChanged`),
    /// as a diode tells the block in front of it before updating that block's neighbours.
    /// Typed errors refuse stale/unavailable levels or invalid states; Ok(false) means an
    /// unloaded or out-of-height position. Errors never mutate the level.
    fn level_neighbor_changed(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockState,
    ) -> Result<bool, LevelWriteError>;
}
