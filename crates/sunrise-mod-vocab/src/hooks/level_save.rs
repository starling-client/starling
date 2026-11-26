//! The `level_save` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `dimension`'s level was saved (NeoForge's `LevelEvent.Save`, in
    /// `ServerLevel.save`): its level data, chunks and entities, by the
    /// autosave, `/save-all`, a pause, a closing level or the server
    /// stopping, and the integrated server's initial save before
    /// `server_starting`. A world-wide save visits levels in their creation
    /// order, regardless of which level is active. Idle chunk eviction is
    /// not a level save. Individual record failures are reported and do not
    /// suppress this hook; a failed awaited storage flush does. Server-side.
    ///
    /// On a full server save the placed players are saved before any level,
    /// and world metadata is saved after the levels. The integrated server's
    /// initial save has no placed owner and does not write the owner's player file.
    /// A new world's metadata still names the profile when known.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Save",
        paper = "WorldSaveEvent",
        version = 2
    )]
    fn level_save(dimension: &str);
}
