//! The `potion_splash` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0322

pub trait Hooks {
    /// A thrown splash or lingering potion applied its hit effects.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_potion_splash",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn potion_splash(item: &str, pos: [f64; 3]);
}
