//! The `contents_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server-only, non-cancellable backing-block storage notice at Container.setChanged / BlockEntity.setChanged, after the write and before existing dirty/comparator updates. The event carries LevelRef, BlockPos and BlockState. Native After::Changed transactions notify each backing block once at the root commit, never on abort or simulation; explicit set_changed invalidates even equal contents. Nested writes to the block whose notice is active still apply but do not re-enter its notice; writes to another block notify normally. Double containers notify each backing component, not the other half merely refreshed for comparators. Vehicle/menu placeholder coordinates are excluded. Replaces the Create port positional contents_changed mixin for depot, package and schematic storage; there is no matching NeoForge bus event.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "none (Container.setChanged has no bus event)",
        cancellable = false
    )]
    fn contents_changed();
}
