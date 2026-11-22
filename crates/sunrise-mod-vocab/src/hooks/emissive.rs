//! The `emissive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0002

pub trait Hooks {
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_emissive",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "layers",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)"
    )]
    fn emissive(layers: &mut Vec<(&'static str, &'static str)>);
}
