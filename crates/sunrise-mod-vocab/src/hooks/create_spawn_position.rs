//! The `create_spawn_position` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a new server level selects its initial world spawn
    /// (NeoForge's `LevelEvent.CreateSpawnPosition`), after the Overworld's
    /// `level_load` and before the remaining configured levels are opened.
    /// The vanilla Debug preset uses its fixed spawn and skips this hook.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LevelEvent.CreateSpawnPosition",
        cancellable = true,
        version = 2
    )]
    fn create_spawn_position(position: &mut [i32; 3], rotation: &mut [f32; 2], cancel: &mut bool);
}
