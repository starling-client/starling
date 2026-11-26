//! The `screen_init_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A screen initialized or rebuilt its children (`ScreenEvent.Init.Post`, vanilla's
    /// `Screen.init` / `Screen.rebuildWidgets`). Use `event.widgets` to list and remove children
    /// or mount buttons, text fields, and custom widgets. A layout-only resize keeps the existing
    /// children and focus and does not fire this hook. A new chat instance initializes once even
    /// when it replaces one with the same ID and rectangle. A vetoed opening does not initialize
    /// a screen. A panicking listener publishes no edits. The pause and title screens list their
    /// own controls first, in vanilla's order.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 4,
        contract = "listen",
        staged_surface = "widgets",
        neoforge = "ScreenEvent.Init.Post",
        event_per_listener = true,
        cancellable = false
    )]
    fn screen_init_post(screen: &ScreenView<'_>, widgets: &mut dyn ScreenWidgets);
}
