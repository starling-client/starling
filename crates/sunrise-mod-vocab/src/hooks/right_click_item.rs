//! The `right_click_item` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `PlayerInteractEvent.RightClickItem`, immediately before `Item.use`.
    /// Cancel skips its Part and vanilla behaviour; its result defaults to
    /// `Pass`. `Success`, `SuccessServer` and `Consume` end the client's
    /// hand loop. `Success` swings on the client, `SuccessServer` on the
    /// server, and `Consume` has no swing.
    /// A client without this mod predicts vanilla here; the server corrects it.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.RightClickItem",
        cancellable = true
    )]
    fn right_click_item(
        click: &ItemClick<'_>,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    );
}
