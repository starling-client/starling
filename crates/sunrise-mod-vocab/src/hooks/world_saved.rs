//! The `world_saved` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0344

pub trait Hooks {
    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_world_saved",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_saved(wait: bool);
}
