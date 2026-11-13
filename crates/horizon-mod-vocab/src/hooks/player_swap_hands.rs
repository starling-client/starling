//! The `player_swap_hands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0262

pub trait Hooks {
    /// `player` (`name`) pressed the swap-hands key: heard before the main and
    /// off hand items swap (the `SWAP_ITEM_WITH_OFFHAND` player action that
    /// `ServerGamePacketListenerImpl.handlePlayerAction` answers; Bukkit's
    /// `PlayerSwapHandItemsEvent`), not for a spectator. Set `cancel` to keep
    /// the items where they are: the player's client, which swapped them at
    /// once, gets its slots back. The player's state is swapped in, so
    /// `host.players().acting_player()`, `PlayerRef::tell` and the item helpers reach them.
    /// Once for each press, on a server. Not heard: the same swap made in an
    /// open inventory (hovering a slot and pressing the key, a menu click) and
    /// the host of a LAN world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingSwapItemsEvent.Hands"
    )]
    fn player_swap_hands();
}
