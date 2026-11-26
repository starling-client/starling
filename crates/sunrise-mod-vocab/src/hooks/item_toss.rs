//! The `item_toss` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ItemTossEvent`: `player` tosses the item entity `item` (`Player.drop`: the drop key, a menu's throw, what no longer fits in the inventory), not what scatters when they die. The entity is made, with its thrower, but has not joined the level yet: its handle reads and changes the whole stack, the pickup delay and the target, and those changes stay. Set `canceled` and it never joins the level; the items are already out of the inventory and are lost. A canceled toss awards no drop statistic, as vanilla awards one only when an item entity came of it. Server-side. Diverges from upstream: replaces the listen-only `item_dropped`, which fired for the drop key and menu throws only.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemTossEvent",
        cancellable = true,
        version = 2
    )]
    fn item_toss(player: u128, item: ItemEntityRef, canceled: &mut bool);
}
