//! Capability parts (arismoko/horizon#41 §7): providers a mod registers for its block entity
//! type or its block to answer a capability, such as [`ITEMS`], beside the game's own
//! (NeoForge's `RegisterCapabilitiesEvent`). A handler holds no borrow of the game: each call
//! is given the host, so a handler may look up and call a neighbour's capability (a pipe), and
//! it keeps its state where the game keeps the block entity's (its slots, its saved compound, its
//! attachments).
use super::item_stack::read_stack_view;
use crate::{
    Answer, BlockEntityRef, BlockEntityTypeKey, BlockProviders, BlockState, Direction,
    ItemResource, LevelHost, LevelRef, Overanswered, ResourceHandler, ServerHost, StorageSlot,
    TransactionContext, TransactionError, backend_mut_of, backend_of,
};
use std::{
    fmt::Debug,
    marker::PhantomData,
    ops::{Deref, Range},
};

// no wire: process-local capability declaration carries a provider function pointer and type identity.
/// A capability: its name (`sunrise:items`, `mymod:chemicals`), the handler trait `T` it answers
/// with, and the game's own provider, if the game has one.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CapabilityKey<T: ?Sized + 'static> {
    /// The name.
    pub name: &'static str,
    pub(crate) default: Option<BlockCapabilityDefault<T>>,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

pub(crate) enum BlockCapabilityDefault<T: ?Sized> {
    Items(fn(BlockSide) -> Box<T>),
    Server(fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>>),
}

/// The subject of a lookup, resolved once from the same resident level view the providers read.
// hidden: internal bridge from the engine's restricted level to the provider table
// exhaustive: only the engine's lookup resolves the site's inputs
// no wire: the subject is scoped to one in-process provider dispatch
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BlockCapabilitySite {
    /// The queried position and side.
    pub(crate) at: BlockSide,
    /// The current state at the position.
    pub(crate) state: BlockState,
    /// The resident block entity, if any.
    pub(crate) entity: Option<BlockEntityRef>,
    /// The entity's registered type, if known.
    pub(crate) kind: Option<BlockEntityTypeKey>,
    /// Whether the game offers its own item handler at this site.
    pub(crate) builtin_items: bool,
}

impl BlockCapabilitySite {
    /// Bundle the inputs resolved by the engine. An entity's registered type is only
    /// meaningful when the lookup found a resident entity.
    pub fn new(
        at: BlockSide,
        state: BlockState,
        resident: Option<(BlockEntityRef, Option<BlockEntityTypeKey>)>,
        builtin_items: bool,
    ) -> Self {
        let (entity, kind) = resident.map_or((None, None), |(entity, kind)| (Some(entity), kind));
        Self {
            at,
            state,
            entity,
            kind,
            builtin_items,
        }
    }
}

/// A provider's scoped answer. Leaving it untouched declines the lookup.
// exhaustive: fields are private; only the dispatcher creates offers
// no wire: a scoped in-process reply that accepts a non-escaping constructor
pub struct BlockCapabilityOffer<'a, T: ?Sized> {
    mode: OfferMode<'a, T>,
}

enum OfferMode<'a, T: ?Sized> {
    Presence(&'a mut bool),
    Acquire(&'a mut Option<Box<T>>),
}

impl<'a, T: ?Sized> BlockCapabilityOffer<'a, T> {
    pub(crate) fn presence(found: &'a mut bool) -> Self {
        Self {
            mode: OfferMode::Presence(found),
        }
    }
    pub(crate) fn construct(handler: &'a mut Option<Box<T>>) -> Self {
        Self {
            mode: OfferMode::Acquire(handler),
        }
    }

    /// Offer a handler. The first offer wins. `make` runs only when acquiring the handler,
    /// never for a presence check. Make the decision to decline before calling `offer`.
    pub fn offer(&mut self, make: impl FnOnce() -> Box<T>) {
        match &mut self.mode {
            OfferMode::Presence(found) => **found = true,
            OfferMode::Acquire(handler) if handler.is_none() => **handler = Some(make()),
            OfferMode::Acquire(_) => {}
        }
    }
}

/// Project the first answer into a presence result without constructing any handler.
pub fn item_capability_present(view: &dyn LevelHost, site: BlockCapabilitySite) -> bool {
    BlockProviders::at(&ITEMS, site).map_or(site.builtin_items, |providers| providers.present(view))
}

/// A position a capability is asked of (its block, and the block entity there if any), by its
/// level and position, and the side it is
/// reached through (none: from no particular side): NeoForge's block capability context.
///
/// This is a local query address. At a serialization or network boundary, put the canonical
/// dimension name, `pos`, and `side` in your own `Wire` type. Get the name with
/// `level.dimension().id(host)`; if it returns `None`, do not encode the address. Do not send
/// the `DimensionKey`: custom dimension slots can differ between processes. On receipt, use
/// `host.level(&name)` and reconstruct the address only if it returns a level.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::BlockSide>();
/// ```
// no wire: local block-capability query context carrying a process-local LevelRef.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct BlockSide {
    /// The level.
    pub level: LevelRef,
    /// The pos.
    pub pos: [i32; 3],
    /// The side.
    pub side: Option<Direction>,
}

// no wire: owned trait-object capability handler; invoked through its provider, not serialized.
/// A capability's handler, with who answered: a mod's provider or the game's own.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct Capability<T: ?Sized> {
    pub(crate) handler: Box<T>,
    /// The mod whose provider answered; none for the game.
    pub(crate) owner: Option<&'static str>,
}

impl<T: ?Sized> Deref for Capability<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.handler
    }
}

impl<T: ?Sized> Capability<T> {
    /// The id of the mod whose provider answered, or "the game", for warnings.
    pub fn owner(&self) -> String {
        self.owner.unwrap_or("the game").to_owned()
    }
    /// `answer`, the handler's amount for a request of `asked` at `at`: itself, or none beyond
    /// `asked`, a mod bug warned once naming the mod. The caller aborts the move.
    pub(crate) fn checked<N: PartialOrd>(&self, answer: N, asked: N, at: &dyn Debug) -> Option<N> {
        if answer > asked {
            crate::block::item_place::warn_broken(self.owner, at);
            return None;
        }
        Some(answer)
    }
}

// Keep a provider's specialised slot-less operations and queries when it is used as a handler.
impl<R> ResourceHandler<R> for Capability<dyn ResourceHandler<R>> {
    fn set_max_stack_size(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        limit: u32,
    ) -> Result<bool, TransactionError<()>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.handler.set_max_stack_size(host, tx, limit)
    }

    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.handler.size(host)
    }

    fn next_transfer_index(
        &self,
        host: &mut dyn ServerHost,
        indices: Range<usize>,
        inserting: Option<&R>,
    ) -> Option<usize> {
        self.handler.next_transfer_index(host, indices, inserting)
    }

    fn uniform_slots_end(&self, host: &mut dyn ServerHost, indices: Range<usize>) -> usize {
        self.handler.uniform_slots_end(host, indices)
    }

    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        self.handler.resource(host, index)
    }

    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.handler.amount(host, index)
    }

    fn amount_int(&self, host: &mut dyn ServerHost, index: usize) -> i32 {
        self.handler.amount_int(host, index)
    }

    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
        self.handler.capacity(host, index, resource)
    }

    fn capacity_int(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> i32 {
        self.handler.capacity_int(host, index, resource)
    }

    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        self.handler.is_valid(host, index, resource)
    }

    fn backing(&self, host: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
        self.handler.backing(host, index)
    }

    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.check_resource_transfer(
            self.handler.insert_at(host, tx, index, resource, amount),
            amount,
            "insert_at",
        )
    }

    fn insert_at_in_range(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        indices: Range<usize>,
        resource: &R,
        amount: u32,
    ) -> Result<(u32, usize), TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        match self
            .handler
            .insert_at_in_range(host, tx, indices, resource, amount)
        {
            Ok((answer, next)) => Ok((
                self.check_resource_transfer(Ok(answer), amount, "insert_at_in_range")?,
                next,
            )),
            Err(error) => Err(self
                .check_resource_transfer(Err(error), amount, "insert_at_in_range")
                .unwrap_err()),
        }
    }

    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.check_resource_transfer(
            self.handler.extract_at(host, tx, index, resource, amount),
            amount,
            "extract_at",
        )
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.check_resource_transfer(
            self.handler.insert(host, tx, resource, amount),
            amount,
            "insert",
        )
    }

    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.check_resource_transfer(
            self.handler.extract(host, tx, resource, amount),
            amount,
            "extract",
        )
    }
}

impl<R> Capability<dyn ResourceHandler<R>> {
    fn check_resource_transfer(
        &self,
        answer: Result<u32, TransactionError<Overanswered>>,
        amount: u32,
        operation: &str,
    ) -> Result<u32, TransactionError<Overanswered>> {
        answer
            .and_then(|answer| {
                crate::content::resource::checked_transfer(answer, amount)
                    .map_err(TransactionError::Aborted)
            })
            .map_err(|mut error| {
                match &mut error {
                    TransactionError::InvalidContext
                    | TransactionError::Unavailable
                    | TransactionError::CommitRefused => {
                        let owner = self.owner.unwrap_or("the game");
                        log::warn!("Resource handler from {owner}: {operation}: {error:?}");
                    }
                    TransactionError::Aborted(over) if over.owner.is_none() => {
                        crate::block::item_place::warn_broken(self.owner, &operation);
                        over.owner = Some(self.owner());
                    }
                    TransactionError::Aborted(_) => {}
                }
                error
            })
    }
}

impl<T: ?Sized + 'static> CapabilityKey<T> {
    /// A mod's own capability, answered only by the parts registered for it.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            default: None,
            handler: PhantomData,
        }
    }
    /// Whether this key's published metadata marks unilateral forwarding to another block safe.
    /// Unmarked, unknown and differently typed capabilities are not proxyable.
    pub fn is_proxyable(&self, host: &dyn crate::Host) -> bool {
        host.capability_providers()
            .block_capability_proxyable(self.name, Some(self.binding().0))
    }
}

/// Item slots, as NeoForge 26.3's `Capabilities.Item.BLOCK`: container slots as one side
/// reaches them. The game answers for block entities with slots by their `WorldlyContainer`
/// sides, including double chests as one container and a mod block's `Registrar::block_container`,
/// decorated pots and composters without block entities. A composter has one input slot from above
/// and one output slot from below; unsided and horizontal lookups give no game handler. Its input
/// consumes one compostable item per insertion below level seven, its output holds one bone
/// meal only at level eight, and feedback waits for the root commit. Its position-local random
/// stream is also journaled, so aborted retries keep their chance outcome without rewinding
/// the level's unrelated random draws.
///
/// # Hopper and dropper automation rules
///
/// Vanilla block containers, composters and decorated pots keep their direct block automation
/// paths even if a mod also exposes a block `ITEMS` handler; only blocks without a vanilla
/// automation container use the block `ITEMS` storage attempt path. Container entities (such as
/// chest or hopper minecarts and chest boats) use their own entity automation paths separately
/// from block `ITEMS`.
///
/// - **Hopper pull:** skips only a fully stacked hopper. It visits nonempty source indices on the
///   bottom-face `ITEMS` handler whose move is not cancelled, extracts one in a fresh transaction,
///   then tries eligible hopper slots in order (empty or matching item and components with room;
///   an empty lower index wins over a matching later stack), committing on the first success or
///   aborting an ordinary refusal and continuing. For a block hopper, falling back to item
///   entities happens only when no source (block container, `ITEMS` handler, or container entity)
///   is found above it—a failed pull from a found source does not fall back to item entities. A
///   hopper minecart still tries item entities after a failed pull and has no block-hopper
///   transfer cooldown.
/// - **Hopper push:** skips only storage whose slots are all at capacity. One root transaction
///   covers eligible nonempty hopper slots in order (skipping cancelled moves): snapshots and
///   removes one, calls indexless insertion on the face opposite the hopper's facing, restores a
///   refused source slot directly and continues, committing on the first success or aborting if
///   every eligible slot is refused.
/// - **Dropper:** in one transaction, reserves (removes) the offered item from the source slot
///   before calling `insert`, so writable insertion or root-commit callbacks cannot move the same
///   source item a second time. An ordinary refusal (`Ok(0)`) restores the reserved item to the
///   source slot and still commits the transaction; only an exceptional refusal (such as a
///   writable callback replacing the source slot with another item, or an invalid overanswer)
///   rolls back the dropper attempt's transaction.
/// - **Ordinary refusal vs. invalid overanswer:** returning `Ok(0)` is an ordinary refusal handled
///   as above; returning more than offered or requested is an invalid overanswer
///   (`TransactionError::Aborted(Overanswered)`), which rolls back that attempt's transaction
///   scope (or the `resource_util` helper's transaction scope when called through a helper, even
///   if an enclosing transaction later commits).
pub const ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> = CapabilityKey {
    name: "sunrise:items",
    default: Some(BlockCapabilityDefault::Items(|at| Box::new(GameItems(at)))),
    handler: PhantomData,
};

/// The game's own [`ITEMS`]: the container slots or composter port `.side` reaches, by the
/// rules hoppers follow.
struct GameItems(BlockSide);
impl ResourceHandler<ItemResource> for GameItems {
    fn set_max_stack_size(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        limit: u32,
    ) -> Result<bool, TransactionError<()>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(backend_mut_of(host)
            .is_some_and(|backend| backend.container_set_stack_limit(self.0, limit, tx)))
    }

    fn size(&self, host: &mut dyn ServerHost) -> usize {
        backend_of(&*host)
            .and_then(|backend| backend.container_size(self.0))
            .unwrap_or(0)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        let backend = backend_of(&*host)?;
        read_stack_view(
            |view| view.project().map(ItemResource::from_owned_stack),
            |lend| {
                backend.lend_container_stack(self.0, index, lend);
                Some(())
            },
        )?
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        backend_of(&*host)
            .and_then(|backend| {
                read_stack_view(
                    |view| view.count(),
                    |lend| {
                        backend.lend_container_stack(self.0, index, lend);
                        Some(())
                    },
                )
            })
            .unwrap_or(0)
    }
    fn capacity(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        resource: Option<&ItemResource>,
    ) -> u32 {
        backend_of(&*host).map_or(0, |backend| {
            backend.container_capacity(self.0, index, resource.map(ItemResource::as_stack))
        })
    }
    fn backing(&self, host: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
        backend_of(&*host)?.container_slot_backing(self.0, index)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        if amount == 0 {
            return Ok(0);
        }
        let Some(backend) = backend_mut_of(host) else {
            return Ok(0);
        };
        let Ok(rest) = backend.container_insert(self.0, index, resource, amount, tx) else {
            return Ok(0);
        };
        Ok(amount - rest.map_or(0, |rest| rest.count().min(amount)))
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let matches = backend_of(&*host).and_then(|backend| {
            read_stack_view(
                |view| resource.matches_view(&view),
                |lend| {
                    backend.lend_container_stack(self.0, index, lend);
                    Some(())
                },
            )
        });
        if matches != Some(true) {
            return Ok(0);
        }
        let Some(backend) = backend_mut_of(host) else {
            return Ok(0);
        };
        Ok(backend
            .container_extract(self.0, index, amount, tx)
            .map_or(0, |out| out.count().min(amount)))
    }
}

impl LevelRef {
    /// `key`'s handler for the block at `pos` in this level, reached through `side`
    /// (`Level.getCapability`): the first provider to answer, of those mods registered
    /// `before_game` for the block or for the type of the block entity there, the game's own,
    /// and the mods' others, each group in registration order across both kinds. Against the
    /// game's own and against another mod's at the same priority, a mod should not count on its
    /// place: one that must be asked first registers `before_game`. None
    /// for a stale handle, and for a level not in place ([`LevelRef::in_place`]), whose block
    /// entities the game doesn't write: no provider is asked for one. Disabled mods' providers
    /// are skipped; re-enabling a mod makes its registrations available to the next lookup.
    pub fn capability<T: ?Sized + 'static>(
        self,
        host: &mut dyn ServerHost,
        key: &CapabilityKey<T>,
        pos: [i32; 3],
        side: Option<Direction>,
    ) -> Option<Capability<T>> {
        let at = BlockSide {
            level: self,
            pos,
            side,
        };
        let mut answer = None;
        backend_of(host)?
            .level_with_capability_site(at, &mut |view, site| {
                let game = match key.default {
                    Some(BlockCapabilityDefault::Items(_)) => site.builtin_items,
                    Some(BlockCapabilityDefault::Server(_)) => true,
                    None => false,
                };
                answer = Some(BlockProviders::at(key, site).map_or_else(
                    || if game { Answer::Game } else { Answer::Nobody },
                    |providers| providers.acquire(view, game, false),
                ));
            })
            .ok()?;
        match answer? {
            Answer::Game => match key.default {
                Some(BlockCapabilityDefault::Items(make)) => {
                    crate::answered(Answer::Game, Some(make(at)))
                }
                Some(BlockCapabilityDefault::Server(default)) => {
                    // The restricted view is no longer borrowed. Some defaults need a server
                    // host (STORAGE, for example, may acquire ITEMS). Resume after the game's
                    // slot in the order only when that default declines.
                    if let Some(handler) = default(host, at) {
                        return crate::answered(Answer::Game, Some(handler));
                    }
                    let mut fallback = None;
                    backend_of(host)?
                        .level_with_capability_site(at, &mut |view, site| {
                            if let Some(providers) = BlockProviders::at(key, site) {
                                fallback =
                                    crate::answered(providers.acquire(view, false, true), None);
                            }
                        })
                        .ok()?;
                    fallback
                }
                None => None,
            },
            selected => crate::answered(selected, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Backend, BackendRef, ComponentId, DimensionKey, Host, ItemKey, ItemStack, ItemStackView,
        Tag, WorldEpoch, prelude::ItemsBackend,
    };
    use std::cell::Cell;

    struct ResidentSlot {
        at: BlockSide,
        stack: ItemStack,
        projections: Cell<usize>,
    }

    impl ItemsBackend for ResidentSlot {
        fn container_size(&self, at: BlockSide) -> Option<usize> {
            (at == self.at).then_some(1)
        }

        fn lend_container_stack(
            &self,
            at: BlockSide,
            slot: usize,
            read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
        ) {
            if at != self.at || slot != 0 {
                return;
            }
            let project = |_| {
                self.projections.set(self.projections.get() + 1);
                Some(self.stack.clone())
            };
            read(ItemStackView::resident(
                self.stack.read_metadata(),
                &project,
            ));
        }
    }

    impl Backend for ResidentSlot {}
    impl Host for ResidentSlot {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    impl ServerHost for ResidentSlot {}
    crate::impl_backend_areas!(ResidentSlot, except(items));
    crate::impl_host_areas!(ResidentSlot);
    crate::impl_server_host_areas!(ResidentSlot);

    #[test]
    fn resident_container_amount_does_not_project_but_resource_projects_once() {
        let level = LevelRef::new(WorldEpoch::new(1), DimensionKey::OVERWORLD);
        let pos = [1, 64, 0];
        let at = BlockSide {
            level,
            pos,
            side: Some(Direction::Up),
        };
        let item = ItemKey(17);
        let component = ComponentId::new("minecraft:custom_data").unwrap();
        let custom_data = Tag::Compound([("marker".into(), Tag::Int(42))].into_iter().collect());
        let mut stack = ItemStack::new(item, 7).unwrap();
        stack
            .set_components(
                [(component.as_str().into(), custom_data.clone())]
                    .into_iter()
                    .collect(),
            )
            .unwrap();
        let mut host = ResidentSlot {
            at,
            stack,
            projections: Cell::new(0),
        };

        let items = GameItems(at);
        assert_eq!(items.amount(&mut host, 0), 7);
        assert_eq!(host.projections.get(), 0);

        let resource = items.resource(&mut host, 0).unwrap();
        assert_eq!(resource.item(), item);
        assert_eq!(resource.component(&component), Some(custom_data));
        assert_eq!(resource.as_stack().count(), 1);
        assert_eq!(host.projections.get(), 1);
    }
}
