//! A menu's linked storage in menu order: the block entities whose containers
//! it reads through `ModLinks`.
use crate::{containers::BlockEntity, items::ItemStack, world::World};

/// The slots of the block entity at `k` its menu shows: a mod container's
/// count, though items past it stay saved.
pub(crate) fn shown(world: &World, k: (i32, i32, i32)) -> Option<&[Option<ItemStack>]> {
    let e = world.block_entities.get(&k)?;
    let slots = e.slots();
    if !matches!(e, BlockEntity::Mod(_)) {
        return Some(slots);
    }
    let state = world.block(k.0, k.1, k.2).0;
    let count = world
        .block_entities
        .mod_container(state)
        .map_or(0, |c| c.slots);
    Some(&slots[..count.min(slots.len())])
}

/// A menu's linked storage in menu order. An index in the linked range without backing
/// storage is empty and unavailable, never a slot of the opening block.
#[derive(Clone, Copy)]
pub struct LinkedSlots<'a> {
    world: &'a World,
    layout: crate::containers::ModLinks,
    links: &'a [(i32, i32, i32)],
}

impl<'a> LinkedSlots<'a> {
    pub fn new(
        world: &'a World,
        layout: crate::containers::ModLinks,
        links: &'a [(i32, i32, i32)],
    ) -> Self {
        Self {
            world,
            layout,
            links,
        }
    }

    pub fn unbound(self) -> std::ops::Range<usize> {
        let range = self.layout.slots();
        range.start + linked_slot_count(self.world, self.links).min(range.len())..range.end
    }

    /// Outer None means outside the linked range; Some(None) means there is no backing.
    pub fn backing(self, slot: usize) -> Option<Option<((i32, i32, i32), usize)>> {
        let range = self.layout.slots();
        if !range.contains(&slot) {
            return None;
        }
        let mut offset = slot - range.start;
        for &pos in self.links {
            let len = shown(self.world, pos).map_or(0, <[_]>::len);
            if offset < len {
                return Some(Some((pos, offset)));
            }
            offset -= len;
        }
        Some(None)
    }

    /// Outer None means outside the linked range; Some(None) means there is no backing.
    pub(crate) fn get(self, slot: usize) -> Option<Option<&'a Option<ItemStack>>> {
        let range = self.layout.slots();
        range
            .contains(&slot)
            .then(|| linked_stacks(self.world, self.links).nth(slot - range.start))
    }

    /// Stage the same live projection in the click's working storage.
    pub fn overlay(self, items: &mut [Option<ItemStack>]) {
        let slots = items
            .iter_mut()
            .skip(self.layout.from)
            .take(self.layout.count);
        for (slot, stack) in slots.zip(
            linked_stacks(self.world, self.links)
                .cloned()
                .chain(std::iter::repeat(None)),
        ) {
            *slot = stack;
        }
    }
}

pub(crate) fn linked_stacks<'a>(
    world: &'a World,
    links: &'a [(i32, i32, i32)],
) -> impl Iterator<Item = &'a Option<ItemStack>> {
    links
        .iter()
        .flat_map(|&k| shown(world, k).unwrap_or_default().iter())
}

/// How many slots the linked containers `links` have between them.
pub fn linked_slot_count(world: &World, links: &[(i32, i32, i32)]) -> usize {
    links
        .iter()
        .map(|&k| shown(world, k).map_or(0, <[_]>::len))
        .sum()
}
