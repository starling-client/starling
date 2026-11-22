//! The `entities_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0197

pub trait Hooks {
    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entities_unload",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entities_unload(dimension: &str, chunk: [i32; 2], count: usize);
}
