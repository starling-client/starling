//! Session-only, spatially indexed watered areas. Mod switches eagerly release attributed boxes.
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static LEVELS: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());

/// Opaque, non-reused identity for one watered-area ticket, issued only by [`WaterTickets::add`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaterTicketId(u64);

#[derive(Clone, Copy)]
struct TileKey {
    depth: u8,
    coords: [u32; 2],
}

struct Ticket {
    owner: Option<&'static str>,
    min: [i32; 3],
    max: [i32; 3],
    tile: TileKey,
    master: [i32; 2],
}
#[derive(Default)]
struct Index {
    tickets: HashMap<WaterTicketId, Ticket>,
    depths: BTreeMap<u8, HashMap<[u32; 2], Vec<WaterTicketId>>>,
    by_master: HashMap<[i32; 2], Vec<WaterTicketId>>,
}
struct Shared {
    nonempty: AtomicBool,
    index: Mutex<Index>,
}

/// Bias a signed chunk coordinate (`block >> 4`, in `-2^27..2^27`) into `0..2^28`.
fn biased_chunk(block: i32) -> u32 {
    ((block >> 4) as u32).wrapping_add(1u32 << 27)
}

/// Chunk coordinate in `[min >> 4, (max - 1) >> 4]` whose center (`16 * c + 8`) is closest to
/// the half-open box midpoint `(min + max) / 2.0`, matching `FarmlandWaterManager.addAABBTicket`
/// in O(1) without overflow; ties break deterministically to the lower coordinate.
fn master_axis(min: i32, max: i32) -> i32 {
    let min_chunk = i64::from(min >> 4);
    let max_chunk = i64::from((max - 1) >> 4);
    let sum = i64::from(min) + i64::from(max);
    (sum - 1).div_euclid(32).clamp(min_chunk, max_chunk) as i32
}

fn remove_from_depths(
    depths: &mut BTreeMap<u8, HashMap<[u32; 2], Vec<WaterTicketId>>>,
    tile: TileKey,
    id: WaterTicketId,
) {
    if let Some(tiles) = depths.get_mut(&tile.depth) {
        if let Some(ids) = tiles.get_mut(&tile.coords) {
            ids.retain(|other| *other != id);
            if ids.is_empty() {
                tiles.remove(&tile.coords);
            }
        }
        if tiles.is_empty() {
            depths.remove(&tile.depth);
        }
    }
}

fn remove_from_master(
    by_master: &mut HashMap<[i32; 2], Vec<WaterTicketId>>,
    master: [i32; 2],
    id: WaterTicketId,
) {
    if let Some(ids) = by_master.get_mut(&master) {
        ids.retain(|other| *other != id);
        if ids.is_empty() {
            by_master.remove(&master);
        }
    }
}

/// A level's transient watered areas. Empty levels allocate nothing and queries only test the
/// empty-index guard; neither queries nor farmland ticks call mod code.
#[derive(Default)]
pub struct WaterTickets {
    shared: Option<Arc<Shared>>,
}

impl WaterTickets {
    /// Add a half-open integer box (`min <= pos < max`) and issue its opaque [`WaterTicketId`].
    /// Empty or reversed boxes are refused.
    /// `owner` is evaluated under the lock shared with owner-disable cleanup: `Ok(Some(id))`
    /// attributes the ticket to an enabled installed mod, `Ok(None)` admits an unattributed
    /// ticket, and `Err(())` refuses admission (for example when a disabled installed mod races
    /// with disable cleanup).
    /// Each ticket is attached to the covered chunk closest to its horizontal center (NeoForge's
    /// `FarmlandWaterManager.addAABBTicket` master chunk); unloading that chunk invalidates it.
    pub fn add(
        &mut self,
        min: [i32; 3],
        max: [i32; 3],
        owner: impl FnOnce() -> Result<Option<&'static str>, ()>,
    ) -> Option<WaterTicketId> {
        if (0..3).any(|axis| min[axis] >= max[axis]) {
            return None;
        }
        let min_x = biased_chunk(min[0]);
        let max_x = biased_chunk(max[0] - 1);
        let min_z = biased_chunk(min[2]);
        let max_z = biased_chunk(max[2] - 1);
        let depth = (u32::BITS - ((min_x ^ max_x) | (min_z ^ max_z)).leading_zeros()) as u8;
        let tile = TileKey {
            depth,
            coords: [min_x >> depth, min_z >> depth],
        };
        let master = [master_axis(min[0], max[0]), master_axis(min[2], max[2])];
        // Serialize cleanup attribution with owner-disable cleanup so a concurrent disable
        // either removes the ticket or refuses admission rather than leaking an unattributed box.
        let mut levels = LEVELS.lock().unwrap();
        let Ok(owner) = owner() else {
            return None;
        };
        let id = NEXT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()
            .map(WaterTicketId)?;
        let shared = self.shared.get_or_insert_with(|| {
            let shared = Arc::new(Shared {
                nonempty: AtomicBool::new(false),
                index: Mutex::new(Index::default()),
            });
            levels.retain(|level| level.strong_count() != 0);
            levels.push(Arc::downgrade(&shared));
            shared
        });
        let mut index = shared.index.lock().unwrap();
        index.tickets.insert(
            id,
            Ticket {
                owner,
                min,
                max,
                tile,
                master,
            },
        );
        index
            .depths
            .entry(tile.depth)
            .or_default()
            .entry(tile.coords)
            .or_default()
            .push(id);
        index.by_master.entry(master).or_default().push(id);
        shared.nonempty.store(true, Ordering::Release);
        Some(id)
    }

    /// Remove the ticket with this issued identity; overlapping boxes remain independent.
    pub fn remove(&mut self, id: WaterTicketId) -> bool {
        let Some(shared) = &self.shared else {
            return false;
        };
        let mut index = shared.index.lock().unwrap();
        let Some(ticket) = index.tickets.remove(&id) else {
            return false;
        };
        remove_from_depths(&mut index.depths, ticket.tile, id);
        remove_from_master(&mut index.by_master, ticket.master, id);
        shared
            .nonempty
            .store(!index.tickets.is_empty(), Ordering::Release);
        true
    }

    /// Invalidate all tickets whose master chunk is `chunk` (`FarmlandWaterManager.removeTickets`
    /// and `SimpleTicket.unload`), returning how many were removed. Overlapping tickets mastered
    /// in other chunks remain active.
    pub fn unload_chunk(&mut self, chunk: [i32; 2]) -> usize {
        let Some(shared) = &self.shared else {
            return 0;
        };
        if !shared.nonempty.load(Ordering::Acquire) {
            return 0;
        }
        let mut index = shared.index.lock().unwrap();
        let Some(ids) = index.by_master.remove(&chunk) else {
            return 0;
        };
        let mut removed = 0;
        for id in ids {
            if let Some(ticket) = index.tickets.remove(&id) {
                remove_from_depths(&mut index.depths, ticket.tile, id);
                removed += 1;
            }
        }
        shared
            .nonempty
            .store(!index.tickets.is_empty(), Ordering::Release);
        removed
    }

    /// Test only the position's tile buckets at populated depths, without allocating or invoking a
    /// mod callback.
    pub fn contains(&self, pos: [i32; 3]) -> bool {
        let Some(shared) = &self.shared else {
            return false;
        };
        if !shared.nonempty.load(Ordering::Acquire) {
            return false;
        }
        let px = biased_chunk(pos[0]);
        let pz = biased_chunk(pos[2]);
        let index = shared.index.lock().unwrap();
        index.depths.iter().any(|(&depth, tiles)| {
            tiles.get(&[px >> depth, pz >> depth]).is_some_and(|ids| {
                ids.iter().any(|id| {
                    let ticket = &index.tickets[id];
                    (0..3).all(|axis| ticket.min[axis] <= pos[axis] && pos[axis] < ticket.max[axis])
                })
            })
        })
    }

    /// Closing a level invalidates all its handles, even if the same level object is reused.
    pub fn clear(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.nonempty.store(false, Ordering::Release);
        }
    }
}

/// Called at a mod's disable boundary, not on a tick. Weak registrations do not keep worlds alive.
/// Releases only tickets attributed to `owner`; anonymous and other mods' tickets stay intact.
pub fn remove_owner(owner: &str) {
    LEVELS.lock().unwrap().retain(|level| {
        let Some(shared) = level.upgrade() else {
            return false;
        };
        let mut index = shared.index.lock().unwrap();
        let Index {
            tickets,
            depths,
            by_master,
        } = &mut *index;
        tickets.retain(|_, ticket| ticket.owner != Some(owner));
        depths.retain(|_, tiles| {
            tiles.retain(|_, ids| {
                ids.retain(|id| tickets.contains_key(id));
                !ids.is_empty()
            });
            !tiles.is_empty()
        });
        by_master.retain(|_, ids| {
            ids.retain(|id| tickets.contains_key(id));
            !ids.is_empty()
        });
        shared
            .nonempty
            .store(!tickets.is_empty(), Ordering::Release);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_open_boxes_cross_negative_chunk_edges_and_remove_independently() {
        let mut level = WaterTickets::default();
        assert!(!level.contains([0, 64, 0]));
        let a = level
            .add([-17, 64, -1], [18, 65, 2], || Ok(Some("irrigator")))
            .unwrap();
        for pos in [[-17, 64, -1], [-16, 64, 0], [0, 64, 0], [17, 64, 1]] {
            assert!(level.contains(pos), "{pos:?}");
        }
        for pos in [
            [-18, 64, 0],
            [18, 64, 0],
            [0, 63, 0],
            [0, 65, 0],
            [0, 64, -2],
            [0, 64, 2],
        ] {
            assert!(!level.contains(pos), "{pos:?}");
        }
        let anon = level.add([0, 64, 0], [5, 65, 5], || Ok(None)).unwrap();
        let b = level
            .add([2, 64, 2], [6, 65, 6], || Ok(Some("neighbor")))
            .unwrap();
        assert!(level.remove(a));
        assert!(!level.remove(a));
        assert!(!level.contains([-17, 64, 0]));
        assert!(level.contains([0, 64, 0]));
        assert!(level.contains([5, 64, 5]));
        // Unrelated owner cleanup leaves anonymous and separately attributed overlapping rows.
        remove_owner("irrigator");
        assert!(level.contains([0, 64, 0]));
        assert!(level.contains([5, 64, 5]));
        assert!(level.remove(anon));
        assert!(!level.remove(anon));
        assert!(!level.contains([0, 64, 0]));
        assert!(level.contains([2, 64, 2]));
        assert!(level.remove(b));
        assert!(!level.remove(b));
        assert!(!level.contains([2, 64, 2]));
    }

    #[test]
    fn owner_disable_releases_all_levels_without_resurrecting_old_handles() {
        let mut overworld = WaterTickets::default();
        let mut nether = WaterTickets::default();
        let old = overworld
            .add([0, 64, 0], [5, 65, 5], || Ok(Some("water_test_owner")))
            .unwrap();
        let anon = overworld.add([1, 64, 1], [2, 65, 2], || Ok(None)).unwrap();
        assert!(!nether.contains([0, 64, 0]));
        let nether_owned = nether
            .add([0, 64, 0], [5, 65, 5], || Ok(Some("water_test_owner")))
            .unwrap();
        let nether_other = nether
            .add([3, 64, 3], [4, 65, 4], || Ok(Some("water_test_other")))
            .unwrap();
        remove_owner("water_test_owner");
        assert!(!overworld.contains([0, 64, 0]));
        assert!(overworld.contains([1, 64, 1]));
        assert!(!nether.contains([0, 64, 0]));
        assert!(nether.contains([3, 64, 3]));
        assert!(!overworld.remove(old));
        assert!(!nether.remove(nether_owned));
        // A disabled-owner refusal denies admission without leaking an unattributed ticket.
        assert!(overworld.add([6, 64, 6], [7, 65, 7], || Err(())).is_none());
        assert!(!overworld.contains([6, 64, 6]));
        // An unattributed caller still adds without reviving old keys.
        let unattributed = overworld.add([4, 64, 4], [5, 65, 5], || Ok(None)).unwrap();
        assert_ne!(old, unattributed);
        assert!(overworld.contains([4, 64, 4]));
        assert!(!overworld.contains([0, 64, 0]));
        assert!(!overworld.remove(old));
        let new = overworld
            .add([0, 64, 0], [5, 65, 5], || Ok(Some("water_test_owner")))
            .unwrap();
        assert_ne!(old, new);
        assert!(!overworld.remove(old));
        assert!(overworld.contains([0, 64, 0]));
        assert!(overworld.remove(anon));
        assert!(overworld.remove(unattributed));
        overworld.clear();
        assert!(!overworld.contains([0, 64, 0]));
        assert!(!overworld.remove(new));
        assert!(nether.remove(nether_other));
        assert!(!nether.contains([3, 64, 3]));
        let reopened = WaterTickets::default();
        assert!(!reopened.contains([0, 64, 0]));
    }

    #[test]
    fn wide_boxes_index_past_the_old_threshold_and_preserve_overlapping_narrow_tickets() {
        let mut level = WaterTickets::default();
        // 257 x 256 chunk columns: past the old 65,536-column threshold.
        let past_old_cap = level
            .add([0, 64, 0], [4097, 65, 4096], || Ok(Some("wide")))
            .unwrap();
        assert!(level.contains([0, 64, 0]));
        assert!(level.contains([4096, 64, 4095]));
        assert!(!level.contains([4097, 64, 4095]));
        assert!(!level.contains([4096, 64, 4096]));
        assert!(!level.contains([-1, 64, 0]));

        let narrow = level
            .add([10, 64, 10], [15, 65, 15], || Ok(Some("sprinkler")))
            .unwrap();
        let near_full = level
            .add([i32::MIN, 64, i32::MIN], [i32::MAX, 65, i32::MAX], || {
                Ok(Some("wide"))
            })
            .unwrap();
        for pos in [
            [i32::MIN, 64, i32::MIN],
            [-1_000_000, 64, -17],
            [12, 64, 12],
            [1_000_000, 64, 500_000],
            [i32::MAX - 1, 64, i32::MAX - 1],
        ] {
            assert!(level.contains(pos), "{pos:?}");
        }
        for pos in [[i32::MAX, 64, 0], [0, 64, i32::MAX], [0, 63, 0], [0, 65, 0]] {
            assert!(!level.contains(pos), "{pos:?}");
        }

        assert!(level.remove(near_full));
        assert!(!level.remove(near_full));
        assert!(!level.contains([-1_000_000, 64, -17]));
        assert!(!level.contains([i32::MAX - 1, 64, i32::MAX - 1]));
        assert!(level.contains([4096, 64, 4095]));
        assert!(level.contains([12, 64, 12]));

        assert!(level.remove(past_old_cap));
        assert!(!level.contains([4096, 64, 4095]));
        assert!(!level.contains([0, 64, 0]));
        assert!(level.contains([10, 64, 10]));
        assert!(level.contains([14, 64, 14]));
        assert!(!level.contains([15, 64, 10]));
        assert!(!level.contains([10, 64, 15]));
        {
            let index = level.shared.as_ref().unwrap().index.lock().unwrap();
            assert_eq!(index.depths.len(), 1);
            assert!(index.depths.contains_key(&0));
            assert_eq!(index.by_master.len(), 1);
        }

        assert!(level.remove(narrow));
        assert!(!level.contains([12, 64, 12]));
        let shared = level.shared.as_ref().unwrap();
        assert!(!shared.nonempty.load(Ordering::Acquire));
        let index = shared.index.lock().unwrap();
        assert!(index.depths.is_empty());
        assert!(index.by_master.is_empty());
    }

    #[test]
    fn master_chunk_unload_invalidates_only_tickets_mastered_in_that_chunk() {
        let mut level = WaterTickets::default();
        assert_eq!(level.unload_chunk([0, 0]), 0);
        // Box spans chunks (-1, 0), (0, 0), (1, 0) with midpoint x = 8.5 -> master chunk (0, 0).
        let centered_at_zero = level
            .add([-4, 64, 2], [21, 65, 10], || Ok(Some("sprinkler")))
            .unwrap();
        // Overlapping box spans chunks (0, 0) and (1, 0) with midpoint x = 23.0 -> master chunk (1, 0).
        let centered_at_one = level
            .add([12, 64, 2], [34, 65, 10], || Ok(Some("neighbor")))
            .unwrap();
        // Unloading a non-master covered chunk leaves the ticket active everywhere.
        assert_eq!(level.unload_chunk([-1, 0]), 0);
        assert!(level.contains([-4, 64, 2]));
        assert!(level.contains([20, 64, 2]));
        // Unloading chunk (0, 0) invalidates only the ticket mastered at (0, 0).
        assert_eq!(level.unload_chunk([0, 0]), 1);
        assert!(!level.remove(centered_at_zero));
        assert!(!level.contains([-4, 64, 2]));
        assert!(!level.contains([4, 64, 2]));
        // The overlapping ticket mastered at (1, 0) still hydrates its overlap in chunk (0, 0).
        assert!(level.contains([12, 64, 2]));
        assert!(level.contains([33, 64, 2]));
        // Reissuing after chunk unload mints a distinct live handle.
        let reissued = level
            .add([-4, 64, 2], [21, 65, 10], || Ok(Some("sprinkler")))
            .unwrap();
        assert_ne!(reissued, centered_at_zero);
        assert!(!level.remove(centered_at_zero));
        assert!(level.contains([-4, 64, 2]));
        // Unloading chunk (1, 0) removes only the ticket mastered there.
        assert_eq!(level.unload_chunk([1, 0]), 1);
        assert!(!level.remove(centered_at_one));
        assert!(!level.contains([33, 64, 2]));
        assert!(level.contains([12, 64, 2]));
        assert!(level.remove(reissued));
        let shared = level.shared.as_ref().unwrap();
        assert!(!shared.nonempty.load(Ordering::Acquire));
        let index = shared.index.lock().unwrap();
        assert!(index.depths.is_empty());
        assert!(index.by_master.is_empty());
    }

    #[test]
    fn master_axis_matches_closest_chunk_center_including_extreme_bounds() {
        assert_eq!(master_axis(0, 16), 0);
        assert_eq!(master_axis(0, 32), 0); // tie between chunk 0 (center 8) and 1 (center 24)
        assert_eq!(master_axis(0, 33), 1); // midpoint 16.5 -> chunk 1 (center 24)
        assert_eq!(master_axis(-16, 0), -1);
        assert_eq!(master_axis(-16, 16), -1); // tie between -1 (-8) and 0 (8)
        assert_eq!(master_axis(-15, 16), 0); // midpoint 0.5 -> chunk 0 (8)
        assert_eq!(master_axis(15, 17), 0); // midpoint 16.0 -> tie -> chunk 0
        assert_eq!(master_axis(15, 18), 1); // midpoint 16.5 -> chunk 1
        assert_eq!(master_axis(i32::MIN, i32::MAX), -1); // midpoint -0.5 -> chunk -1 (-8)
    }

    #[test]
    fn invalid_boxes_do_not_create_an_index() {
        let mut level = WaterTickets::default();
        for (min, max) in [
            ([0, 64, 0], [0, 65, 5]),
            ([5, 64, 0], [0, 65, 5]),
            ([0, 64, 0], [5, 64, 5]),
            ([0, 64, 5], [5, 65, 0]),
            ([i32::MIN, 0, 0], [i32::MIN, 1, 1]),
            ([0, 0, 0], [i32::MIN, 1, 1]),
        ] {
            assert!(
                level.add(min, max, || Ok(Some("invalid"))).is_none(),
                "{min:?}..{max:?}"
            );
        }
        assert!(level.shared.is_none());
    }
}
