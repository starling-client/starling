//! The `mob_split` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A slime, magma cube or sulfur cube is dying and about to split into
    /// smaller ones (NeoForge's `MobSplitEvent`, vanilla's
    /// `AbstractCubeMob.remove`): `dimension` is the level it dies in (a
    /// `PendingMobSpec` for a new child names it), `parent` its entity id (still resolvable
    /// with `host.entity(parent)`), `kind` its type,
    /// `size` its size and `child_size` the size of the children (half of
    /// it). The children vanilla made (2 to 4 for a slime or magma cube, 2
    /// for an unprimed sulfur cube, or 0 for a primed sulfur cube above size 1)
    /// are already constructed, in order, and wait in an unbounded mutable list
    /// lent as `children` (`MobSplitChildren<'_>`, matching NeoForge's
    /// `MobSplitEvent.getChildren()`): inspect a child (`event.children.get(i)`),
    /// edit one in place (`event.children.get_mut(i)`), construct a detached
    /// [`UnspawnedMob`] (`event.children.construct(host, spec)?`) and add or swap it in
    /// (`event.children.push(child)?`, `event.children.insert(i, child)?`,
    /// `event.children.replace(i, child)?`), or remove children
    /// (`event.children.remove(i)`, `event.children.clear()`). Each child is a
    /// complete mob that has not joined the level. All original proposals have already
    /// fired `entity_constructing`; removed and canceled proposals discard their unpublished
    /// state but do not undo other Host writes. Any detached [`UnspawnedMob`] not left in the
    /// list is cleanly discarded when dropped.
    /// If a listener panics, the shared dispatcher restores its cancellation flag and
    /// disables that listener for the session. Direct writes to children and their list
    /// are not journaled, so edits made before the panic remain visible to later listeners.
    /// Set `cancel` (or return `Flow::Cancel`, or leave the list empty) to
    /// spawn no children: the construction draws stay spent and the parent still dies.
    /// If accepted, children join once each in list order. The parent's removal effects
    /// still run when it is removed, even if the split was canceled. Only for a parent above
    /// size 1, once its children are made. With no listener nothing is allocated for the list.
    /// On the server.
    #[notes = r#"## Mob split

- **`mob_split(dimension, parent, kind, size, child_size, children, cancel)`** (NeoForge's `MobSplitEvent`, version 4): fires when a slime, magma cube or sulfur cube above size 1 dies and splits, after vanilla has constructed its children (`AbstractCubeMob.remove`: the count, then each half-size child and the parent's yaw draw for it; a primed sulfur cube above size 1 dispatches with an empty list). `children` (`MobSplitChildren<'_>`) lends an unbounded mutable child list directly on the stack, matching NeoForge's `MobSplitEvent.getChildren()`.
- The list editor offers `len`, `is_empty`, `get` (read-only `UnspawnedMobView<'_>`), `get_mut` (mutable `PendingChildEditor<'_>` with `set_position`, `set_yaw`, `set_baby`, `set_size` and all unspawned-mob edits), `construct` (taking a `PendingMobSpec` and returning an owned `UnspawnedMob`), `push`, `insert`, `replace`, `remove` and `clear`. Any detached `UnspawnedMob` not left in the list is cleanly discarded on normal Rust drop with no arena or sweep. There is no 64-child cap, no lease token and no `Expired`/`Ownership`/`Capacity`/`Budget` errors; an out-of-bounds index on `insert`/`replace`, an unknown kind, a wrong dimension, a non-finite position or yaw, or an invalid size answers `Invalid` without mutating the list.
- A panicking listener is disabled for this hook by the shared dispatcher. Its cancellation flag is restored, but direct writes to children and their list are not journaled and remain visible to later listeners.
- A detached child constructed by a listener fires `entity_constructing` immediately, even if it is never added to the list. Every original proposal fires `entity_constructing` before `mob_split`, including proposals later removed or canceled. Canceling (`Flow::Cancel`) or removing every child spawns none. Construction draws and unrelated Host writes stay, and the parent still dies. If accepted, the remaining children join once each in list order. Removal effects read the live parent after the split and child admissions, so removing an effect during either callback prevents its removal action.
- Not fired for a parent of size 1. With no listener the split runs exactly as vanilla and allocates nothing for the list."#]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        version = 4,
        contract = "cancel",
        out = "cancel",
        neoforge = "MobSplitEvent",
        paper = "SlimeSplitEvent",
        cancellable = true
    )]
    fn mob_split(
        dimension: &str,
        parent: u32,
        kind: &str,
        size: i32,
        child_size: i32,
        children: MobSplitChildren<'_>,
        cancel: &mut bool,
    );
}
