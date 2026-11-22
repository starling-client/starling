//! The `player_destroy_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0250

pub trait Hooks {
    /// The player's item was used up or broke. `original` is the complete
    /// stack before the use or durability loss, including count, damage and
    /// components. `hand` is the hand it was in, or None for worn equipment
    /// and ammunition consumed from the inventory. `player` and
    /// `player_entity_id` identify the acting player, including the local host.
    /// Listen only, after consumption or the break has applied and before a
    /// use remainder is installed. A mining break notifies after block drops.
    /// Both sides hear their own actions; the client does not report a break
    /// inferred from an equipment status packet. Partial consumption, refused
    /// actions and creative use that preserves the stack do not fire.
    #[hook(
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "listen",
        neoforge = "PlayerDestroyItemEvent"
    )]
    fn player_destroy_item(
        player: u128,
        player_entity_id: i32,
        original: &ItemStack,
        hand: Option<Hand>,
    );
}
