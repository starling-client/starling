//! The `screen_opening` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Gui.setScreen` is about to replace the `current` screen with `new`
    /// (`ScreenEvent.Opening`; the screen `playing` is none: vanilla's null).
    /// Answer `ScreenOpening::Cancel` to keep the current one,
    /// `ScreenOpening::ModScreen` to open a mod's own screen instead, or
    /// `ScreenOpening::Vanilla` to open another vanilla screen instead.
    /// A new chat screen instance can replace another chat screen; `current`
    /// is the foreground screen, including a prompt or editor it will replace.
    /// The waiting screen and a warning's live return dialog can replace a
    /// receiver with the same `dialog` id. `Cancel` keeps its mounted widgets
    /// and focus, without emitting Closing or the replacement's Init.Post.
    /// A server's container close that leaves no screen fires no Opening and
    /// can't be cancelled; `screen_closing` still gets the screen it closes.
    /// One that opens the title or the death screen fires Opening for it.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "replace",
        out = "outcome",
        answered_by = "mod",
        neoforge = "ScreenEvent.Opening",
        event_per_listener = true,
        version = 2,
        cancellable = true
    )]
    fn screen_opening(
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        outcome: &mut Option<Answered<ScreenOpening>>,
    );
}
