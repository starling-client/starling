//! The `map_color` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The map colour the block `state` at `pos` in `dimension` shows on a map
    /// (`IBlockExtension.getMapColor`, which `BlockState.getMapColor` asks): set `color` to a
    /// `MapColor` id. `color` starts with the state's own colour, `StateFacts.map_color`. `MapItem.update`
    /// asks it at four places: the dirt (10) and stone (11) noise of a level with a ceiling,
    /// asked of the default states of those blocks at `BlockPos::ZERO`; every step of the scan
    /// down from the surface, where 0 (`NONE`) keeps the scan going; and the final colour of the
    /// scanned block, or of its fluid's block when the fluid is not sturdy on top. An answer
    /// above 61 is not a `MapColor` id and is refused: the game logs it once per block, keeps
    /// the colour the hook started with, and never panics. Asked on a server only, with the
    /// level read-only. With no part attached to the state, the cost is one table lookup.
    /// Diverges from upstream: new hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "color",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getMapColor"
    )]
    fn map_color(dimension: LevelDimension, pos: BlockPos, state: BlockState, color: &mut u8);
}
