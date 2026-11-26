//! The `render_gui_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the complete in-world HUD layer walk (NeoForge's `RenderGuiEvent.Post`),
    /// including under F1, and before screens, toasts, debug overlays and IME draw.
    /// Use `event.draw` to draw in GUI pixels and `ClientHost::hud_layer_bounds` to
    /// read the clipped bounds of earlier layers (e.g. `"minecraft:effects"`).
    /// Use `ClientHost::debug_overlay_visible()` to leave room for the debug overlay.
    /// Skipped during replay, when there is no in-world HUD pass, or when a mod
    /// cancels `render_gui_pre`. A panicking listener's drawing is discarded. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 4,
        contract = "listen",
        staged_surface = "draw",
        neoforge = "RenderGuiEvent.Post"
    )]
    fn render_gui_post(draw: &mut dyn DrawContext);
}
