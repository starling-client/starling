//! The `is_pathfindable` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Changes whether a path of a `PathComputationType` (`Land`, `Water` or `Air`) may go through a block state. `event.pathfindable` starts with the block's own answer. The answer depends on the state and the type alone: the game asks once per state and type when parts resolve, at world load and on every `/reload`, so path finding and spawning read the stored answer and a part cannot answer from a position, level or time.
    #[hook(
        side = "server",
        contract = "collect",
        out = "pathfindable",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "none (vanilla BlockBehaviour.isPathfindable; NeoForge 26.3 has no IBlockExtension.isPathfindable, IBlockExtension.getBlockPathType answers a different question)"
    )]
    fn is_pathfindable();
}
