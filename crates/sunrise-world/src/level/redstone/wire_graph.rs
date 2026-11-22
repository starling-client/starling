//! The per-cascade cache of the inputs that decide a wire's power.
//!
//! `DefaultRedstoneWireEvaluator.updatePowerStrength` recomputes two things for
//! every wire it visits:
//!
//! - `getBestNeighborSignal` with wire signals suppressed, which asks six
//!   neighbours for their signal and, for a conductor, runs
//!   `getDirectSignalTo`'s six more; and
//! - `getIncomingSignal`, which walks four horizontal neighbours and the
//!   position above or below each of them, reading a wire's power wherever it
//!   finds one.
//!
//! Neither depends on any wire's `power`: the first runs with wire signals off,
//! and the second only reads `power`. So while a cascade is propagating, they
//! are the same for every visit to the same wire, and a cascade visits a wire
//! about three and a half times (a power change notifies the position and its
//! six neighbours, and every wire in that set notifies again). The benchmark's
//! 128-cell line costs 447 visits and 12,352 level reads per toggle, of which
//! ~27 reads per visit are these two walks.
//!
//! A node therefore caches the neighbour signal, which offsets around the wire
//! hold a wire whose power feeds it, and that wire's power as of the run's last
//! write to it. The offsets are a bitmask over [`SOURCE_OFFSETS`], so a visit
//! looks up the wires the mask holds and nothing else. The cache belongs to one
//! neighbour-update run — see [`WireGraph`] for why the run and not the call is
//! the unit — and nothing survives it, so there is no cross-tick invalidation
//! to get wrong.
//!
//! Inside a run, what can change a cached input?
//!
//! - A block write. A wire change notifies repeaters, lamps and the rest, which
//!   can write their own state immediately, and that changes a neighbouring
//!   wire's neighbour signal. `chunk_set_block_state` clears the cache for any
//!   write that is not a redstone wire on both sides; a wire's own power and
//!   shape feed neither cached value, so the cascade's own writes keep it, and
//!   they update the node's power in place.
//! - A comparator's output, which lives in its block entity and moves without
//!   necessarily writing the block at all. It moves only from a scheduled tick
//!   and from a player's interaction, and neither runs inside a run, so it
//!   cannot move while this cache is open; `comparator_refresh` asserts that.
//! - A block entity's signal — a trapped chest's open count, a jukebox — cannot
//!   move during a run either, because no block entity ticks inside one.
//! - A mod's signal callbacks: these are keyed by position, state and
//!   direction, and a subscription cannot appear mid-run, which is the same
//!   premise the wire-shape memo already rests on. A callback that answers
//!   differently on each call would break that memo too.
//!
//! Nothing else writes during a run, so those cases are the whole of it.
//!
//! The cache never decides *when* a wire is written or in what order: visits
//! happen exactly where vanilla notifies them, in the same order, and the write
//! still goes through `set_block` and `update_neighbors_at`. It only answers
//! the two questions above without re-walking the level. Vanilla's wire
//! propagation is order-dependent (a wire reads the power its neighbours
//! happen to have when it is visited, and the step-up/step-down rule makes the
//! relation asymmetric), so the visit order is the specification.

use sunrise_core::{fast_hash::FxHashMap, position::BlockPos};

/// The offsets `getIncomingSignal` can read a wire's power from, relative to
/// the wire: the four horizontal neighbours, then the position above and below
/// each of them.
///
/// Vanilla:
///
/// ```text
/// for (Direction direction : Direction.Plane.HORIZONTAL) {
///     BlockPos neighbour = pos.relative(direction);
///     BlockState state = level.getBlockState(neighbour);
///     signal = max(signal, state.getBlock() instanceof RedstoneWireBlock
///         ? state.getValue(POWER) : 0);
///     if (state.isRedstoneConductor(level, neighbour)
///             && !level.getBlockState(above(pos)).isRedstoneConductor(level, above(pos))) {
///         signal = max(signal, powerOf(level.getBlockState(above(neighbour))));
///     } else if (!state.isRedstoneConductor(level, neighbour)) {
///         signal = max(signal, powerOf(level.getBlockState(below(neighbour))));
///     }
/// }
/// ```
pub(crate) const SOURCE_OFFSETS: [(i32, i32, i32); 12] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 0, 1),
    (0, 0, -1),
    (1, 1, 0),
    (-1, 1, 0),
    (0, 1, 1),
    (0, 1, -1),
    (1, -1, 0),
    (-1, -1, 0),
    (0, -1, 1),
    (0, -1, -1),
];

/// The offset index of a horizontal neighbour, by direction step.
pub(crate) const fn source_index(dx: i32, dz: i32, dy: i32) -> u32 {
    // Same level first, then the step up, then the step down, in the order
    // `SOURCE_OFFSETS` lists them.
    let horizontal = match (dx, dz) {
        (1, 0) => 0,
        (-1, 0) => 1,
        (0, 1) => 2,
        _ => 3,
    };
    let layer = match dy {
        0 => 0,
        1 => 4,
        _ => 8,
    };
    horizontal + layer
}

/// One wire's cached inputs during a cascade.
#[derive(Clone, Copy)]
pub(crate) struct WireNode {
    /// `getBestNeighborSignal` with wire signals suppressed. A wire's own power
    /// never feeds this, so it is fixed for the cascade.
    pub block_signal: u8,
    /// Bit `i` set when the position at `SOURCE_OFFSETS[i]` holds a redstone
    /// wire, whose power `getIncomingSignal` therefore reads.
    pub sources: u16,
    /// The wire's `POWER` as of the last write this run made to it; see
    /// [`WireGraph::set_power`].
    pub power: u8,
}

/// The wires one neighbour-update run has looked at.
///
/// One run is one cascade: `add_and_run` trampolines the notifications instead
/// of recursing, so a wire update returns before the next one runs and the run
/// — not the call — is the unit the cache lives for.
#[derive(Default)]
pub(crate) struct WireGraph {
    nodes: FxHashMap<BlockPos, WireNode>,
    /// Set between [`Self::begin`] and [`Self::end`]. A `wire_update_power`
    /// outside a run (placement asks for one directly) walks the level as
    /// before rather than reading a cache nobody owns.
    active: bool,
}

impl WireGraph {
    /// A neighbour-update run started (`add_and_run` with no run in flight).
    pub(crate) fn begin(&mut self) {
        debug_assert!(!self.active, "an update run left its wire cache behind");
        self.active = true;
        self.nodes.clear();
    }

    /// The run ended: everything it cached is about to be stale, because the
    /// run's last writes are only now observable to the next one.
    pub(crate) fn end(&mut self) {
        self.active = false;
        self.nodes.clear();
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }

    pub(crate) fn get(&self, pos: BlockPos) -> Option<WireNode> {
        self.nodes.get(&pos).copied()
    }

    pub(crate) fn insert(&mut self, pos: BlockPos, node: WireNode) {
        self.nodes.insert(pos, node);
    }

    /// Record the power a wire was just written with. A run writes a wire's
    /// power only through `chunk_set_block_state`, which calls this, so a node
    /// never disagrees with the level.
    pub(crate) fn set_power(&mut self, pos: BlockPos, power: u8) {
        if let Some(node) = self.nodes.get_mut(&pos) {
            node.power = power;
        }
    }

    /// Drop every cached input: a block that is not a redstone wire changed.
    pub(crate) fn invalidate(&mut self) {
        if self.active {
            self.nodes.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SOURCE_OFFSETS, source_index};

    #[test]
    fn every_offset_is_listed_once_and_indexed_by_its_step() {
        let mut seen = Vec::new();
        for (index, &(dx, dy, dz)) in SOURCE_OFFSETS.iter().enumerate() {
            assert!(
                !seen.contains(&(dx, dy, dz)),
                "{dx},{dy},{dz} is listed twice"
            );
            seen.push((dx, dy, dz));
            assert_eq!(
                source_index(dx, dz, dy) as usize,
                index,
                "{dx},{dy},{dz} does not index itself"
            );
            assert_eq!(dx.abs() + dz.abs(), 1, "a source is horizontal");
            assert!(dy.abs() <= 1);
        }
        assert_eq!(seen.len(), 12);
    }
}
