//! The `item_picked_up` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0050

pub trait Hooks {
    /// `player` is about to take `count` of `item` from the ground
    /// (`ItemEntity.playerTouch`). Set `canceled` to leave it there: the
    /// stack isn't offered again for 10 ticks. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemEntityPickupEvent.Pre"
    )]
    fn item_picked_up();
}
