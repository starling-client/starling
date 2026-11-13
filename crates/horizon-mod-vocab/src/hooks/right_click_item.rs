//! The `right_click_item` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `PlayerInteractEvent.RightClickItem`, immediately before `Item.use`.
    /// Cancel skips its Part and vanilla behaviour; its result defaults to
    /// `Pass`. On the client only `Success` ends the hand loop.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.RightClickItem",
        cancellable = true
    )]
    fn right_click_item();
}
