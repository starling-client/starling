//! The `level_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0139

pub trait Hooks {
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A custom level closes once nobody has
    /// been in it for 300 ticks, after its chunks' unloads; built-in levels
    /// (Overworld, Nether, End) stay open until server shutdown, unloading
    /// only their unticketed chunks while idle. Every level still open closes
    /// when the server stops, before `WorldCloseFinal`, and when its open or
    /// its loop fails, before `WorldOpenFailed` or `WorldCloseFailed`. Once
    /// for each `level_load`. Don't read its blocks. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_level_unload",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false,
        version = 2
    )]
    fn level_unload(dimension: &str);
}
