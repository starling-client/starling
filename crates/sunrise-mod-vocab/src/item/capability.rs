//! Capability parts (arismoko/horizon#41 §7): providers a mod registers for its block entity
//! type or its block to answer a capability, such as [`ITEMS`], beside the game's own
//! (NeoForge's `RegisterCapabilitiesEvent`). A handler holds no borrow of the game: each call
//! is given the host, so a handler may look up and call a neighbour's capability (a pipe), and
//! it keeps its state where the game keeps the block entity's (its slots, its saved compound, its
//! attachments).
use crate::{
    Answer, BlockEntityRef, BlockEntityTypeKey, BlockProviders, BlockState, Direction,
    ItemResource, LevelHost, LevelRef, Overanswered, ResourceHandler, ServerHost, Transaction,
    backend_mut_of, backend_of,
};
use std::{fmt::Debug, marker::PhantomData, ops::Deref};

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
    fn set_max_stack_size(&self, host: &mut dyn ServerHost, tx: &Transaction, limit: u32) -> bool {
        self.handler.set_max_stack_size(host, tx, limit)
    }

    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.handler.size(host)
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

    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        let answer = self.handler.insert_at(host, tx, index, resource, amount);
        // Keep the raw answer so the caller knows to abort; this boundary knows whom to warn about.
        let _ = self.checked(answer, amount, &index);
        answer
    }

    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        let answer = self.handler.extract_at(host, tx, index, resource, amount);
        let _ = self.checked(answer, amount, &index);
        answer
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> Result<u32, Overanswered> {
        self.check_resource_transfer(
            self.handler.insert(host, tx, resource, amount),
            amount,
            "insert",
        )
    }

    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> Result<u32, Overanswered> {
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
        answer: Result<u32, Overanswered>,
        amount: u32,
        operation: &str,
    ) -> Result<u32, Overanswered> {
        answer
            .and_then(|answer| crate::content::resource::checked_transfer(answer, amount))
            .map_err(|mut error| {
                if error.owner.is_none() {
                    crate::block::item_place::warn_broken(self.owner, &operation);
                    error.owner = Some(self.owner());
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
    /// Unmarked and unknown capabilities are not proxyable.
    pub fn is_proxyable(&self, host: &dyn crate::Host) -> bool {
        crate::block_capability_proxyable(host, self.name)
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
pub const ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> = CapabilityKey {
    name: "sunrise:items",
    default: Some(BlockCapabilityDefault::Items(|at| Box::new(GameItems(at)))),
    handler: PhantomData,
};

/// The game's own [`ITEMS`]: the container slots or composter port `.side` reaches, by the
/// rules hoppers follow.
struct GameItems(BlockSide);
impl ResourceHandler<ItemResource> for GameItems {
    fn set_max_stack_size(&self, host: &mut dyn ServerHost, tx: &Transaction, limit: u32) -> bool {
        backend_mut_of(host)
            .is_some_and(|backend| backend.container_set_stack_limit(self.0, limit, tx))
    }

    fn size(&self, host: &mut dyn ServerHost) -> usize {
        backend_of(&*host)
            .and_then(|backend| backend.container_size(self.0))
            .unwrap_or(0)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        let stack = backend_of(&*host)?.container_stack(self.0, index)?;
        Some(ItemResource::of(&stack))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        backend_of(&*host)
            .and_then(|backend| backend.container_stack(self.0, index))
            .map_or(0, |stack| stack.count())
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
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        if amount == 0 {
            return 0;
        }
        let Some(backend) = backend_mut_of(host) else {
            return 0;
        };
        let Ok(rest) = backend.container_insert(self.0, index, resource, amount, tx) else {
            return 0;
        };
        amount - rest.map_or(0, |rest| rest.count().min(amount))
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let held = backend_of(&*host).and_then(|backend| backend.container_stack(self.0, index));
        if !held.is_some_and(|held| resource.matches(&held)) {
            return 0;
        }
        let Some(backend) = backend_mut_of(host) else {
            return 0;
        };
        backend
            .container_extract(self.0, index, amount, tx)
            .map_or(0, |out| out.count().min(amount))
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
