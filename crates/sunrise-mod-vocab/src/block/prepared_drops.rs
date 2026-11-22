//! Borrowed live views of the prepared item drops for a broken block.

use crate::{ItemDrop, ItemKey, ItemStack, ItemStackView, StackWriteError};
use std::num::NonZeroU32;

/// Engine backing for a live prepared block-drops list.
// hidden: the engine owns the prepared drop list and stack validation; listeners use PreparedBlockDrops.
#[doc(hidden)]
pub trait PreparedBlockDropsAccess {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn stack_metadata(&self, index: usize) -> Option<(ItemKey, NonZeroU32, Option<u32>)>;
    fn project_stack(
        &self,
        index: usize,
        metadata: (ItemKey, NonZeroU32, Option<u32>),
    ) -> Option<ItemStack>;
    fn position(&self, index: usize) -> Option<[f64; 3]>;
    fn velocity(&self, index: usize) -> Option<[f64; 3]>;
    fn pickup_delay(&self, index: usize) -> Option<u32>;
    fn thrower(&self, index: usize) -> Option<Option<u128>>;
    fn target(&self, index: usize) -> Option<Option<u128>>;
    fn no_gravity(&self, index: usize) -> Option<bool>;
    fn age(&self, index: usize) -> Option<i32>;
    fn set_stack(&mut self, index: usize, stack: ItemStack) -> Result<bool, StackWriteError>;
    fn set_position(&mut self, index: usize, position: [f64; 3]) -> bool;
    fn set_velocity(&mut self, index: usize, velocity: [f64; 3]) -> bool;
    fn set_pickup_delay(&mut self, index: usize, pickup_delay: u32) -> bool;
    fn set_thrower(&mut self, index: usize, thrower: Option<u128>) -> bool;
    fn set_target(&mut self, index: usize, target: Option<u128>) -> bool;
    fn set_no_gravity(&mut self, index: usize, no_gravity: bool) -> bool;
    fn set_age(&mut self, index: usize, age: i32) -> bool;
    fn push(&mut self, drop: ItemDrop) -> Result<(), StackWriteError>;
    fn insert(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError>;
    fn replace(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError>;
    fn remove(&mut self, index: usize) -> bool;
    fn clear(&mut self);
    fn truncate(&mut self, len: usize);
    fn swap(&mut self, a: usize, b: usize) -> bool;
}

/// A live mutable view of the prepared item drops for a broken block.
///
/// Reading an entry's item key, count, damage or entity fields does not project
/// its component patch; call [`ItemStackView::project`] or
/// [`PreparedDropView::to_item_drop`] to retain an owned copy. Stack writes are
/// validated individually so a rejected write leaves existing drops intact.
// exhaustive: only the engine constructs a live prepared-drops view.
// no wire: borrows the live prepared block-drops storage.
pub struct PreparedBlockDrops<'a> {
    access: &'a mut dyn PreparedBlockDropsAccess,
}

impl<'a> PreparedBlockDrops<'a> {
    // hidden: the engine constructs a live prepared-drops view over its own drop storage.
    /// Wrap the engine's prepared drop storage.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn PreparedBlockDropsAccess) -> Self {
        Self { access }
    }

    /// Number of prepared drops currently in the list.
    pub fn len(&self) -> usize {
        self.access.len()
    }

    /// Whether there are currently no prepared drops.
    pub fn is_empty(&self) -> bool {
        self.access.is_empty()
    }

    /// Borrow the prepared drop at `index`, or `None` if `index` is out of bounds.
    pub fn get(&self, index: usize) -> Option<PreparedDropView<'_>> {
        if index >= self.access.len() {
            return None;
        }
        Some(PreparedDropView {
            access: &*self.access,
            index,
        })
    }

    /// Mutably borrow the prepared drop at `index`, or `None` if `index` is out of bounds.
    pub fn get_mut(&mut self, index: usize) -> Option<PreparedDropMut<'_>> {
        if index >= self.access.len() {
            return None;
        }
        Some(PreparedDropMut {
            access: &mut *self.access,
            index,
        })
    }

    /// Iterate over every prepared drop in order without projecting component patches.
    pub fn iter(&self) -> impl Iterator<Item = PreparedDropView<'_>> + '_ {
        (0..self.len()).map(|index| PreparedDropView {
            access: &*self.access,
            index,
        })
    }

    /// Append a drop to the end of the prepared drop list.
    /// Returns `Err(StackWriteError)` without mutating the list if `drop.stack` is invalid.
    pub fn push(&mut self, drop: ItemDrop) -> Result<(), StackWriteError> {
        self.access.push(drop)
    }

    /// Insert a drop at `index`.
    /// Returns `Ok(false)` without panicking if `index > self.len()`, or
    /// `Err(StackWriteError)` without mutating the list if `drop.stack` is invalid.
    pub fn insert(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError> {
        self.access.insert(index, drop)
    }

    /// Replace the drop at `index`.
    /// Returns `Ok(false)` without panicking if `index >= self.len()`, or
    /// `Err(StackWriteError)` leaving the existing entry intact if `drop.stack` is invalid.
    pub fn replace(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError> {
        self.access.replace(index, drop)
    }

    /// Remove the drop at `index`. Returns `false` if `index` is out of bounds.
    pub fn remove(&mut self, index: usize) -> bool {
        self.access.remove(index)
    }

    /// Remove the last prepared drop. Returns `false` if the list is empty.
    pub fn pop(&mut self) -> bool {
        let Some(last) = self.access.len().checked_sub(1) else {
            return false;
        };
        self.access.remove(last)
    }

    /// Remove every prepared drop from the list.
    pub fn clear(&mut self) {
        self.access.clear();
    }

    /// Shorten the list to at most `len` prepared drops.
    pub fn truncate(&mut self, len: usize) {
        self.access.truncate(len);
    }

    /// Swap the drops at `a` and `b` in place.
    /// Returns `false` if either index is out of bounds.
    pub fn swap(&mut self, a: usize, b: usize) -> bool {
        self.access.swap(a, b)
    }

    /// Retain only the drops for which `keep` returns `true`, preserving order
    /// and existing entry identity without projecting stacks unless `keep` does.
    pub fn retain(&mut self, mut keep: impl FnMut(PreparedDropView<'_>) -> bool) {
        let len = self.access.len();
        let mut write = 0;
        for read in 0..len {
            let keep_entry = keep(PreparedDropView {
                access: &*self.access,
                index: read,
            });
            if keep_entry {
                if write != read {
                    self.access.swap(write, read);
                }
                write += 1;
            }
        }
        self.access.truncate(write);
    }
}

/// A read-only view of one prepared block drop.
///
/// Reading the item key, count, damage or entity fields does not copy or encode
/// the stack's component patch. Call [`ItemStackView::project`] or
/// [`Self::to_item_drop`] to retain an owned copy.
// exhaustive: only PreparedBlockDrops constructs a prepared-drop view.
// no wire: borrows one entry from the live prepared block-drops storage.
pub struct PreparedDropView<'a> {
    access: &'a dyn PreparedBlockDropsAccess,
    index: usize,
}

impl<'a> PreparedDropView<'a> {
    /// Borrow this drop's stack without encoding its components.
    /// Returns `None` if the backing stack cannot be represented as a nonempty mod-API stack.
    pub fn stack(&self) -> Option<ItemStackView<'a>> {
        let metadata = self.access.stack_metadata(self.index)?;
        Some(ItemStackView::prepared_drop(
            self.index,
            metadata,
            self.access,
        ))
    }

    /// Where the item entity will appear, in blocks.
    pub fn position(&self) -> [f64; 3] {
        self.access.position(self.index).unwrap_or([0.0; 3])
    }

    /// The item entity's motion in blocks per tick.
    pub fn velocity(&self) -> [f64; 3] {
        self.access.velocity(self.index).unwrap_or([0.0; 3])
    }

    /// Ticks before a player may pick up the item entity.
    pub fn pickup_delay(&self) -> u32 {
        self.access.pickup_delay(self.index).unwrap_or(0)
    }

    /// `ItemEntity.setThrower`: the UUID of the entity that threw it, if any.
    pub fn thrower(&self) -> Option<u128> {
        self.access.thrower(self.index).flatten()
    }

    /// `ItemEntity.setTarget`: the UUID of the only player who may pick it up, if any.
    pub fn target(&self) -> Option<u128> {
        self.access.target(self.index).flatten()
    }

    /// `Entity.setNoGravity`: whether the item entity ignores gravity.
    pub fn no_gravity(&self) -> bool {
        self.access.no_gravity(self.index).unwrap_or(false)
    }

    /// The item entity's starting `age` in ticks
    /// ([`ItemDrop::UNLIMITED_LIFETIME`] never despawns).
    pub fn age(&self) -> i32 {
        self.access.age(self.index).unwrap_or(0)
    }

    /// Copy this prepared drop into an owned [`ItemDrop`], projecting its stack's
    /// components on demand. Returns `None` if the stack cannot be projected.
    pub fn to_item_drop(&self) -> Option<ItemDrop> {
        let stack = self.stack()?.project()?;
        Some(
            ItemDrop::new(stack, self.position(), self.velocity(), self.pickup_delay())
                .with_thrower(self.thrower())
                .with_target(self.target())
                .with_no_gravity(self.no_gravity())
                .with_age(self.age()),
        )
    }
}

/// A mutable view of one prepared block drop.
///
/// Updates mutate the prepared drop in place. Stack writes are validated
/// individually; a refused [`Self::set_stack`] returns `Err` and leaves the
/// existing stack intact.
// exhaustive: only PreparedBlockDrops constructs a mutable prepared-drop view.
// no wire: mutably borrows one entry from the live prepared block-drops storage.
pub struct PreparedDropMut<'a> {
    access: &'a mut dyn PreparedBlockDropsAccess,
    index: usize,
}

impl PreparedDropMut<'_> {
    /// Borrow this drop as a read-only [`PreparedDropView`].
    pub fn as_view(&self) -> PreparedDropView<'_> {
        PreparedDropView {
            access: &*self.access,
            index: self.index,
        }
    }

    /// Borrow this drop's stack without encoding its components.
    /// Returns `None` if the backing stack cannot be represented as a nonempty mod-API stack.
    pub fn stack(&self) -> Option<ItemStackView<'_>> {
        self.as_view().stack()
    }

    /// Where the item entity will appear, in blocks.
    pub fn position(&self) -> [f64; 3] {
        self.as_view().position()
    }

    /// The item entity's motion in blocks per tick.
    pub fn velocity(&self) -> [f64; 3] {
        self.as_view().velocity()
    }

    /// Ticks before a player may pick up the item entity.
    pub fn pickup_delay(&self) -> u32 {
        self.as_view().pickup_delay()
    }

    /// `ItemEntity.setThrower`: the UUID of the entity that threw it, if any.
    pub fn thrower(&self) -> Option<u128> {
        self.as_view().thrower()
    }

    /// `ItemEntity.setTarget`: the UUID of the only player who may pick it up, if any.
    pub fn target(&self) -> Option<u128> {
        self.as_view().target()
    }

    /// `Entity.setNoGravity`: whether the item entity ignores gravity.
    pub fn no_gravity(&self) -> bool {
        self.as_view().no_gravity()
    }

    /// The item entity's starting `age` in ticks
    /// ([`ItemDrop::UNLIMITED_LIFETIME`] never despawns).
    pub fn age(&self) -> i32 {
        self.as_view().age()
    }

    /// Copy this prepared drop into an owned [`ItemDrop`], projecting its stack's
    /// components on demand. Returns `None` if the stack cannot be projected.
    pub fn to_item_drop(&self) -> Option<ItemDrop> {
        self.as_view().to_item_drop()
    }

    /// Replace this drop's stack in place.
    /// Returns `Err(StackWriteError)` and leaves the existing stack intact if `stack` is invalid.
    pub fn set_stack(&mut self, stack: ItemStack) -> Result<(), StackWriteError> {
        if self.access.set_stack(self.index, stack)? {
            Ok(())
        } else {
            Err(StackWriteError::Unavailable)
        }
    }

    /// Set where the item entity will appear, in blocks.
    pub fn set_position(&mut self, position: [f64; 3]) {
        self.access.set_position(self.index, position);
    }

    /// Set the item entity's motion in blocks per tick.
    pub fn set_velocity(&mut self, velocity: [f64; 3]) {
        self.access.set_velocity(self.index, velocity);
    }

    /// Set ticks before a player may pick up the item entity.
    pub fn set_pickup_delay(&mut self, pickup_delay: u32) {
        self.access.set_pickup_delay(self.index, pickup_delay);
    }

    /// Set `ItemEntity.setThrower`: the UUID of the entity that threw it.
    pub fn set_thrower(&mut self, thrower: Option<u128>) {
        self.access.set_thrower(self.index, thrower);
    }

    /// Set `ItemEntity.setTarget`: the UUID of the only player who may pick it up.
    pub fn set_target(&mut self, target: Option<u128>) {
        self.access.set_target(self.index, target);
    }

    /// Set `Entity.setNoGravity`: whether the item entity ignores gravity.
    pub fn set_no_gravity(&mut self, no_gravity: bool) {
        self.access.set_no_gravity(self.index, no_gravity);
    }

    /// Set the item entity's starting `age` in ticks
    /// ([`ItemDrop::UNLIMITED_LIFETIME`] never despawns).
    pub fn set_age(&mut self, age: i32) {
        self.access.set_age(self.index, age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Compound, JavaString, Tag};
    use std::cell::RefCell;

    const REJECTED_ITEM: ItemKey = ItemKey(999);

    #[derive(Clone, Debug)]
    struct FakeDrop {
        id: u32,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
        thrower: Option<u128>,
        target: Option<u128>,
        no_gravity: bool,
        age: i32,
    }

    impl FakeDrop {
        fn new(id: u32, stack: ItemStack, position: [f64; 3]) -> Self {
            Self {
                id,
                stack,
                position,
                velocity: [0.05, 0.2, -0.05],
                pickup_delay: 10,
                thrower: None,
                target: None,
                no_gravity: false,
                age: 0,
            }
        }

        fn from_item_drop(id: u32, drop: ItemDrop) -> Self {
            Self {
                id,
                stack: drop.stack,
                position: drop.position,
                velocity: drop.velocity,
                pickup_delay: drop.pickup_delay,
                thrower: drop.thrower,
                target: drop.target,
                no_gravity: drop.no_gravity,
                age: drop.age,
            }
        }
    }

    struct FakePreparedDrops {
        entries: Vec<FakeDrop>,
        next_id: u32,
        projected: RefCell<Vec<usize>>,
    }

    impl FakePreparedDrops {
        fn new(entries: Vec<FakeDrop>) -> Self {
            let next_id = entries.iter().map(|entry| entry.id).max().unwrap_or(0) + 1;
            Self {
                entries,
                next_id,
                projected: RefCell::new(Vec::new()),
            }
        }

        fn validate_stack(stack: &ItemStack) -> Result<(), StackWriteError> {
            if stack.item() == REJECTED_ITEM {
                return Err(StackWriteError::UnknownItem);
            }
            if stack.count() > 64 {
                return Err(StackWriteError::CountExceeded { maximum: 64 });
            }
            Ok(())
        }

        fn ids(&self) -> Vec<u32> {
            self.entries.iter().map(|entry| entry.id).collect()
        }
    }

    impl PreparedBlockDropsAccess for FakePreparedDrops {
        fn len(&self) -> usize {
            self.entries.len()
        }

        fn stack_metadata(&self, index: usize) -> Option<(ItemKey, NonZeroU32, Option<u32>)> {
            Some(self.entries.get(index)?.stack.read_metadata())
        }

        fn project_stack(
            &self,
            index: usize,
            metadata: (ItemKey, NonZeroU32, Option<u32>),
        ) -> Option<ItemStack> {
            let entry = self.entries.get(index)?;
            self.projected.borrow_mut().push(index);
            assert_eq!(entry.stack.read_metadata(), metadata);
            Some(entry.stack.clone())
        }

        fn position(&self, index: usize) -> Option<[f64; 3]> {
            self.entries.get(index).map(|entry| entry.position)
        }

        fn velocity(&self, index: usize) -> Option<[f64; 3]> {
            self.entries.get(index).map(|entry| entry.velocity)
        }

        fn pickup_delay(&self, index: usize) -> Option<u32> {
            self.entries.get(index).map(|entry| entry.pickup_delay)
        }

        fn thrower(&self, index: usize) -> Option<Option<u128>> {
            self.entries.get(index).map(|entry| entry.thrower)
        }

        fn target(&self, index: usize) -> Option<Option<u128>> {
            self.entries.get(index).map(|entry| entry.target)
        }

        fn no_gravity(&self, index: usize) -> Option<bool> {
            self.entries.get(index).map(|entry| entry.no_gravity)
        }

        fn age(&self, index: usize) -> Option<i32> {
            self.entries.get(index).map(|entry| entry.age)
        }

        fn set_stack(&mut self, index: usize, stack: ItemStack) -> Result<bool, StackWriteError> {
            if index >= self.entries.len() {
                return Ok(false);
            }
            Self::validate_stack(&stack)?;
            self.entries[index].stack = stack;
            Ok(true)
        }

        fn set_position(&mut self, index: usize, position: [f64; 3]) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.position = position;
            true
        }

        fn set_velocity(&mut self, index: usize, velocity: [f64; 3]) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.velocity = velocity;
            true
        }

        fn set_pickup_delay(&mut self, index: usize, pickup_delay: u32) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.pickup_delay = pickup_delay;
            true
        }

        fn set_thrower(&mut self, index: usize, thrower: Option<u128>) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.thrower = thrower;
            true
        }

        fn set_target(&mut self, index: usize, target: Option<u128>) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.target = target;
            true
        }

        fn set_no_gravity(&mut self, index: usize, no_gravity: bool) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.no_gravity = no_gravity;
            true
        }

        fn set_age(&mut self, index: usize, age: i32) -> bool {
            let Some(entry) = self.entries.get_mut(index) else {
                return false;
            };
            entry.age = age;
            true
        }

        fn push(&mut self, drop: ItemDrop) -> Result<(), StackWriteError> {
            Self::validate_stack(&drop.stack)?;
            let id = self.next_id;
            self.next_id += 1;
            self.entries.push(FakeDrop::from_item_drop(id, drop));
            Ok(())
        }

        fn insert(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError> {
            if index > self.entries.len() {
                return Ok(false);
            }
            Self::validate_stack(&drop.stack)?;
            let id = self.next_id;
            self.next_id += 1;
            self.entries
                .insert(index, FakeDrop::from_item_drop(id, drop));
            Ok(true)
        }

        fn replace(&mut self, index: usize, drop: ItemDrop) -> Result<bool, StackWriteError> {
            if index >= self.entries.len() {
                return Ok(false);
            }
            Self::validate_stack(&drop.stack)?;
            let id = self.next_id;
            self.next_id += 1;
            self.entries[index] = FakeDrop::from_item_drop(id, drop);
            Ok(true)
        }

        fn remove(&mut self, index: usize) -> bool {
            if index >= self.entries.len() {
                return false;
            }
            self.entries.remove(index);
            true
        }

        fn clear(&mut self) {
            self.entries.clear();
        }

        fn truncate(&mut self, len: usize) {
            self.entries.truncate(len);
        }

        fn swap(&mut self, a: usize, b: usize) -> bool {
            if a >= self.entries.len() || b >= self.entries.len() {
                return false;
            }
            self.entries.swap(a, b);
            true
        }
    }

    #[test]
    fn scalar_reads_never_project_and_projection_is_lazy() {
        let mut first_stack = ItemStack::new(ItemKey(1), 3).unwrap();
        first_stack.set_damage(5);
        let mut patch = Compound::new();
        patch.insert(JavaString::from("test:marker"), Tag::Int(42));
        first_stack.set_components(patch).unwrap();

        let mut first = FakeDrop::new(10, first_stack.clone(), [1.5, 64.5, -2.5]);
        first.velocity = [0.1, 0.25, -0.15];
        first.pickup_delay = 20;
        first.thrower = Some(11);
        first.target = Some(22);
        first.no_gravity = true;
        first.age = 15;

        let second_stack = ItemStack::of(ItemKey(2));
        let second = FakeDrop::new(20, second_stack.clone(), [2.5, 65.0, 3.5]);

        let mut backing = FakePreparedDrops::new(vec![first, second]);
        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            assert!(!drops.is_empty());
            assert_eq!(drops.len(), 2);
            assert!(drops.get(2).is_none());
            assert!(drops.get_mut(2).is_none());

            let items: Vec<_> = drops
                .iter()
                .map(|drop| {
                    let stack = drop.stack().unwrap();
                    (
                        stack.item(),
                        stack.count(),
                        stack.damage(),
                        stack.damage_component(),
                        drop.position(),
                        drop.velocity(),
                        drop.pickup_delay(),
                        drop.thrower(),
                        drop.target(),
                        drop.no_gravity(),
                        drop.age(),
                    )
                })
                .collect();
            assert_eq!(
                items,
                [
                    (
                        ItemKey(1),
                        3,
                        5,
                        Some(5),
                        [1.5, 64.5, -2.5],
                        [0.1, 0.25, -0.15],
                        20,
                        Some(11),
                        Some(22),
                        true,
                        15,
                    ),
                    (
                        ItemKey(2),
                        1,
                        0,
                        None,
                        [2.5, 65.0, 3.5],
                        [0.05, 0.2, -0.05],
                        10,
                        None,
                        None,
                        false,
                        0,
                    ),
                ]
            );
        }
        assert!(backing.projected.borrow().is_empty());

        {
            let drops = PreparedBlockDrops::new(&mut backing);
            let projected_stack = drops.get(0).unwrap().stack().unwrap().project().unwrap();
            assert_eq!(projected_stack, first_stack);
        }
        assert_eq!(*backing.projected.borrow(), [0]);

        {
            let drops = PreparedBlockDrops::new(&mut backing);
            let copied_first = drops.get(0).unwrap().to_item_drop().unwrap();
            assert_eq!(
                copied_first,
                ItemDrop::new(first_stack, [1.5, 64.5, -2.5], [0.1, 0.25, -0.15], 20)
                    .with_thrower(Some(11))
                    .with_target(Some(22))
                    .with_no_gravity(true)
                    .with_age(15)
            );
            let copied_second = drops.get(1).unwrap().to_item_drop().unwrap();
            assert_eq!(
                copied_second,
                ItemDrop::new(second_stack, [2.5, 65.0, 3.5], [0.05, 0.2, -0.05], 10)
            );
        }
        assert_eq!(*backing.projected.borrow(), [0, 0, 1]);
    }

    #[test]
    fn drop_mut_updates_live_and_preserves_existing_stack_on_error() {
        let initial_stack = ItemStack::new(ItemKey(1), 4).unwrap();
        let mut backing =
            FakePreparedDrops::new(vec![FakeDrop::new(10, initial_stack, [0.5, 70.5, 0.5])]);

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            let mut entry = drops.get_mut(0).unwrap();
            assert_eq!(entry.as_view().stack().unwrap().item(), ItemKey(1));
            assert_eq!(entry.stack().unwrap().count(), 4);
            assert_eq!(entry.position(), [0.5, 70.5, 0.5]);
            assert_eq!(entry.velocity(), [0.05, 0.2, -0.05]);
            assert_eq!(entry.pickup_delay(), 10);
            assert_eq!(entry.thrower(), None);
            assert_eq!(entry.target(), None);
            assert!(!entry.no_gravity());
            assert_eq!(entry.age(), 0);

            entry.set_position([3.0, 71.0, -4.0]);
            entry.set_velocity([0.2, 0.3, 0.4]);
            entry.set_pickup_delay(40);
            entry.set_thrower(Some(101));
            entry.set_target(Some(202));
            entry.set_no_gravity(true);
            entry.set_age(ItemDrop::UNLIMITED_LIFETIME);

            assert_eq!(
                entry.set_stack(ItemStack::of(REJECTED_ITEM)),
                Err(StackWriteError::UnknownItem)
            );
            assert_eq!(
                entry.set_stack(ItemStack::new(ItemKey(3), 65).unwrap()),
                Err(StackWriteError::CountExceeded { maximum: 64 })
            );
            assert_eq!(entry.stack().unwrap().item(), ItemKey(1));
            assert_eq!(entry.stack().unwrap().count(), 4);

            let replacement = ItemStack::new(ItemKey(3), 12).unwrap();
            entry.set_stack(replacement.clone()).unwrap();
            assert_eq!(
                entry.to_item_drop().unwrap(),
                ItemDrop::new(replacement, [3.0, 71.0, -4.0], [0.2, 0.3, 0.4], 40)
                    .with_thrower(Some(101))
                    .with_target(Some(202))
                    .with_no_gravity(true)
                    .with_age(ItemDrop::UNLIMITED_LIFETIME)
            );
        }

        assert_eq!(backing.ids(), [10]);
        assert_eq!(backing.entries[0].stack.item(), ItemKey(3));
        assert_eq!(backing.entries[0].stack.count(), 12);
        assert_eq!(backing.entries[0].position, [3.0, 71.0, -4.0]);
        assert_eq!(backing.entries[0].velocity, [0.2, 0.3, 0.4]);
        assert_eq!(backing.entries[0].pickup_delay, 40);
        assert_eq!(backing.entries[0].thrower, Some(101));
        assert_eq!(backing.entries[0].target, Some(202));
        assert!(backing.entries[0].no_gravity);
        assert_eq!(backing.entries[0].age, ItemDrop::UNLIMITED_LIFETIME);
    }

    #[test]
    fn list_writes_reject_invalid_stacks_and_oob_without_panicking() {
        let mut backing = FakePreparedDrops::new(vec![
            FakeDrop::new(10, ItemStack::of(ItemKey(1)), [1.0, 0.0, 0.0]),
            FakeDrop::new(20, ItemStack::of(ItemKey(2)), [2.0, 0.0, 0.0]),
        ]);

        let invalid_unknown = ItemDrop::new(
            ItemStack::of(REJECTED_ITEM),
            [9.0, 9.0, 9.0],
            [0.0, 0.0, 0.0],
            0,
        );
        let invalid_count = ItemDrop::new(
            ItemStack::new(ItemKey(4), 65).unwrap(),
            [9.0, 9.0, 9.0],
            [0.0, 0.0, 0.0],
            0,
        );
        let valid_a = ItemDrop::new(
            ItemStack::new(ItemKey(3), 2).unwrap(),
            [3.0, 0.0, 0.0],
            [0.0, 0.1, 0.0],
            5,
        );
        let valid_b = ItemDrop::new(
            ItemStack::new(ItemKey(4), 3).unwrap(),
            [4.0, 0.0, 0.0],
            [0.0, 0.2, 0.0],
            6,
        );
        let valid_c = ItemDrop::new(
            ItemStack::new(ItemKey(5), 4).unwrap(),
            [5.0, 0.0, 0.0],
            [0.0, 0.3, 0.0],
            7,
        );

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            assert_eq!(
                drops.push(invalid_unknown.clone()),
                Err(StackWriteError::UnknownItem)
            );
            assert_eq!(
                drops.insert(0, invalid_count),
                Err(StackWriteError::CountExceeded { maximum: 64 })
            );
            assert_eq!(
                drops.replace(1, invalid_unknown.clone()),
                Err(StackWriteError::UnknownItem)
            );
            assert_eq!(drops.insert(3, valid_a.clone()), Ok(false));
            assert_eq!(drops.insert(3, invalid_unknown), Ok(false));
            assert_eq!(drops.replace(2, valid_a.clone()), Ok(false));
            assert!(!drops.remove(2));
            assert!(!drops.swap(0, 2));
            assert!(!drops.swap(2, 0));
        }
        assert_eq!(backing.ids(), [10, 20]);
        assert_eq!(backing.entries[0].stack.item(), ItemKey(1));
        assert_eq!(backing.entries[1].stack.item(), ItemKey(2));

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            drops.push(valid_a).unwrap();
            assert_eq!(drops.insert(1, valid_b), Ok(true));
            assert_eq!(drops.replace(0, valid_c), Ok(true));
            assert_eq!(drops.len(), 4);
            let order: Vec<ItemKey> = drops
                .iter()
                .map(|drop| drop.stack().unwrap().item())
                .collect();
            assert_eq!(order, [ItemKey(5), ItemKey(4), ItemKey(2), ItemKey(3)]);
        }
        assert_eq!(backing.ids(), [23, 22, 20, 21]);
    }

    #[test]
    fn remove_pop_swap_retain_truncate_clear_preserve_order_and_identity() {
        let mut backing = FakePreparedDrops::new(vec![
            FakeDrop::new(10, ItemStack::of(ItemKey(1)), [1.0, 0.0, 0.0]),
            FakeDrop::new(20, ItemStack::of(ItemKey(2)), [2.0, 0.0, 0.0]),
            FakeDrop::new(30, ItemStack::of(ItemKey(3)), [3.0, 0.0, 0.0]),
            FakeDrop::new(40, ItemStack::of(ItemKey(4)), [4.0, 0.0, 0.0]),
            FakeDrop::new(50, ItemStack::of(ItemKey(5)), [5.0, 0.0, 0.0]),
        ]);

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            assert!(drops.swap(1, 3));
            assert!(drops.remove(2));
            assert!(drops.pop());
            drops.retain(|drop| drop.stack().unwrap().item() != ItemKey(4));
        }
        assert_eq!(backing.ids(), [10, 20]);
        assert!(backing.projected.borrow().is_empty());

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            drops
                .push(ItemDrop::new(
                    ItemStack::of(ItemKey(6)),
                    [1.5, 0.0, 0.0],
                    [0.0; 3],
                    0,
                ))
                .unwrap();
            drops
                .push(ItemDrop::new(
                    ItemStack::of(ItemKey(7)),
                    [7.0, 0.0, 0.0],
                    [0.0; 3],
                    0,
                ))
                .unwrap();
            drops.retain(|drop| drop.position()[0] >= 2.0);
        }
        assert_eq!(backing.ids(), [20, 52]);
        assert!(backing.projected.borrow().is_empty());

        {
            let mut drops = PreparedBlockDrops::new(&mut backing);
            drops.truncate(10);
            assert_eq!(drops.len(), 2);
            drops.truncate(1);
            assert_eq!(drops.len(), 1);
            assert_eq!(drops.get(0).unwrap().stack().unwrap().item(), ItemKey(2));
            drops.clear();
            assert!(drops.is_empty());
            assert!(!drops.pop());
            assert!(!drops.remove(0));
        }
        assert!(backing.entries.is_empty());
    }
}
