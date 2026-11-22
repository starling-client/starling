//! The `thunder_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0335

pub trait Hooks {
    /// The server changed its thunder state through `/weather`. Set `cancel`
    /// to keep the previous thunder state and timer. Server-side.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_thunder_change",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn thunder_change(dimension: &str, thundering: bool, cancel: &mut bool);
}
