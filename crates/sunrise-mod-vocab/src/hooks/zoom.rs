//! The `zoom` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0018

pub trait Hooks {
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_zoom",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "replace",
        out = "zoom",
        neoforge = "none (Sunrise-only; ViewportEvent.ComputeFov is the nearest way to change the view)"
    )]
    fn zoom(zoom: &mut Option<mod_api::Zoom>);
}
