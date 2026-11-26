//! Bulk item storage: [`STORAGE`], a block's items by kind, in 64-bit amounts and without slots,
//! for the stores, pipes and networks of a large base. Every block entity with slots answers it
//! through the game's own adapter over [`ITEMS`], so a pipe reaches a chest and a mod's drive
//! through one capability; a mod's bulk store (drawers, a drive, a vault) answers it itself, one
//! entry per kind however many items it holds.
//!
//! **Diverges from upstream: Sunrise addition.** NeoForge 26.3's transfer API has no by-kind
//! capability; `ITEMS` (`Capabilities.Item.BLOCK`) is its counterpart and stays as it is.
//!
//! What a call costs is the handler's. A bulk store keyed by kind answers a question about one
//! kind, and a move of one kind, in constant time whatever it holds. The game's own adapter
//! answers each question in one pass over the slots the side reaches (O(slots), with no work per
//! item moved); the adapter for a mod's `ITEMS` can skip empty gaps in sparse handlers. [`transfer`] is
//! four handler calls whatever it moves. [`ItemStorage::visit_entries`] reads entries in
//! provider order and stops as soon as the visitor breaks.
//!
//! **When not to use it.** A pipe that moves a few items a tick between small containers is
//! cheaper on `ITEMS` and [`move_resources`](crate::resource_util::move_resources), slot by slot: in the
//! measurements behind this API, 256 pipes moving 8 items a tick each cost 2.8 ms a tick that
//! way against 3.7 ms through `STORAGE`. `STORAGE` pays off for bulk stores, large moves, and
//! networks that sleep while idle.
use crate::content::resource::{advance_transfer_index, insert_candidate};
use crate::resource_util::visit_slots;
use crate::{
    BlockSide, Capability, CapabilityKey, HostTransaction as _, ITEMS, ItemResource,
    ResourceHandler, ServerHost, TransactionContext, TransactionError,
};
use std::{collections::HashSet, marker::PhantomData, ops::ControlFlow};

/// Why a [`STORAGE`] question or move was refused.
// no wire: reports refusals from in-process storage reads and transfers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageError {
    /// The supplied context does not identify this host's innermost open transaction.
    InvalidContext,
    /// A total or an amount does not fit in 64 bits.
    Overflow,
    /// A handler answered more than it was asked: a mod bug, warned once naming the mod. The
    /// move it was part of is undone.
    Overanswered {
        /// The mod whose handler answered, or "the game".
        owner: String,
    },
    /// The host refused to open the transfer savepoint.
    TransactionUnavailable,
    /// The savepoint ran, but the host rolled back its requested commit.
    CommitRefused,
    /// The handler could not answer.
    Unavailable {
        /// The provider whose handler was asked, or "the game".
        owner: String,
    },
}

impl From<(crate::Identifier, String)> for StorageError {
    fn from((owner, _): (crate::Identifier, String)) -> Self {
        Self::Unavailable {
            owner: owner.to_string(),
        }
    }
}

impl From<crate::Overanswered> for StorageError {
    fn from(error: crate::Overanswered) -> Self {
        Self::Overanswered {
            owner: error.owner.unwrap_or_else(|| "unknown provider".into()),
        }
    }
}

/// A block's items by kind: how much of one kind it holds and has room for, and its writes, by
/// kind and in transactions, as a [`ResourceHandler`] is written. Stateless and given the host,
/// as every capability handler is. Writes require the innermost context on that host.
/// Amounts are 64-bit: a million items is one call.
pub trait ItemStorage {
    /// How much of `resource` it holds.
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError>;
    /// A planning bound for `resource`, not an admission decision. The game's default
    /// handler does not ask `BlockEntityPart::can_place_item` here; an insert checks the
    /// actual offer and may take less.
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError>;
    /// Puts up to `amount` of `resource` in, in transaction `tx`: how much went in, never more
    /// than `amount`. A mismatched `tx` returns `Err(StorageError::InvalidContext)`.
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError>;
    /// Takes up to `amount` of `resource` out, in transaction `tx`: how much came out, never more
    /// than `amount`. A mismatched `tx` returns `Err(StorageError::InvalidContext)`.
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError>;
    /// Visits occupied entries in provider order. For a container, this is the order of the
    /// slots exposed on the queried side, not necessarily physical slot order.
    ///
    /// Providers skip empty entries and report repeated kinds separately, each with its own
    /// amount. Read entries as needed: `ControlFlow::Continue(())` asks for the next entry;
    /// `ControlFlow::Break(())` returns `Ok(())` without reading another. Exhaustion also
    /// returns `Ok(())`; a read failure returns `Err`.
    ///
    /// The provider may hold a read borrow for the duration of the visit. Use each kind only
    /// within its callback or clone it to keep it. Do not re-enter the provider or write from
    /// the callback: wait until the visit returns `Ok(())`.
    ///
    /// ```
    /// use mod_api::{ItemResource, ItemStorage, ServerHost, StorageError};
    /// use std::ops::ControlFlow;
    ///
    /// fn first_at_least(
    ///     storage: &dyn ItemStorage,
    ///     host: &mut dyn ServerHost,
    ///     minimum: u64,
    /// ) -> Result<Option<(ItemResource, u64)>, StorageError> {
    ///     let mut found = None;
    ///     storage.visit_entries(host, &mut |kind, amount| {
    ///         if amount >= minimum {
    ///             found = Some((kind.clone(), amount));
    ///             ControlFlow::Break(())
    ///         } else {
    ///             ControlFlow::Continue(())
    ///         }
    ///     })?;
    ///     Ok(found)
    /// }
    /// ```
    fn visit_entries(
        &self,
        host: &mut dyn ServerHost,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
    ) -> Result<(), StorageError>;
}

const STORAGE_SOURCE: &CapabilityKey<dyn ResourceHandler<ItemResource>> = &ITEMS;

/// The default storage handler adapts this capability, including its mod providers.
pub(crate) fn default_source(name: &str) -> Option<&'static str> {
    (name == STORAGE.name).then_some(STORAGE_SOURCE.name)
}

/// Items by kind: a container's or composter port's [`ItemStorage`] as one side reaches it.
/// The game answers it for every container or port [`ITEMS`] reaches, from those slots; a mod's
/// bulk store answers it with a provider of its own (`block_entity_capability(&STORAGE, ..)`).
/// A block with slots and a store of its own registers its provider `before_game`, or the slots
/// answer first.
pub const STORAGE: CapabilityKey<dyn ItemStorage> = CapabilityKey {
    name: "sunrise:storage",
    default: Some(crate::BlockCapabilityDefault::Server(|host, at| {
        let slots = at.level.capability(host, STORAGE_SOURCE, at.pos, at.side)?;
        // The game's own slots have a direct path; use handler traversal for mod ITEMS.
        Some(match slots.owner {
            None => Box::new(GameStorage(at)) as Box<dyn ItemStorage>,
            Some(_) => Box::new(Slots(slots)),
        })
    })),
    handler: PhantomData,
};

/// The game's own [`STORAGE`] for container or composter slots: each question and write one
/// pass over the slots `.side` reaches, in the game, by the rules hoppers follow.
struct GameStorage(BlockSide);

impl ItemStorage for Capability<dyn ItemStorage> {
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        self.handler.amount(host, resource)
    }
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        self.handler.space_for(host, resource)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        let moved = self.handler.insert(host, tx, resource, amount)?;
        self.checked(moved, amount, resource)
            .ok_or_else(|| StorageError::Overanswered {
                owner: self.owner(),
            })
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        let moved = self.handler.extract(host, tx, resource, amount)?;
        self.checked(moved, amount, resource)
            .ok_or_else(|| StorageError::Overanswered {
                owner: self.owner(),
            })
    }
    fn visit_entries(
        &self,
        host: &mut dyn ServerHost,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
    ) -> Result<(), StorageError> {
        self.handler.visit_entries(host, visitor)
    }
}

impl ItemStorage for GameStorage {
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        crate::backend_of(&*host)
            .map_or(Some(0), |b| b.container_kind_amount(self.0, resource))
            .ok_or(StorageError::Overflow)
    }
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        crate::backend_of(&*host)
            .map_or(Some(0), |b| b.container_kind_space(self.0, resource))
            .ok_or(StorageError::Overflow)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        Ok(crate::backend_mut_of(host)
            .map_or(0, |b| b.container_insert_kind(self.0, resource, amount, tx))
            .min(amount))
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        Ok(crate::backend_mut_of(host)
            .map_or(0, |b| {
                b.container_extract_kind(self.0, resource, amount, tx)
            })
            .min(amount))
    }
    fn visit_entries(
        &self,
        host: &mut dyn ServerHost,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
    ) -> Result<(), StorageError> {
        crate::backend_of(&*host)
            .ok_or(crate::LevelReadError::Unavailable)
            .and_then(|backend| backend.container_visit_entries(self.0, visitor))
            .map_err(|_| StorageError::Unavailable {
                owner: "the game".into(),
            })
    }
}

/// [`STORAGE`] over a mod's own [`ITEMS`] handler. It inserts as
/// `SimpleContainer.addItem` orders it: the slots that hold the kind first, then the empty ones.
struct Slots(Capability<dyn ResourceHandler<ItemResource>>);

impl Slots {
    /// What slot `index` holds when it holds `resource`.
    fn holds(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> bool {
        self.0.resource(host, index).as_ref() == Some(resource)
    }
}

impl ItemStorage for Slots {
    fn amount(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        let mut held = 0u64;
        let mut indices = 0..self.0.size(host);
        while let Some(index) = advance_transfer_index(&self.0, host, &mut indices, None) {
            if self.holds(host, index, resource) {
                held = held
                    .checked_add(self.0.amount(host, index) as u64)
                    .ok_or(StorageError::Overflow)?;
            }
        }
        Ok(held)
    }
    fn space_for(
        &self,
        host: &mut dyn ServerHost,
        resource: &ItemResource,
    ) -> Result<u64, StorageError> {
        let mut room = 0u64;
        visit_slots(host, &self.0, |host, slots, held| {
            let index = slots.start;
            let here = match held {
                Some(held) if held == *resource => self
                    .0
                    .capacity(host, index, Some(resource))
                    .saturating_sub(self.0.amount(host, index)),
                Some(_) => 0,
                None if self.0.is_valid(host, index, resource) => {
                    self.0.capacity(host, index, Some(resource))
                }
                None => 0,
            };
            let space = u64::from(here)
                .checked_mul(slots.len() as u64)
                .ok_or(StorageError::Overflow)?;
            room = room.checked_add(space).ok_or(StorageError::Overflow)?;
            Ok::<_, StorageError>(())
        })?;
        Ok(room)
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        let mut left = amount;
        for (inserting, held) in [(None, Some(resource)), (Some(resource), None)] {
            if left == 0 {
                break;
            }
            let mut indices = 0..self.0.size(host);
            while left > 0 {
                let Some(index) = advance_transfer_index(&self.0, host, &mut indices, inserting)
                else {
                    break;
                };
                if self.0.resource(host, index).as_ref() == held {
                    let ask = left.min(u64::from(u32::MAX)) as u32;
                    let (wrote, next) =
                        insert_candidate(&self.0, host, tx, index..indices.end, resource, ask)
                            .map_err(storage_error_from_tx)?;
                    indices.start = next;
                    left -= u64::from(wrote.min(ask));
                }
            }
        }
        Ok(amount - left)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u64,
    ) -> Result<u64, StorageError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(StorageError::InvalidContext);
        }
        let mut left = amount;
        let mut indices = 0..self.0.size(host);
        while left > 0 {
            let Some(index) = advance_transfer_index(&self.0, host, &mut indices, None) else {
                break;
            };
            if self.holds(host, index, resource) {
                let ask = left.min(u32::MAX as u64) as u32;
                let moved = self
                    .0
                    .extract_at(host, tx, index, resource, ask)
                    .map_err(storage_error_from_tx)?;
                left -= moved.min(ask) as u64;
            }
        }
        Ok(amount - left)
    }
    fn visit_entries(
        &self,
        host: &mut dyn ServerHost,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
    ) -> Result<(), StorageError> {
        let mut indices = 0..self.0.size(host);
        while let Some(index) = advance_transfer_index(&self.0, host, &mut indices, None) {
            if let Some(resource) = self.0.resource(host, index) {
                let amount = u64::from(self.0.amount(host, index));
                if visitor(&resource, amount).is_break() {
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Moves up to `max` of `resource` out of `from` into `to`, in transaction `tx`: how much moved.
/// Extract from `from` first, then offer the extracted amount to `to` in the same journal. If
/// `to` takes only part, roll back and retry once with that amount. A handler that answers more
/// than it was asked rolls back the move with [`StorageError::Overanswered`]. If `from` has none
/// or `to` has no room, neither is asked to transfer anything. A refused transfer savepoint
/// returns [`StorageError::TransactionUnavailable`]. An invalid loan returns
/// [`StorageError::InvalidContext`] before querying either provider.
pub fn transfer(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ItemStorage>,
    to: &Capability<dyn ItemStorage>,
    resource: &ItemResource,
    max: u64,
    tx: &TransactionContext,
) -> Result<u64, StorageError> {
    if !host.transaction_matches(Some(tx)) {
        return Err(StorageError::InvalidContext);
    }
    let want = max
        .min(from.amount(host, resource)?)
        .min(to.space_for(host, resource)?);
    if want == 0 {
        return Ok(0);
    }
    let step = |host: &mut dyn ServerHost, want: u64| {
        host.transaction(|host, tx| {
            let out = from.extract(host, tx, resource, want).map_err(Err)?;
            let took = to.insert(host, tx, resource, out).map_err(Err)?;
            if took == out && took > 0 {
                Ok(out)
            } else {
                Err(Ok(took))
            }
        })
    };
    match step(host, want) {
        Err(TransactionError::Aborted(Ok(took))) if took > 0 => step(host, took),
        answer => answer,
    }
    .or_else(|error| match error {
        TransactionError::InvalidContext => Err(StorageError::InvalidContext),
        TransactionError::Unavailable => Err(StorageError::TransactionUnavailable),
        TransactionError::CommitRefused => Err(StorageError::CommitRefused),
        TransactionError::Aborted(Ok(_)) => Ok(0),
        TransactionError::Aborted(Err(error)) => Err(error),
    })
}

fn storage_error_from_tx(error: TransactionError<crate::Overanswered>) -> StorageError {
    match error {
        TransactionError::Aborted(over) => over.into(),
        TransactionError::InvalidContext => StorageError::InvalidContext,
        TransactionError::Unavailable => StorageError::TransactionUnavailable,
        TransactionError::CommitRefused => StorageError::CommitRefused,
    }
}

/// Moves up to `max` of what `filter` accepts out of `from` into `to` (NeoForge's
/// `ResourceHandlerUtil.move`, by kind), in transaction `tx`: how much moved. A zero `max` does
/// not read the source. Visits the source once in provider order to collect matching kinds
/// (deduplicated by kind, since [`transfer`] moves across the whole storage by kind). After the
/// visit returns and the provider's read borrow has ended, transfers each collected kind once in
/// first-seen order until `max` items have moved or the list ends.
pub fn transfer_matching(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ItemStorage>,
    to: &Capability<dyn ItemStorage>,
    filter: impl Fn(&ItemResource) -> bool,
    max: u64,
    tx: &TransactionContext,
) -> Result<u64, StorageError> {
    if !host.transaction_matches(Some(tx)) {
        return Err(StorageError::InvalidContext);
    }
    if max == 0 {
        return Ok(0);
    }
    let mut seen = HashSet::new();
    let mut matched = Vec::new();
    from.visit_entries(host, &mut |resource, _| {
        if filter(resource) && !seen.contains(resource) {
            let owned = resource.clone();
            seen.insert(owned.clone());
            matched.push(owned);
        }
        ControlFlow::Continue(())
    })?;
    let mut moved = 0;
    for resource in matched {
        if moved == max {
            break;
        }
        moved += transfer(host, from, to, &resource, max - moved, tx)?;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::ItemsHost;
    use crate::testing::FakeHost;
    use crate::{HostTransaction, Identifier, ItemStack, Tag, TransactionError};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::hash::BuildHasher;
    use std::rc::Rc;

    fn host_with(items: &[&str]) -> (FakeHost, Vec<ItemResource>) {
        let mut host = FakeHost::new();
        let ids: Vec<_> = items
            .iter()
            .map(|id| Identifier::parse(id).unwrap())
            .collect();
        host.registries.resolve_items(ids.clone()).unwrap();
        for id in items {
            host.registries
                .item_component(id, "minecraft:max_stack_size", Tag::Int(64));
        }
        let resources = ids
            .iter()
            .map(|id| ItemResource::of(&ItemStack::new(host.item_key(id).unwrap(), 1).unwrap()))
            .collect();
        (host, resources)
    }

    fn chest(host: &mut FakeHost, pos: [i32; 3], slots: &[Option<(&ItemResource, u32)>]) {
        let entity = host.level.block_entity(pos).with_items(slots.len());
        for (slot, held) in slots.iter().enumerate() {
            entity.items.as_mut().unwrap().slots[slot] = held.map(|(r, n)| r.to_stack(n).unwrap());
        }
    }

    fn storage_at(host: &mut FakeHost, pos: [i32; 3]) -> Capability<dyn ItemStorage> {
        let level = host.level.level();
        let server: &mut dyn ServerHost = host;
        level.capability(server, &STORAGE, pos, None).unwrap()
    }

    /// A bulk store in memory, counting the calls made to it: no transactions, for the cases
    /// that never abort.
    #[derive(Default)]
    struct Bulk {
        held: RefCell<HashMap<ItemResource, u64>>,
        room: u64,
        calls: Cell<u32>,
    }
    struct Shared(std::rc::Rc<Bulk>);
    impl ItemStorage for Shared {
        fn amount(&self, _: &mut dyn ServerHost, r: &ItemResource) -> Result<u64, StorageError> {
            self.0.calls.set(self.0.calls.get() + 1);
            Ok(self.0.held.borrow().get(r).copied().unwrap_or(0))
        }
        fn space_for(&self, _: &mut dyn ServerHost, _: &ItemResource) -> Result<u64, StorageError> {
            self.0.calls.set(self.0.calls.get() + 1);
            Ok(self.0.room - self.0.held.borrow().values().sum::<u64>())
        }
        fn insert(
            &self,
            h: &mut dyn ServerHost,
            _: &TransactionContext,
            r: &ItemResource,
            n: u64,
        ) -> Result<u64, StorageError> {
            let went = n.min(self.space_for(h, r)?);
            *self.0.held.borrow_mut().entry(r.clone()).or_default() += went;
            Ok(went)
        }
        fn extract(
            &self,
            h: &mut dyn ServerHost,
            _: &TransactionContext,
            r: &ItemResource,
            n: u64,
        ) -> Result<u64, StorageError> {
            let came = n.min(self.amount(h, r)?);
            *self.0.held.borrow_mut().entry(r.clone()).or_default() -= came;
            Ok(came)
        }
        fn visit_entries(
            &self,
            _: &mut dyn ServerHost,
            visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
        ) -> Result<(), StorageError> {
            for (resource, &amount) in self.0.held.borrow().iter() {
                if amount != 0 && visitor(resource, amount).is_break() {
                    break;
                }
            }
            Ok(())
        }
    }
    fn bulk(
        room: u64,
        held: &[(&ItemResource, u64)],
    ) -> (std::rc::Rc<Bulk>, Capability<dyn ItemStorage>) {
        let store = std::rc::Rc::new(Bulk {
            held: RefCell::new(held.iter().map(|(r, n)| ((*r).clone(), *n)).collect()),
            room,
            ..Default::default()
        });
        let handler = Capability {
            handler: Box::new(Shared(store.clone())) as Box<dyn ItemStorage>,
            owner: None,
        };
        (store, handler)
    }

    #[test]
    fn wrong_context_does_not_reach_storage_providers() {
        let (mut host, items) = host_with(&["minecraft:stone"]);
        let stone = &items[0];
        let (source, from) = bulk(64, &[(stone, 5)]);
        let (target, to) = bulk(64, &[]);
        host.transaction(|host, parent| {
            host.transaction(|host, _| {
                assert_eq!(
                    from.extract(host, parent, stone, 1),
                    Err(StorageError::InvalidContext)
                );
                assert_eq!(
                    to.insert(host, parent, stone, 1),
                    Err(StorageError::InvalidContext)
                );
                assert_eq!(
                    transfer(host, &from, &to, stone, 1, parent),
                    Err(StorageError::InvalidContext)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(source.calls.get(), 0);
        assert_eq!(target.calls.get(), 0);
    }

    #[test]
    fn the_game_answers_storage_for_slots_by_kind() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let (stone, dirt) = (&r[0], &r[1]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((stone, 40)), Some((dirt, 5)), Some((stone, 10)), None],
        );
        let storage = storage_at(&mut host, [0, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        assert_eq!(storage.amount(host, stone), Ok(50));
        assert_eq!(storage.amount(host, dirt), Ok(5));
        // 24 and 54 topping up the stone, 64 in the empty slot.
        assert_eq!(storage.space_for(host, stone), Ok(24 + 54 + 64));
        let went = host.transaction(|host, tx| storage.insert(host, tx, stone, 500));
        assert_eq!(went, Ok(142), "no more than the room");
        assert_eq!(storage.amount(host, stone), Ok(192));
        assert_eq!(
            storage.amount(host, dirt),
            Ok(5),
            "the dirt's slot is not touched"
        );
        let came = host.transaction(|host, tx| storage.extract(host, tx, stone, 100));
        assert_eq!(came, Ok(100));
        let mut total = 0;
        storage
            .visit_entries(host, &mut |_, amount| {
                total += amount;
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(total, 97);
    }

    #[test]
    fn a_slot_write_aborted_with_its_transaction_is_put_back() {
        let (mut host, r) = host_with(&["minecraft:stone"]);
        chest(&mut host, [0, 64, 0], &[Some((&r[0], 10)), None]);
        let storage = storage_at(&mut host, [0, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        let aborted: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
            assert_eq!(storage.insert(host, tx, &r[0], 70), Ok(70));
            Err(())
        });
        assert!(aborted.is_err());
        assert_eq!(storage.amount(host, &r[0]), Ok(10));
    }

    #[test]
    fn a_visit_keeps_all_entries_in_order_with_duplicate_kinds() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let slots: Vec<_> = (1..=300)
            .flat_map(|amount| [None, Some((&r[(amount % 2) as usize], amount))])
            .collect();
        chest(&mut host, [0, 64, 0], &slots);
        let storage = storage_at(&mut host, [0, 64, 0]);
        let mut entries = Vec::new();
        storage
            .visit_entries(&mut host, &mut |resource, amount| {
                entries.push((resource.clone(), amount));
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(
            entries,
            (1..=300u64)
                .map(|amount| (r[(amount % 2) as usize].clone(), amount))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_slots_visit_probes_only_through_the_first_match() {
        #[derive(Default)]
        struct Reads {
            resources: RefCell<Vec<usize>>,
            amounts: RefCell<Vec<usize>>,
        }
        struct Probe {
            entries: [Option<(ItemResource, u32)>; 3],
            reads: Rc<Reads>,
        }

        impl ResourceHandler<ItemResource> for Probe {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                self.entries.len()
            }
            fn resource(&self, _: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
                self.reads.resources.borrow_mut().push(index);
                self.entries[index].as_ref().map(|(kind, _)| kind.clone())
            }
            fn amount(&self, _: &mut dyn ServerHost, index: usize) -> u32 {
                self.reads.amounts.borrow_mut().push(index);
                self.entries[index]
                    .as_ref()
                    .map_or(0, |(_, amount)| *amount)
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
                unreachable!("a visit does not ask for capacity")
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<crate::Overanswered>> {
                unreachable!("a visit does not insert")
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<crate::Overanswered>> {
                unreachable!("a visit does not extract")
            }
        }

        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let reads = Rc::new(Reads::default());
        let storage = Slots(Capability {
            handler: Box::new(Probe {
                entries: [None, Some((r[0].clone(), 7)), Some((r[1].clone(), 13))],
                reads: Rc::clone(&reads),
            }),
            owner: None,
        });
        let mut delivered = Vec::new();
        storage
            .visit_entries(&mut host, &mut |kind, amount| {
                delivered.push((kind.clone(), amount));
                if *kind == r[0] {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert_eq!(reads.resources.borrow().as_slice(), &[0, 1]);
        assert_eq!(reads.amounts.borrow().as_slice(), &[1]);
        assert_eq!(delivered, [(r[0].clone(), 7)]);
    }

    #[test]
    fn a_transfer_moves_billions_in_four_calls() {
        let (mut host, r) = host_with(&["minecraft:cobblestone"]);
        let (from_store, from) = bulk(u64::MAX, &[(&r[0], 5_000_000_000)]);
        let (into_store, into) = bulk(u64::MAX, &[]);
        let host: &mut dyn ServerHost = &mut host;
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], u64::MAX, tx));
        assert_eq!(moved, Ok(5_000_000_000));
        assert_eq!(into_store.held.borrow()[&r[0]], 5_000_000_000);
        // amount and space_for to plan, then the extract (with its amount) and the insert
        // (with its space_for).
        assert_eq!(from_store.calls.get() + into_store.calls.get(), 4);
    }

    #[test]
    fn a_transfer_into_slots_takes_what_fits_and_leaves_the_rest() {
        let (mut host, r) = host_with(&["minecraft:stone"]);
        chest(&mut host, [0, 64, 0], &[None, None, None]);
        let into = storage_at(&mut host, [0, 64, 0]);
        let (from_store, from) = bulk(u64::MAX, &[(&r[0], 1000)]);
        let host: &mut dyn ServerHost = &mut host;
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], 1000, tx));
        assert_eq!(moved, Ok(192));
        assert_eq!(from_store.held.borrow()[&r[0]], 808);
        let moved = host.transaction(|host, tx| transfer(host, &from, &into, &r[0], 1000, tx));
        assert_eq!(moved, Ok(0), "a full target is not tried");
    }

    #[test]
    fn transfer_matching_moves_only_what_the_filter_takes() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((&r[0], 30)), Some((&r[1], 20)), Some((&r[1], 7))],
        );
        let from = storage_at(&mut host, [0, 64, 0]);
        let (into_store, into) = bulk(u64::MAX, &[]);
        let host: &mut dyn ServerHost = &mut host;
        let dirt = r[1].clone();
        let moved = host.transaction(|host, tx| {
            transfer_matching(host, &from, &into, |res| *res == dirt, u64::MAX, tx)
        });
        assert_eq!(moved, Ok(27));
        assert_eq!(from.amount(host, &r[0]), Ok(30));
        assert_eq!(into_store.held.borrow().get(&r[0]), None);
    }

    #[test]
    fn transfer_matching_ends_reads_and_visits_once_across_compaction() {
        struct Ordered {
            entries: Rc<RefCell<Vec<(ItemResource, u64)>>>,
            refuses: Option<ItemResource>,
            visits: Rc<Cell<u32>>,
        }

        // These fixtures only make full moves or refuse during planning, so no journal is
        // aborted. Extracting during a visit would conflict with the provider's read borrow.
        impl ItemStorage for Ordered {
            fn amount(
                &self,
                _: &mut dyn ServerHost,
                kind: &ItemResource,
            ) -> Result<u64, StorageError> {
                Ok(self
                    .entries
                    .borrow()
                    .iter()
                    .filter(|(held, _)| held == kind)
                    .map(|(_, amount)| *amount)
                    .sum())
            }
            fn space_for(
                &self,
                _: &mut dyn ServerHost,
                kind: &ItemResource,
            ) -> Result<u64, StorageError> {
                Ok(if self.refuses.as_ref() == Some(kind) {
                    0
                } else {
                    u64::MAX
                })
            }
            fn insert(
                &self,
                host: &mut dyn ServerHost,
                tx: &TransactionContext,
                kind: &ItemResource,
                amount: u64,
            ) -> Result<u64, StorageError> {
                assert!(host.transaction_matches(Some(tx)));
                let amount = amount.min(self.space_for(host, kind)?);
                if amount > 0 {
                    self.entries.borrow_mut().push((kind.clone(), amount));
                }
                Ok(amount)
            }
            fn extract(
                &self,
                host: &mut dyn ServerHost,
                tx: &TransactionContext,
                kind: &ItemResource,
                amount: u64,
            ) -> Result<u64, StorageError> {
                assert!(host.transaction_matches(Some(tx)));
                let mut entries = self.entries.borrow_mut();
                let mut left = amount;
                let mut index = 0;
                while index < entries.len() && left > 0 {
                    if entries[index].0 == *kind {
                        let moved = left.min(entries[index].1);
                        entries[index].1 -= moved;
                        left -= moved;
                        if entries[index].1 == 0 {
                            entries.remove(index);
                            continue;
                        }
                    }
                    index += 1;
                }
                Ok(amount - left)
            }
            fn visit_entries(
                &self,
                _: &mut dyn ServerHost,
                visitor: &mut dyn FnMut(&ItemResource, u64) -> ControlFlow<()>,
            ) -> Result<(), StorageError> {
                self.visits.set(self.visits.get() + 1);
                let entries = self.entries.borrow();
                for (kind, amount) in entries.iter() {
                    if visitor(kind, *amount).is_break() {
                        break;
                    }
                }
                Ok(())
            }
        }

        let (mut host, r) =
            host_with(&["minecraft:stone", "minecraft:dirt", "minecraft:cobblestone"]);
        let source = Rc::new(RefCell::new(vec![
            (r[0].clone(), 3),
            (r[1].clone(), 5),
            (r[2].clone(), 7),
            (r[1].clone(), 4),
        ]));
        let destination = Rc::new(RefCell::new(Vec::new()));
        let visits = Rc::new(Cell::new(0));
        let from = Capability::<dyn ItemStorage> {
            handler: Box::new(Ordered {
                entries: Rc::clone(&source),
                refuses: None,
                visits: Rc::clone(&visits),
            }),
            owner: None,
        };
        let to = Capability::<dyn ItemStorage> {
            handler: Box::new(Ordered {
                entries: Rc::clone(&destination),
                refuses: Some(r[0].clone()),
                visits: Rc::new(Cell::new(0)),
            }),
            owner: None,
        };
        let host: &mut dyn ServerHost = &mut host;
        let filtered = RefCell::new(Vec::new());
        let zero = host.transaction(|host, tx| {
            transfer_matching(
                host,
                &from,
                &to,
                |res| {
                    filtered.borrow_mut().push(res.clone());
                    true
                },
                0,
                tx,
            )
        });
        assert_eq!(zero, Ok(0));
        assert_eq!(visits.get(), 0);
        assert!(filtered.borrow().is_empty());

        let moved = host.transaction(|host, tx| {
            transfer_matching(
                host,
                &from,
                &to,
                |res| {
                    filtered.borrow_mut().push(res.clone());
                    true
                },
                99,
                tx,
            )
        });
        assert_eq!(moved, Ok(16));
        assert_eq!(visits.get(), 1);
        assert_eq!(
            filtered.borrow().as_slice(),
            &[r[0].clone(), r[1].clone(), r[2].clone(), r[1].clone()]
        );
        assert_eq!(source.borrow().as_slice(), &[(r[0].clone(), 3)]);
        assert_eq!(
            destination.borrow().as_slice(),
            &[(r[1].clone(), 9), (r[2].clone(), 7)]
        );
    }

    #[test]
    fn transfers_aborted_and_committed_conserve_every_item() {
        let (mut host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        chest(
            &mut host,
            [0, 64, 0],
            &[Some((&r[0], 64)), Some((&r[1], 40)), Some((&r[0], 9))],
        );
        chest(&mut host, [1, 64, 0], &[None, Some((&r[1], 3)), None]);
        let a = storage_at(&mut host, [0, 64, 0]);
        let b = storage_at(&mut host, [1, 64, 0]);
        let host: &mut dyn ServerHost = &mut host;
        let total = |host: &mut dyn ServerHost| {
            r.iter()
                .map(|kind| a.amount(host, kind).unwrap() + b.amount(host, kind).unwrap())
                .sum::<u64>()
        };
        let before = total(host);
        for round in 0..40u64 {
            let (from, to) = if round.is_multiple_of(2) {
                (&a, &b)
            } else {
                (&b, &a)
            };
            let kind = &r[(round % 3 % 2) as usize];
            let result: Result<u64, TransactionError<()>> = host.transaction(|host, tx| {
                let moved =
                    transfer(host, from, to, kind, round + 1, tx).expect("test transfer succeeds");
                // Every third round aborts after it wrote.
                if round.is_multiple_of(3) {
                    return Err(());
                }
                Ok(moved)
            });
            assert_eq!(result.is_err(), round.is_multiple_of(3));
            assert_eq!(total(host), before, "round {round}");
        }
    }

    #[test]
    fn a_resource_keys_a_map_by_item_damage_and_components() {
        let (host, r) = host_with(&["minecraft:stone", "minecraft:dirt"]);
        let stone = host
            .item_key(&Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        let state = std::hash::RandomState::new();
        let hash_with = |resource: &ItemResource| state.hash_one(resource);
        // Equal stacks of different counts are one kind, and hash alike.
        let many = ItemResource::of(&ItemStack::new(stone, 64).unwrap());
        assert_eq!(many, r[0]);
        assert_eq!(hash_with(&many), hash_with(&r[0]));
        // Damage and a component patch each make another kind.
        let mut worn = ItemStack::new(stone, 9).unwrap();
        worn.set_damage(3);
        let mut named = ItemStack::new(stone, 1).unwrap();
        named
            .set_components(
                [(
                    "minecraft:custom_data".into(),
                    Tag::Compound([("bench".into(), Tag::Int(1))].into()),
                )]
                .into(),
            )
            .unwrap();
        let named = ItemResource::of(&named);
        assert_ne!(named, r[0]);
        let mut counts: HashMap<ItemResource, u64> = HashMap::new();
        for resource in [
            &r[0],
            &r[1],
            &ItemResource::of(&worn),
            &named,
            &many,
            &named,
        ] {
            *counts.entry(resource.clone()).or_default() += 1;
        }
        assert_eq!(counts.len(), 4);
        assert_eq!(counts[&r[0]], 2, "stone of any count is one key");
        assert_eq!(counts[&named], 2, "the patch keys its own entry");
    }
}
