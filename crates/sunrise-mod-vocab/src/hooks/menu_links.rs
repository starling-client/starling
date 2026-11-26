//! The `menu_links` hook: declarations read by the mod API generator.
pub trait Hooks {
    /// Containers exposed by linked slots of server player `player`'s open mod menu
    /// `menu` (`namespace:path`) at `pos`. The player's state is not swapped in.
    /// Push container positions to `links`, in order. The engine resolves distinct live
    /// lendable containers and refreshes their linked slots in the same tick. Asked each
    /// server tick only for open mod menu types declaring links, and dispatched only to
    /// subscribers. With no subscribers the linked-container tick does no work and
    /// allocates nothing. Silently skips `pos`, duplicate positions, and missing or
    /// non-lendable containers. First accepted occurrence wins, preserving answer order,
    /// with no separate container-count cap.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "links",
        version = 2,
        neoforge = "none (Sunrise mod menus expose linked container slots)",
        cancellable = false
    )]
    fn menu_links(player: u128, menu: &str, pos: [i32; 3], links: &mut Vec<[i32; 3]>);
}
