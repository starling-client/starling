//! The `render_gui_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the in-world HUD layer walk (NeoForge's `RenderGuiEvent.Pre`), including under F1.
    /// Return `Flow::Cancel` to skip the walk (including layer Pre and Post), `render_gui_post`
    /// and the HUD chat layer. Focused chat is drawn by its screen, not the layer walk.
    /// Toasts draw after the active screen or overlay and are outside this veto. F1 hides
    /// ordinary toasts and audio stream failure notices, but not other callback error toasts.
    /// Fires before Options over a world. Does not fire during replay or when a
    /// server dialog or the connection screen covers the world HUD.
    /// Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiEvent.Pre",
        cancellable = true,
        version = 4
    )]
    fn render_gui_pre(canceled: &mut bool);
}
