//! The `level_save` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The end of `dimension`'s level save pass (NeoForge's `LevelEvent.Save`
    /// in `ServerLevel.save`), for autosave, `/save-all`, pause and shutdown.
    /// Individual record failures are reported and do not suppress this hook;
    /// a failed awaited storage flush does. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Save",
        paper = "WorldSaveEvent"
    )]
    fn level_save(dimension: &str);
}
