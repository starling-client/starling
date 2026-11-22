//! The `render_gui_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the in-world HUD layer walk (NeoForge's `RenderGuiEvent.Pre`), including under F1.
    /// Return `Flow::Cancel` to skip the walk (including layer Pre and Post), `render_gui_post`
    /// and ordinary toasts. Focused chat is drawn by its screen, not the layer walk.
    /// Callback error toasts are outside this veto and remain visible under F1. Audio stream
    /// failure notices are outside this veto too, but the player can hide them with F1.
    /// Does not fire when a server dialog or the connection screen covers the world HUD.
    /// Client-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiEvent.Pre",
        cancellable = true,
        version = 3
    )]
    fn render_gui_pre(canceled: &mut bool);
}
