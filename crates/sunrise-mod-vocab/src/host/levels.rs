//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `levels` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait LevelsHost {
    /// Side: both.
    /// Whether a mod with the `sunrise-mod.toml` id `id` is installed: a
    /// native or content-only package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    #[static_call]
    fn is_loaded(&self, id: &str) -> bool;
    /// Side: both.
    /// Find a loaded chunk in an open server level (`ServerChunkCache.getChunkNow`) or in a
    /// logged-in client's loaded level. On a server, a FULL chunk found becomes one of the
    /// level's recent FULL lookups, so a `level.load_chunk` of it before the end of the level's
    /// next chunk tick places no ticket.
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
}
/// The handles' client writes of the `levels` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelsClientBackendMut {
    /// Side: client.
    /// Show or clear the crack of `breaker` on the block at `pos` in this client's own view
    /// (`LevelRenderer.destroyBlockProgress`): stages 0..9 show a crack, any other stage removes
    /// it. Nothing is sent to the server or to other players, and a crack not updated for 400
    /// ticks goes away. False for a stale or foreign level or before login.
    ///
    /// ```compile_fail
    /// use mod_api::{LevelRef, ServerHost};
    /// fn wrong_side(level: LevelRef, host: &mut dyn ServerHost) {
    ///     level.set_client_break_progress(host, -1, [0, 64, 0], 3);
    /// }
    /// ```
    fn level_set_client_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool;
}
