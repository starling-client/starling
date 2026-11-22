//! The `contents_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server-only, non-cancellable backing-block notice at `Container.setChanged` /
    /// `BlockEntity.setChanged`, after the write and before dirty/comparator updates. Default
    /// `ITEMS` and item `STORAGE` handlers coalesce implicit notices per backing block at root
    /// commit, for vanilla containers and engine-managed mod slots. Notices committed during
    /// `menu_quick_move` wait until that hook's accepted source remainder is in place (or until
    /// the hook finishes without one). If the original backing container was removed or
    /// replaced, neither its replacement nor watchers receive the deferred notice. Explicit
    /// `set_changed` notifies immediately, even for equal contents and inside a transaction;
    /// the notice survives abort.
    /// Direct `set_item_slot`, writes to storage a mod provides itself, and rollback restores
    /// synthesize no notice. Nested writes to the backing block whose notice is active still
    /// apply, with dirty/comparator updates, but do not re-enter or queue a second notice for
    /// that block. Another block notifies normally. Double containers notify each backing
    /// component, not the other half merely refreshed for comparators.
    /// Vehicle/menu placeholder coordinates are excluded. The source level, pos and state
    /// always name the changed backing block. `observer` is `None` for its own Parts, or `Some`
    /// for Parts of a block entity watching that source through `watch_contents`. Source Parts
    /// run first, then a snapshot of observers in registration order; a stale observer is skipped.
    /// A self-watch never duplicates the source callback. Registration changes affect the next
    /// notice. Replaces the Create port positional `contents_changed` mixin for depot, package
    /// and schematic storage; there is no matching NeoForge bus event.
    /// Each Part runs in the existing storage journal. A panic discards that Part's tentative writes (including successful nested transactions), not the backing change that raised the notice or another Part's writes.
    #[hook(
        dispatch = "flat",
        side = "server",
        version = 5,
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "none (Container.setChanged has no bus event)",
        cancellable = false
    )]
    fn contents_changed(
        level: LevelRef,
        pos: BlockPos,
        state: BlockState,
        observer: Option<BlockEntityRef>,
    );
}
