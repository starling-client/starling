//! Memoized `RedstoneWireBlock.getConnectionState` results for one
//! [`Signals`](crate::level::Signals) view.
//!
//! A wire's connection shape is a pure function of the blocks around it and of
//! whether the wire itself is a dot, but `SignalGetter` asks for the same
//! wire's shape from every direction that probes it: `getBestNeighborSignal`
//! walks six, each conductor runs `getDirectSignalTo`'s six, and every one of
//! those re-derives the whole shape — four `getConnectingSide` walks, each
//! reading up to three neighbours, plus the property-table traffic.
//!
//! A `Signals` view borrows the level shared (`&L`) and the ticks shared
//! (`&Ticks`), so while it lives no block state can change, and no `Ticks`
//! field other than this memo can change: a moving piston's progress or a
//! shulker's lid needs `&mut`, and moves between views, never inside one.
//! Every answer a view computes therefore stays true for the whole view,
//! which makes the view the exact lifetime of the memo with no invalidation
//! to get wrong.
//!
//! Two view-constant inputs are deliberately outside the key: `has_hook`
//! (installed hooks do not change during a view) and the input state's side
//! properties (the shape is seeded from the wire's default state, so only its
//! `power` and whether it is a dot reach the result). Views that pass any
//! other state — the shape, placement and interaction paths hand over a cross
//! or a default state — use view 0, which caches nothing.

use crate::registry::BlockStateId;
use std::cell::Cell;
use sunrise_core::position::BlockPos;

/// Slots. One probe touches the wire and its neighbours, so a small
/// direct-mapped table holds a whole view without evicting.
const SLOTS: usize = 64;

#[derive(Clone, Copy)]
struct Slot {
    view: u64,
    pos: BlockPos,
    power: i32,
    was_dot: bool,
    state: BlockStateId,
}

/// A slot no query uses: ids start at 1 and view 0 caches nothing.
const EMPTY: Slot = Slot {
    view: 0,
    pos: BlockPos::new(0, 0, 0),
    power: 0,
    was_dot: false,
    state: BlockStateId(0),
};

/// The connection states computed during one `Signals` view.
pub(crate) struct WireConnections {
    next_view: Cell<u64>,
    slots: [Cell<Slot>; SLOTS],
}

impl Default for WireConnections {
    fn default() -> Self {
        Self {
            next_view: Cell::new(1),
            slots: [const { Cell::new(EMPTY) }; SLOTS],
        }
    }
}

impl WireConnections {
    /// Begin a view. Views start at 1, so no slot is born current.
    pub(crate) fn next_view(&self) -> u64 {
        let view = self.next_view.get();
        self.next_view.set(view.wrapping_add(1).max(1));
        view
    }

    /// The shape for `pos` in `view`, if this view already computed it.
    /// View 0 caches nothing.
    pub(crate) fn get(
        &self,
        view: u64,
        pos: BlockPos,
        power: i32,
        was_dot: bool,
    ) -> Option<BlockStateId> {
        if view == 0 {
            return None;
        }
        let slot = self.slots[index(pos, power, was_dot)].get();
        (slot.view == view && slot.pos == pos && slot.power == power && slot.was_dot == was_dot)
            .then_some(slot.state)
    }

    /// Record the shape computed in `view`. View 0 caches nothing.
    pub(crate) fn put(
        &self,
        view: u64,
        pos: BlockPos,
        power: i32,
        was_dot: bool,
        state: BlockStateId,
    ) {
        if view == 0 {
            return;
        }
        self.slots[index(pos, power, was_dot)].set(Slot {
            view,
            pos,
            power,
            was_dot,
            state,
        });
    }
}

/// The slot of one key: a multiplication hash of the position, power and dot
/// bit, folded to `SLOTS` (a power of two).
fn index(pos: BlockPos, power: i32, was_dot: bool) -> usize {
    let mut hash = (pos.x as u32).wrapping_mul(0x9E37_79B1)
        ^ (pos.y as u32).wrapping_mul(0x85EB_CA77)
        ^ (pos.z as u32).wrapping_mul(0xC2B2_AE3D)
        ^ (power as u32).wrapping_mul(0x27D4_EB2F)
        ^ u32::from(was_dot).wrapping_mul(0x85EB_CA77);
    hash ^= hash >> 15;
    hash ^= hash >> 7;
    hash as usize & (SLOTS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: u32) -> BlockStateId {
        BlockStateId(id)
    }

    #[test]
    fn caches_within_one_view_only() {
        let cache = WireConnections::default();
        let first = cache.next_view();
        let second = cache.next_view();
        assert_ne!(first, second);
        let pos = BlockPos::new(3, 64, -7);
        assert_eq!(cache.get(first, pos, 12, false), None);
        cache.put(first, pos, 12, false, state(1200));
        assert_eq!(cache.get(first, pos, 12, false), Some(state(1200)));
        assert_eq!(
            cache.get(second, pos, 12, false),
            None,
            "a new view starts empty"
        );
        assert_eq!(
            cache.get(first, pos, 11, false),
            None,
            "power is part of the key"
        );
        assert_eq!(
            cache.get(first, pos, 12, true),
            None,
            "dot shape is part of the key"
        );
        assert_eq!(cache.get(first, pos.offset(1, 0, 0), 12, false), None);
    }

    #[test]
    fn a_colliding_key_replaces_its_slot() {
        let cache = WireConnections::default();
        let view = cache.next_view();
        let first = BlockPos::new(0, 0, 0);
        let wanted = index(first, 0, false);
        let second = (1..10_000)
            .map(|dx| BlockPos::new(dx, 0, 0))
            .find(|pos| index(*pos, 0, false) == wanted)
            .expect("a colliding position");
        cache.put(view, first, 0, false, state(1));
        cache.put(view, second, 0, false, state(2));
        assert_eq!(cache.get(view, second, 0, false), Some(state(2)));
        assert_eq!(
            cache.get(view, first, 0, false),
            None,
            "the newer key won the slot"
        );
    }

    #[test]
    fn view_zero_caches_nothing() {
        let cache = WireConnections::default();
        let pos = BlockPos::new(1, 2, 3);
        cache.put(0, pos, 0, false, state(9));
        assert_eq!(cache.get(0, pos, 0, false), None);
        let view = cache.next_view();
        assert_eq!(cache.get(view, pos, 0, false), None);
    }
}
