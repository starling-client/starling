//! The `screen_closing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The foreground `screen` is being replaced (`ScreenEvent.Closing`), after
    /// `screen_opening` let it. This includes a prompt or editor replaced by chat,
    /// and an outgoing chat instance even when the new one has the same ID.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Closing",
        event_per_listener = true,
        version = 2,
        cancellable = false
    )]
    fn screen_closing(screen: &ScreenView<'_>);
}
