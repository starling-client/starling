//! The `adjacent_block_path_type` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The optional contribution of the neighbour `event.state` at `event.pos`
    /// (`IBlockExtension.getAdjacentBlockPathType`). `event.path_type` is that
    /// neighbour's own classified type, not the node's type. The readonly host
    /// reads the owning level. `Some` immediately becomes the node's type and
    /// ends the scan, even if equal to its current type; `None` keeps that
    /// neighbour's fluid and vanilla fallback, never suppressing it. Vanilla
    /// water contributes through its classified path type: a solid waterlogged
    /// block remains `Blocked`. Custom fluids default to `WaterBorder`. Neighbours
    /// are visited x, then y, then z, each from -1 to 1, excluding the node's
    /// own column. No mob is supplied. These answers run after static
    /// classification; per-search evaluator caches remain.
    /// Diverges from upstream: new hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "replace",
        out = "adjacent",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getAdjacentBlockPathType"
    )]
    fn adjacent_block_path_type(
        pos: BlockPos,
        state: BlockState,
        path_type: mod_api::PathType,
        adjacent: &mut Option<mod_api::PathType>,
    );
}
