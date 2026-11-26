//! The `clone_item_stack` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IBlockExtension.getCloneItemStack`, asked by
    /// `ServerGamePacketListenerImpl.handlePickItemFromBlock` on the server.
    /// `player` is the requesting player, available for reads through the Host.
    /// `include_data` is true for a creative player picking with Ctrl. The request
    /// carries no client hit face. Leave `stack` unset to keep the game's item.
    /// Creative breaking, such as a beehive's `playerWillDestroy`, does not ask this query.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "stack",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getCloneItemStack",
        version = 3
    )]
    fn clone_item_stack(
        dimension: LevelDimension,
        pos: BlockPos,
        block: &str,
        state: BlockState,
        include_data: bool,
        player: PlayerRef,
        stack: &mut Option<ItemStack>,
    );
}
