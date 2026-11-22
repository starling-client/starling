//! The `render_gui_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the complete in-world HUD layer walk (NeoForge's `RenderGuiEvent.Post`),
    /// including under F1, and before the open screen draws. Place a box with
    /// `event.corners.place` (boxes stack away from their edge) and draw with `event.draw`.
    /// Use `ClientHost::debug_overlay_visible()` to avoid drawing over the debug overlay.
    /// Not heard when a mod canceled `render_gui_pre`. A panicking listener's
    /// drawing and corner placement are both discarded. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 3,
        contract = "listen",
        staged_surface = "draw",
        neoforge = "RenderGuiEvent.Post"
    )]
    fn render_gui_post(draw: &mut dyn DrawContext, corners: &mut Corners);
}
