//! Scoped scratch item slots for [`FakeHost`](super::FakeHost): single fixed-item slots
//! (`ItemAccess.forStack`) and two sequential dispenser slots (`DispenseFluidContainer`).
use crate::{ItemKey, ItemResource, ItemStack, ScratchSlot};
use std::collections::HashMap;

/// One lent scratch item place: fixed item (`Some`) for a single slot or `None` for two
/// sequential dispenser slots, its primary slot, and its optional overflow slot.
#[derive(Clone, Debug)]
pub(super) struct FakeScratchEntry {
    pub(super) item: Option<ItemKey>,
    pub(super) stack: Option<ItemStack>,
    pub(super) overflow: Option<ItemStack>,
}

/// Live lent scratch slots and per-transaction-frame rollback snapshots for one `FakeHost`.
#[derive(Debug, Default)]
pub(super) struct FakeScratch {
    live: HashMap<u64, FakeScratchEntry>,
    kept_stack: Vec<HashMap<u64, FakeScratchEntry>>,
}

impl Clone for FakeScratch {
    fn clone(&self) -> Self {
        // Cloned hosts never share live loans or open snapshot frames.
        Self::default()
    }
}

impl FakeScratch {
    pub(super) fn push_frame(&mut self) {
        self.kept_stack.push(HashMap::new());
    }

    pub(super) fn insert_loan(&mut self, item: Option<ItemKey>, stack: ItemStack) -> ScratchSlot {
        let slot = ScratchSlot::fresh();
        self.live.insert(
            slot.id(),
            FakeScratchEntry {
                item,
                stack: Some(stack),
                overflow: None,
            },
        );
        slot
    }

    /// Ends loan `id`, removing it from both live storage and the closing transaction frame.
    pub(super) fn remove_loan(&mut self, id: u64) -> Option<FakeScratchEntry> {
        if let Some(frame) = self.kept_stack.last_mut() {
            frame.remove(&id);
        }
        self.live.remove(&id)
    }

    /// Closes the innermost transaction frame: merges live snapshots into its parent on commit,
    /// or restores every live entry snapshotted in this frame on abort or unwind.
    pub(super) fn close_frame(&mut self, commit: bool) {
        let Some(frame) = self.kept_stack.pop() else {
            return;
        };
        if commit {
            if let Some(parent) = self.kept_stack.last_mut() {
                for (id, prev) in frame {
                    if self.live.contains_key(&id) {
                        parent.entry(id).or_insert(prev);
                    }
                }
            }
        } else {
            for (id, prev) in frame {
                if let Some(entry) = self.live.get_mut(&id) {
                    *entry = prev;
                }
            }
        }
    }

    pub(super) fn held(&self, slot: ScratchSlot) -> Option<&ItemStack> {
        self.live.get(&slot.id())?.stack.as_ref()
    }

    pub(super) fn insert(
        &mut self,
        slot: ScratchSlot,
        resource: &ItemResource,
        amount: u32,
        max_stack: u32,
    ) -> u32 {
        if amount == 0 {
            return 0;
        }
        let id = slot.id();
        let Some(held) = self.live.get(&id).cloned() else {
            return 0;
        };
        if held.item.is_some_and(|item| item != resource.item()) {
            return 0;
        }
        let mut stack = held.stack;
        let mut overflow = held.overflow;
        let mut put = fill(&mut stack, resource, amount, max_stack);
        if held.item.is_none() {
            put += fill(&mut overflow, resource, amount - put, max_stack);
        }
        if put == 0 {
            return 0;
        }
        self.snapshot(id);
        if let Some(entry) = self.live.get_mut(&id) {
            entry.stack = stack;
            entry.overflow = overflow;
        }
        put
    }

    pub(super) fn extract(
        &mut self,
        slot: ScratchSlot,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        if amount == 0 {
            return 0;
        }
        let id = slot.id();
        let Some(held) = self
            .live
            .get(&id)
            .and_then(|entry| entry.stack.as_ref())
            .filter(|held| resource.matches(held))
        else {
            return 0;
        };
        let took = amount.min(held.count());
        if took == 0 {
            return 0;
        }
        let remaining = held.count() - took;
        self.snapshot(id);
        if let Some(entry) = self.live.get_mut(&id) {
            entry.stack = match remaining {
                0 => None,
                count => entry.stack.take().and_then(|mut stack| {
                    stack.set_count(count).ok()?;
                    Some(stack)
                }),
            };
        }
        took
    }

    fn snapshot(&mut self, id: u64) {
        if let Some(frame) = self.kept_stack.last_mut()
            && !frame.contains_key(&id)
            && let Some(entry) = self.live.get(&id).cloned()
        {
            frame.insert(id, entry);
        }
    }

    #[cfg(test)]
    pub(super) fn live_count(&self) -> usize {
        self.live.len()
    }

    #[cfg(test)]
    pub(super) fn kept_frame_counts(&self) -> Vec<usize> {
        self.kept_stack.iter().map(HashMap::len).collect()
    }

    #[cfg(test)]
    pub(super) fn kept_contains(&self, frame_index: usize, slot: ScratchSlot) -> bool {
        self.kept_stack
            .get(frame_index)
            .is_some_and(|frame| frame.contains_key(&slot.id()))
    }
}

fn fill(slot: &mut Option<ItemStack>, resource: &ItemResource, count: u32, max_stack: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let have = match slot {
        Some(held) if !resource.matches(held) => return 0,
        Some(held) => held.count(),
        None => 0,
    };
    let put = max_stack.saturating_sub(have).min(count);
    if put > 0 {
        *slot = resource.to_stack(have + put);
    }
    put
}
