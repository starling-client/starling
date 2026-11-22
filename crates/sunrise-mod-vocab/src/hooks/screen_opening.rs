//! The `screen_opening` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Gui.setScreen` is about to replace the `current` screen with `new`
    /// (`ScreenEvent.Opening`; the screen `playing` is none: vanilla's null).
    /// Answer `ScreenOpening::Cancel` to keep the current one,
    /// `ScreenOpening::ModScreen` to open a mod's own screen instead, or
    /// `ScreenOpening::Vanilla` to open another vanilla screen instead.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "replace",
        out = "outcome",
        answered_by = "mod",
        neoforge = "ScreenEvent.Opening",
        cancellable = true
    )]
    fn screen_opening(
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        outcome: &mut Option<Answered<ScreenOpening>>,
    );
}
