//! The `level_ticking` area: what the level ticks now, and running a tick by hand. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_ticking` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTickingBackend {
    /// Side: server.
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    fn level_in_place(&self, level: LevelRef) -> bool;
    /// Side: server.
    /// `Some(true)` when this resident, in-bounds position has entity-ticking tickets.
    /// `Some(false)` for unloaded or out-of-bounds positions or tickets that forbid ticking.
    /// With no ticket system, resident in-bounds chunks tick; an active but empty ticket
    /// system permits none. This reads the current ticket decision, not a tick guarantee.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count;
    /// it does not allocate a memo table or refresh a resident-chunk cache.
    fn level_entity_ticking(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `Some(true)` when scheduled blocks, block events and block entities can tick here:
    /// a resident, in-bounds position with block-ticking tickets and no pending entities.
    /// `Some(false)` for unloaded or out-of-bounds positions, tickets that forbid ticking,
    /// or a chunk waiting for its entities. With no ticket system, resident in-bounds
    /// chunks with no pending entities tick; an active but empty ticket system permits none.
    /// `None` for stale handles, a client or a level not currently in place. Parked
    /// levels cannot answer because the live ticket state belongs to the level in place.
    /// Does not load chunks. Each call shares the ticket map through a reference count
    /// and borrows the pending-entity set without copying it or allocating a memo table.
    fn level_should_tick_blocks_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
}
/// The handles' writes of the `level_ticking` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelTickingBackendMut {
    /// Side: server.
    /// Run the block at `pos`'s scheduled tick now (`ServerLevel.tickBlock`): the path a due
    /// tick from the queue takes, the block's part first, else `BlockState.tick`, with the same
    /// random draws and level events. The queue is not read or changed; `schedule_block_tick`
    /// queues one. False for a refused write, a stale or parked handle, a client and an
    /// unloaded or out-of-height position.
    fn level_block_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Random-tick `pos` now, as `ServerLevel.tickChunk` does for a position it picked: the
    /// block's `randomTick` when it ticks randomly (its part first), then lava's fluid random
    /// tick, with the same random draws and level events. False as for `block_tick`.
    fn level_block_random_tick(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Lend the named level's live stream to `draw`. Run no callback when the level is
    /// unavailable. See [`LevelRef::with_random`].
    #[decline_unavailable]
    fn with_level_random(&mut self, level: LevelRef, draw: RandomDraw<'_>);
}
