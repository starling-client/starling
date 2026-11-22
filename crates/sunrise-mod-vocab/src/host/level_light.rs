//! The `level_light` area: stored and sky light, and relighting. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_light` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelLightBackend {
    /// Side: both.
    /// `Level.getSkyDarken`: the sky-light reduction from dimension, time and weather.
    /// Reads any loaded server level, including parked levels, or the current client level.
    /// None for a stale or unavailable level. Does not load chunks.
    fn level_get_sky_darken(&self, level: LevelRef) -> Option<i32>;
    /// Side: both.
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// None as for `block_state`: a position that isn't loaded has no light to read. On a
    /// client, the light of the level it shows, as the server sent it for a chunk it holds.
    fn level_brightness(&self, level: LevelRef, layer: LightLayer, pos: [i32; 3]) -> Option<i32>;
    /// Side: both.
    /// `IBlockStateExtension.getLightEmission`: the source light of `state` at `pos`, not
    /// the propagated light there. The state need not occupy `pos`. A dynamic state's Part
    /// answer is returned unchanged; a static state or a Part that declines uses the state's
    /// declared emission. No chunks are loaded or relit. A stale handle returns `Stale`;
    /// an unavailable level, host view or state returns `Unavailable`. If a light-emission
    /// provider for this state is already active on this thread in this game, the read returns
    /// `Unavailable` even at a different level or position. Delegation to another state is
    /// allowed. `Ok(0)` is zero emission.
    fn level_get_light_emission(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
    ) -> Result<u8, LevelReadError>;
    /// Side: both.
    /// `BlockAndLightGetter.canSeeSky`: whether the level's published sky light at `pos` is 15.
    /// Works in an open server level or the current client level; stale or unavailable levels
    /// return None. Never loads chunks. See `LevelHost::can_see_sky` for the light storage rules
    /// at its boundaries.
    fn level_can_see_sky(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
}
/// The handles' writes of the `level_light` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelLightBackendMut {
    /// Side: server.
    /// Check the block light at `pos` again (`LevelLightEngine.checkBlock`), at the level's
    /// next light update, as an edit there is checked; checking the same position again before
    /// then queues nothing. A block declaring `dynamic_light` is asked `light_emission` again
    /// first: the light engine never asks on its own, so a part whose answer reads more than
    /// the block's state (the block below, the depth) calls this when that changes. Clients take the new light from the server's light update and ask
    /// their own parts again. False, with nothing queued, off the level in place, for a stale
    /// handle, on a client and for a position that isn't loaded.
    fn level_check_light(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Queues the loaded chunks `chunks` (chunk coordinates) of the level in place, and the loaded
    /// chunks around them, to be lit again whole from their blocks, for a mod that wrote blocks by
    /// other means (`edit_blocks` queues the same relight for its own chunks). Accepted at once and
    /// never lit during the call: the light jobs run at the level's next light update, as those of `edit_blocks` and
    /// `level_check_light` do, and a column whose eight
    /// neighbours are not all loaded waits until they are. The light that changed goes to the
    /// players who see that chunk's border, as any relight's does. There is no completion event
    /// and no per-call count of finished chunks: a mod that needs the new light reads it on a
    /// later tick (`level_brightness`), and `chunk_resend` sends the chunk with whatever light has
    /// landed by the end of that tick. Returns how many columns were queued, the ring included; a
    /// column not loaded, or already waiting in the queue, is not counted. The queue is bounded
    /// (4096 columns in all, and 256 chunks a call): a call that would pass either is refused
    /// whole with `TooManyChunks`, whose `max` is how many chunks a call may take now. Each
    /// refusal queues nothing and is named by `ChunkRefusal`: `Stale`, `Client`, `NotInPlace` (a
    /// level that is not the one being ticked) and `TooManyChunks`.
    fn level_relight_chunks(
        &mut self,
        level: LevelRef,
        chunks: &[[i32; 2]],
    ) -> Result<u32, ChunkRefusal>;
}
