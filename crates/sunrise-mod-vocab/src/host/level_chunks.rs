//! The `level_chunks` area: chunks of an open level: handles, status, loading and resending. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_chunks` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelChunksBackend {
    /// Side: server.
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    fn level_chunk_ref(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
    /// Side: server.
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool;
    /// Side: server.
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus>;
    /// Side: server.
    /// Whether the chunk passes vanilla's seeded slime-chunk test, independent of
    /// dimension, height, biome and other spawn conditions. Works for unloaded chunks
    /// in active or parked open levels. None for a stale level, a client or a chunk
    /// outside world bounds. Does not load chunks or advance the world's random stream.
    fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool>;
    /// Side: server.
    /// The sections of a loaded chunk whose block palette may hold any state of `block`
    /// (`PalettedContainer.maybeHas`), as section Y coordinates (`SectionPos.y`, block Y `>> 4`)
    /// from the bottom up, without reading the 4096 blocks of any section. "May": a palette
    /// keeps a state no block of the section uses any more, so read the blocks of each section
    /// named to find the ones that are there, and a section of more distinct states than a
    /// palette holds is named whenever the block exists. The way to find blocks that stand in a
    /// chunk, such as on `chunk_load`. None for a stale handle, on a client, and for a chunk
    /// that isn't loaded; it loads nothing.
    fn chunk_sections_with_block(&self, chunk: ChunkRef, block: BlockKey) -> Option<Vec<i32>>;
    /// Side: server.
    /// `ChunkMap.getPlayers(pos, false)`: the players watching this chunk, those
    /// it is inside the view of and was sent to, in the order the server keeps
    /// its players. A player watches it from its `chunk_watch` until it leaves
    /// the view. Fake players watch nothing. Loaded or not, a chunk nobody
    /// watches completes without a call. A stale handle returns `Stale`; an unreadable side or
    /// backend returns `Unavailable`. Return `Break(())` to stop and return it to the caller.
    fn chunk_visit_watchers(
        &self,
        chunk: ChunkRef,
        visit: &mut dyn FnMut(PlayerRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: both.
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Visit the resident chunks of an open server level by chunk x then z. Loads nothing.
    /// A stale level returns `Stale`; an unreadable side or backend returns `Unavailable`.
    /// Return `Break(())` to stop or `Continue(())` to finish, including for an empty level.
    fn level_visit_loaded_chunks(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(ChunkRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: both.
    /// Visit each column's height by `heightmap` (`LevelReader.getHeight(type, x, z)`) in
    /// input order: one above the highest block that counts on that map, or `None` for a
    /// column whose chunk is not loaded. Never loads or generates chunks.
    ///
    /// Works on active and parked server levels and on the client's current open level.
    /// No column-count cap applies. The visitor can make shared reads on the same host.
    /// Return `ControlFlow::Continue(())` to keep visiting or `ControlFlow::Break(())` to
    /// stop before sampling later columns. Returns `Ok(ControlFlow::Break(()))` if stopped
    /// or `Ok(ControlFlow::Continue(()))` after every input column (including an empty slice).
    /// Returns `Err(LevelReadError::Stale)` for a stale handle or closed level, and
    /// `Err(LevelReadError::Unavailable)` when world reads are unavailable (such as outside
    /// an open world or on a foreign client dimension). Refusals do not call `visit`.
    fn level_visit_surface_heights(
        &self,
        level: LevelRef,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
        visit: &mut dyn FnMut((i32, i32), Option<i32>) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: server.
    /// Visit each column's height on `heightmap`, server-side biome JSON water colour
    /// (`BiomeSpecialEffects.waterColor`, `0xRRGGBB`), and the complete [`BlockState`] values
    /// from `below` blocks under to `above` blocks from that height (bottom first; air outside
    /// the chunk's vertical range). Unloaded columns yield `None` without loading chunks.
    ///
    /// Colormap-derived grass and foliage tints are client-side only: read blended tints on
    /// the client with [`BlockAndTintGetter::block_tint`] in block tint callbacks or
    /// `ClientHost::biome_tint` in other client hooks.
    ///
    /// Works on active and parked server levels. `range` is `(below, above)`; `below + above`
    /// must be `1..=level_height` (the dimension's build height, `LevelHeightAccessor.getHeight`).
    /// No column-count or total-block cap applies: columns stream one at a time, and returning
    /// `ControlFlow::Break(())` stops before sampling later columns. The visitor can make
    /// shared reads on the same host.
    ///
    /// Returns `Ok(ControlFlow::Break(()))` if stopped or `Ok(ControlFlow::Continue(()))` after
    /// all input columns. Returns `Err(LevelReadError::Stale)` for a stale handle or closed
    /// level, and `Err(LevelReadError::Unavailable)` on a client, outside server world access,
    /// when `range` is empty or exceeds the level's build height, or when block-state
    /// conversion is unavailable. Refusals do not call `visit`.
    fn level_visit_columns(
        &self,
        level: LevelRef,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
        range: (u32, u32),
        visit: &mut dyn for<'a> FnMut(
            (i32, i32),
            Option<ColumnSlice<'a>>,
        ) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, LevelReadError>;
    /// Side: both.
    /// Captures an immutable, thread-safe snapshot of this loaded chunk's sections (blocks, 3D
    /// quart biomes, sky/block light) and heightmaps, usable off-thread without holding a `Host`.
    ///
    /// Block entities and entities are not included in the snapshot. Capture shares the chunk's
    /// copy-on-write section allocations with no array copies and no count cap; holding a snapshot
    /// pins those sections in memory after unload and causes the live chunk's next block, biome,
    /// or light edit to any still-shared section to copy that section before mutating it.
    ///
    /// Returns `None` if the handle is stale or the chunk is not currently resident in its level
    /// (works on open server levels, active or parked, and on a logged-in client's loaded level).
    fn chunk_snapshot(&self, chunk: ChunkRef) -> Option<ChunkSnapshot>;
    /// Side: server.
    /// Returns a thread-safe handle for reading **last saved** chunks from this level's
    /// Anvil storage off the server tick thread, without loading them into the live world.
    ///
    /// Returns `None` on a client, for a stale handle, or for an unsaved/ephemeral level.
    fn level_saved_chunk_reader(&self, level: LevelRef) -> Option<SavedChunkReader>;
    /// Side: server.
    /// Returns a thread-safe detached generator bound to this level's seed, dimension type, data
    /// packs, and mod worldgen hooks, callable off the server tick thread without mutating the
    /// live world or save directory.
    ///
    /// Returns `None` on a client, for a stale handle, or when the level has no generator.
    fn level_detached_generator(&self, level: LevelRef) -> Option<DetachedChunkGenerator>;
}
/// The handles' writes of the `level_chunks` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelChunksBackendMut {
    /// Side: server.
    /// Queues the loaded chunk of any open server level (active or parked) to go whole again
    /// with its light, block entities and biomes when that level broadcasts changes after the
    /// column's light work pending at the call, including any whole-column jobs those edits need
    /// (`ChunkMap.waitForLightBeforeSending`). Only current holders receive it, including in a viewed
    /// level (`ChunkMap.getPlayers`, `PlayerChunkSender`). No holders at the time of the call means
    /// `Ok(())` with nothing queued.
    /// Each refusal queues nothing and is named by `ChunkRefusal`: `Stale` (stale handle or
    /// unloaded level), `Client` and `NotLoaded`.
    fn chunk_resend(&mut self, chunk: ChunkRef) -> Result<(), ChunkRefusal>;
    /// Side: server.
    /// Load chunk `pos` (`[x, z]`, in chunks) of the level from its save, or generate it, to
    /// FULL, as vanilla's `Level.getChunk(x, z)` does (`ServerChunkCache.getChunk(x, z,
    /// ChunkStatus.FULL, true)`), then read or write it with the level's other calls.
    /// `Ok` once the chunk is full, its initial light done; a chunk already full answers `Ok` at
    /// once. A chunk among the level's four most recent FULL lookups (this
    /// call's, or a resident read's such as `host.chunk`), cleared at the end of every tick of
    /// the level's chunks, answers with no ticket (`ServerChunkCache.getChunk`'s cache).
    /// To light it, the 8 chunks around it are loaded too (vanilla only generates them part of
    /// the way); they hold no ticket and unload with the next unload sweep.
    ///
    /// **The call blocks the server tick until the chunk is loaded or generated**, as
    /// vanilla's does: a chunk that has never been generated can take a long while, so load the
    /// chunks a mod needs ahead of time rather than many in one tick. There is no timeout, as in
    /// vanilla; the call gives up with `ChunkLoadError::Unloaded` if the level closes meanwhile,
    /// and with `ChunkLoadError::Failed` once generation panics for the chunk or one its generation needs around it (a
    /// mod's chunk generator: vanilla would crash, Sunrise contains it and reports it).
    ///
    /// Like vanilla, the call places an `UNKNOWN` ticket at FULL's level on the chunk: it keeps
    /// the chunk loaded, not ticking, through the next ticket purge and expires at the one after
    /// (`TicketType.UNKNOWN`'s one-tick timeout), after which the chunk unloads as any other
    /// with no ticket. Depending on when in the tick the call runs, the first purge can come in
    /// the same tick. Nothing is forced or saved: to keep a chunk, use `ChunkRef::set_forced`
    /// or a `TicketController`'s ticket.
    ///
    /// Works in any open level, not only the one in place. Loading a chunk of a parked level
    /// does not put that level in place: block writes that need the level in place
    /// (`set_block`) still answer `LevelWriteError::Unavailable` there. The `chunk_load`
    /// listeners of each chunk the call brings in run during the call, while that chunk is
    /// loaded and before the call returns, as NeoForge's `ChunkEvent.Load` does inside
    /// `getChunk`. `Unloaded`, with no ticket, for a
    /// stale handle, on a client, for a level the server has not opened, for a chunk past
    /// `ChunkPos.isValid` (beyond `ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE` on x or z, where
    /// vanilla's block reads and writes stop, `Level.isInValidBounds`, and vanilla's future is
    /// `ChunkHolder.UNLOADED_CHUNK`), and while the level's generation is paused or stopped.
    fn level_load_chunk(&mut self, level: LevelRef, pos: [i32; 2]) -> Result<(), ChunkLoadError>;
    /// Side: server.
    /// Enqueues a non-blocking load/generation request for chunk `pos` (`[x, z]`, in chunks) to
    /// reach `ChunkLevel.FULL_CHUNK_LEVEL` (level 33 — accessible, not ticking: no entity ticks,
    /// no random ticks, no block or block-entity ticks, no natural spawning), using the level's
    /// existing chunk streaming worker pool.
    ///
    /// - No request count cap (matching vanilla `ServerChunkCache.getChunkFuture`).
    /// - Does not block the server tick; player view and `/forceload` chunks always sort ahead of
    ///   mod async requests.
    /// - Completions are captured on the tick thread during spare tick time (`server_has_time()`),
    ///   so large batches cannot stall a tick.
    /// - Once the chunk is captured into a [`ChunkSnapshot`] (or cancelled), its transient
    ///   level-33 ticket is released and the chunk unloads and saves through vanilla's ordinary
    ///   unload/save path.
    /// - Pending and undrained requests owned by a mod are dropped (and their tickets released)
    ///   automatically if that mod is disabled or the level closes.
    ///
    /// Returns `Err(ChunkLoadError::Unloaded)` for a stale handle, client, closed or paused
    /// level, outside a mod callback, or out-of-bounds `pos` (`!ChunkPos.isValid`), or
    /// `Err(ChunkLoadError::Failed)` if generation previously failed at `pos`.
    fn level_request_chunk(
        &mut self,
        level: LevelRef,
        pos: [i32; 2],
    ) -> Result<ChunkRequestRef, ChunkLoadError>;
    /// Side: server.
    /// Drains every chunk request owned by the calling mod on this level that has completed or
    /// failed since the previous drain call. Never blocks; returns `Some(Vec::new())` when no
    /// requests have completed yet, or `None` if the level handle is stale, closed, on a client,
    /// or called outside a mod callback.
    fn level_drain_chunk_requests(&mut self, level: LevelRef)
    -> Option<Vec<CompletedChunkRequest>>;
    /// Side: server.
    /// Cancels a pending (or undrained) request owned by the calling mod and releases its
    /// level-33 ticket immediately. Returns `true` if the request was still active.
    #[manual_handle]
    fn chunk_request_cancel(&mut self, request: ChunkRequestRef) -> bool;
}
