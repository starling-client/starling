//! The `use_item_on_block` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `UseItemOnBlockEvent`: after a phase was chosen, before its Part or
    /// vanilla behaviour. Cancel skips that behaviour and returns
    /// `cancellation_result` (initially `TryWithEmptyHand`).
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "UseItemOnBlockEvent",
        cancellable = true
    )]
    fn use_item_on_block();
}
