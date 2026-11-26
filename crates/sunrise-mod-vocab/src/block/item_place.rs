//! Items as holders (arismoko/horizon#41 §7): an item's own capability, such as
//! [`ITEM_ITEMS`] for a backpack's slots, reached where its stack lives. NeoForge 26.3 reaches
//! an item through an `ItemAccess`; here the place is an [`ItemPlace`], read and written as one
//! ([`ItemPlace::insert`], [`extract`](ItemPlace::extract) and
//! [`exchange`](ItemPlace::exchange)), so the inventory holding the stack is the one changed.
//! Handlers hold no borrow of the game, as block entity capabilities don't.
//!
//! A write runs in a transaction of its own, nested in the caller's, and changes the place where
//! it is, journaled: an abort puts it back, at every level of an item inside an item. A handler
//! answering more than it was asked is a mod bug, named once, and the write aborts.
use crate::HostTransaction;
use crate::item::item_stack::{component_edits::edit_item_components, read_stack_view};
use crate::{
    BundleItems, Capability, Direction, ENTITY_ITEMS, EquipmentSlot, Hand, ITEMS,
    ItemComponentEditor, ItemKey, ItemResource, ItemStack, ItemStackView, LevelRef, MenuRef,
    MenuSlotBacking, Overanswered, ResourceHandler, ServerHost, StackWriteError, StorageSlot,
    TransactionContext, TransactionError, backend_mut_of, backend_of,
};
use std::{
    fmt::Debug,
    marker::PhantomData,
    num::NonZeroU32,
    sync::atomic::{AtomicU64, Ordering},
};

// no wire: process-local token for a transaction-scoped lent scratch slot.
// exhaustive: its fields are private, so mods cannot build or destructure it.
/// An opaque handle to a scratch item place lent for one
/// [`HostTransaction::scratch_stack`] or [`HostTransaction::scratch_slots`] scope.
///
/// Each loan mints a process-unique id, and the token cannot cross Wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScratchSlot {
    id: u64,
}

impl ScratchSlot {
    /// Mints a process-unique scratch slot token.
    // hidden: a game-side helper, not part of the mod surface.
    #[doc(hidden)]
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("scratch slot sequence exhausted");
        Self { id }
    }

    /// The scratch slot's process-unique id.
    // hidden: a game-side helper, not part of the mod surface.
    #[doc(hidden)]
    pub fn id(self) -> u64 {
        self.id
    }
}

/// Where a stack lives.
///
/// This is a local item address: encode your own `Wire` schema or an [`ItemStack`] at
/// serialization or network boundaries.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::ItemPlace>();
/// ```
// no wire: process-local item address carrying handles, scratch slot IDs and static provider names.
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq)]
pub enum ItemPlace {
    /// Slot `slot` of a player's inventory, by `inventory_tick`'s numbers: 0..36 (0..9
    /// the hotbar), the armour 100..=103 (feet to head), the off hand -106
    /// (`ItemAccess.forPlayerSlot`).
    PlayerSlot {
        /// The player's UUID.
        player: u128,
        /// The slot number, as listed above.
        slot: i32,
    },
    /// What a player holds in `hand` (`forPlayerInteraction`).
    PlayerHand {
        /// The player's UUID.
        player: u128,
        /// Which hand.
        hand: Hand,
    },
    /// The stack on the cursor of a player's open menu, `menu` (`forPlayerCursor`): nothing
    /// once the menu is closed, replaced or its player gone.
    PlayerCursor {
        /// The player's open menu.
        menu: MenuRef,
    },
    /// Slot `slot` of the block entity at `pos` as `side` reaches it, through its [`ITEMS`]
    /// handler (`forHandlerIndex`).
    BlockEntity {
        /// The level the block entity is in.
        level: LevelRef,
        /// The block entity's position.
        pos: [i32; 3],
        /// The face the handler is reached from; `None` for no particular side.
        side: Option<Direction>,
        /// The slot's index in the handler.
        slot: usize,
    },
    /// The stack of item entity `id` in the level in place.
    ItemEntity {
        /// The item entity's network id.
        id: u32,
    },
    /// Slot `slot` of the inventory of entity `entity` in the level in place: a chest or hopper
    /// minecart's, a chest boat's, or a chested donkey's, mule's or llama's
    /// (`Capabilities.Item.ENTITY` through `forHandlerIndex`). Reading or writing a chest
    /// vehicle's slot unpacks its pending loot table without a player
    /// (`ContainerEntity.getChestVehicleItem` / `setChestVehicleItem`). The fill uses empty
    /// slots without replacing occupied ones, and reads and writes use the live vehicle
    /// menu's slots while it is open.
    EntitySlot {
        /// The entity's network id.
        entity: u32,
        /// The slot's index in the entity's inventory.
        slot: usize,
    },
    /// Slot `slot` of living entity `entity`'s equipment (NeoForge's
    /// `LivingEntityEquipmentWrapper` through `Capabilities.Item.ENTITY`), reached through
    /// [`ENTITY_ITEMS`](crate::ENTITY_ITEMS). A slot replacement uses the equipment write
    /// `LivingRef::set_equipment` makes, with the same game events, sounds and attribute
    /// updates. [`edit`](Self::edit) changes only the components without those equip effects.
    /// A transaction that aborts restores the previous slot.
    Equipment {
        /// The entity's network id.
        entity: u32,
        /// The equipment slot.
        slot: EquipmentSlot,
    },
    /// Slot `slot` of the item at `parent`, through its [`ITEM_ITEMS`] handler: a backpack in a
    /// backpack.
    Nested {
        /// The place of the stack that holds the container.
        parent: Box<ItemPlace>,
        /// The slot's index inside that stack's container.
        slot: usize,
    },
    /// Slot `slot` of a player's open menu, `menu`, a fork addition (NeoForge 26.3 has no
    /// `ItemAccess` for a menu slot): the storage the slot shows ([`MenuRef::slot_backing`]),
    /// its own slot only, under the slot's own rules first ([`MenuRef::slot_holds`],
    /// [`MenuRef::slot_gives`]). Nothing where the menu is closed, replaced or its player gone;
    /// nothing goes into or out of the crafting result.
    MenuSlot {
        /// The open menu.
        menu: MenuRef,
        /// The slot's index in that menu.
        slot: usize,
    },
    /// A place a mod keeps, such as an accessory slot: read and written by the [`PlaceProvider`]
    /// it registered as `provider`, which alone knows what `key` means.
    Mod {
        /// The provider's registered name.
        provider: &'static str,
        /// The provider's own key for the place.
        key: u64,
    },
    /// A stack outside the world (`ItemAccess.forStack`), or a two-slot copy whose item may
    /// change (`DispenseFluidContainer`): a crafting result, a preview, a stack about to be
    /// given, or a dispensed fluid container. [`HostTransaction::scratch_stack`] lends a
    /// single-slot place (whose writes keep the same item and refuse what doesn't fit or is
    /// another item, as `StackItemAccess`), and [`HostTransaction::scratch_slots`] lends a
    /// two-slot place whose first slot holds the stack and whose second slot catches what
    /// doesn't fit (`forHandlerIndex`, as a changed item split off a stack does).
    ///
    /// The place lives only for the scope of that callback; its final stack or slots are
    /// returned on commit, and an abort restores or drops it. It cannot cross Wire.
    Scratch(ScratchSlot),
    /// The place inside, where an insert goes into that slot only
    /// (`ItemAccess.forHandlerIndexStrict`): around a [`BlockEntity`](Self::BlockEntity),
    /// [`EntitySlot`](Self::EntitySlot) or [`Nested`](Self::Nested) slot, what doesn't fit is
    /// refused instead of spread to the other slots. Around any other place it is that place.
    Strict(Box<ItemPlace>),
    /// One item at a time of the place inside (`ItemAccess.oneByOne`, `OneByOneItemAccess`): the
    /// view and its projection show one item without changing the stored count, and both inserts
    /// and extracts move at most one item.
    OneByOne(Box<ItemPlace>),
    /// The place inside, as a player with infinite materials reaches it
    /// (`ItemAccess.forInfiniteMaterials`): while `player` has them (creative's `instabuild`),
    /// an extract gives without taking and an insert is taken whole, one of it going to the
    /// player only if they have none: into their inventory, or dropped at the root commit when
    /// it is full. Inserting nothing gives nothing. Otherwise it is the place inside.
    ///
    /// Wrappers may nest to any depth: reading and writing undo them in a loop.
    InfiniteMaterials {
        /// The player's UUID.
        player: u128,
        /// The place the player reaches.
        inner: Box<ItemPlace>,
    },
}

impl Clone for ItemPlace {
    fn clone(&self) -> Self {
        enum Link {
            Strict,
            One,
            Infinite(u128),
            Nested(usize),
        }
        let mut links = Vec::new();
        let mut current = self;
        let mut cloned = loop {
            match current {
                Self::Strict(inner) => {
                    links.push(Link::Strict);
                    current = inner;
                }
                Self::OneByOne(inner) => {
                    links.push(Link::One);
                    current = inner;
                }
                Self::InfiniteMaterials { player, inner } => {
                    links.push(Link::Infinite(*player));
                    current = inner;
                }
                Self::Nested { parent, slot } => {
                    links.push(Link::Nested(*slot));
                    current = parent;
                }
                Self::PlayerSlot { player, slot } => {
                    break Self::PlayerSlot {
                        player: *player,
                        slot: *slot,
                    };
                }
                Self::PlayerHand { player, hand } => {
                    break Self::PlayerHand {
                        player: *player,
                        hand: *hand,
                    };
                }
                Self::PlayerCursor { menu } => break Self::PlayerCursor { menu: *menu },
                Self::BlockEntity {
                    level,
                    pos,
                    side,
                    slot,
                } => {
                    break Self::BlockEntity {
                        level: *level,
                        pos: *pos,
                        side: *side,
                        slot: *slot,
                    };
                }
                Self::ItemEntity { id } => break Self::ItemEntity { id: *id },
                Self::EntitySlot { entity, slot } => {
                    break Self::EntitySlot {
                        entity: *entity,
                        slot: *slot,
                    };
                }
                Self::Equipment { entity, slot } => {
                    break Self::Equipment {
                        entity: *entity,
                        slot: *slot,
                    };
                }
                Self::MenuSlot { menu, slot } => {
                    break Self::MenuSlot {
                        menu: *menu,
                        slot: *slot,
                    };
                }
                Self::Mod { provider, key } => {
                    break Self::Mod {
                        provider,
                        key: *key,
                    };
                }
                Self::Scratch(slot) => break Self::Scratch(*slot),
            }
        };
        for link in links.into_iter().rev() {
            cloned = match link {
                Link::Strict => Self::Strict(Box::new(cloned)),
                Link::One => Self::OneByOne(Box::new(cloned)),
                Link::Infinite(player) => Self::InfiniteMaterials {
                    player,
                    inner: Box::new(cloned),
                },
                Link::Nested(slot) => Self::Nested {
                    parent: Box::new(cloned),
                    slot,
                },
            };
        }
        cloned
    }
}

impl Drop for ItemPlace {
    fn drop(&mut self) {
        let mut current = match self {
            Self::Strict(inner)
            | Self::OneByOne(inner)
            | Self::InfiniteMaterials { inner, .. }
            | Self::Nested { parent: inner, .. } => {
                std::mem::replace(inner, Box::new(Self::ItemEntity { id: 0 }))
            }
            _ => return,
        };
        while let Self::Strict(inner)
        | Self::OneByOne(inner)
        | Self::InfiniteMaterials { inner, .. }
        | Self::Nested { parent: inner, .. } = &mut *current
        {
            let next = std::mem::replace(inner, Box::new(Self::ItemEntity { id: 0 }));
            current = next;
        }
    }
}

/// Why [`ItemPlace::edit`] could not update a stack's components.
///
/// This is a local refusal returned by [`ItemPlace::edit`], not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::ItemPlaceEditError>();
/// ```
// no wire: local refusal returned by ItemPlace::edit.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemPlaceEditError {
    /// The supplied context does not identify this host's innermost open transaction.
    InvalidContext,
    /// The place is empty or cannot be reached.
    NotHeld,
    /// The place has no whole-stack edit (a virtual transfer wrapper such as `OneByOne` or
    /// `InfiniteMaterials`).
    UnsupportedPlace,
    /// The backend or storage refused the write. For native player slots, hands, cursors
    /// and equipment, a missing backend refuses before the editor runs.
    Unavailable,
    /// The component edits did not pass the game's write validation; none were applied.
    Stack(StackWriteError),
}

impl From<StackWriteError> for ItemPlaceEditError {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl crate::NoBackend for ItemPlaceEditError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A mod's own places ([`ItemPlace::Mod`]), registered with
/// [`Registrar::place_provider`](crate::ItemsRegistrar::place_provider): NeoForge's own `ItemAccess`s.
/// Like a handler, it holds no borrow: its stacks live where the mod keeps them (a player's
/// attachments). While its mod is disabled, lookups refuse both reads and writes through its
/// places; re-enabling the mod makes the same registrations available again.
/// Write methods receive the innermost transaction context; pass it to delegated writes.
pub trait PlaceProvider: Send + Sync + 'static {
    /// The stack at `key`, if any.
    fn stack(&self, host: &mut dyn ServerHost, key: u64) -> Option<ItemStack>;
    /// Puts up to `amount` of `resource` at `key`, in transaction `tx`: how much went. What
    /// doesn't stack with what is there may go where the mod puts it, such as the player's
    /// inventory (`ItemAccess.insert`); none where `key` is no longer a place. A mismatched `tx`
    /// returns `Err(TransactionError::InvalidContext)`.
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        key: u64,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>>;
    /// Takes up to `amount` of `resource` from `key`, in transaction `tx`: how much came. A
    /// mismatched `tx` returns `Err(TransactionError::InvalidContext)`.
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        key: u64,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>>;
}

// no wire: process-local capability declaration carries a provider function pointer and type identity.
/// An item capability: its name, the handler trait `T` it answers with, and the game's own
/// provider, if the game has one.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemCapabilityKey<T: ?Sized + 'static> {
    /// The capability's name.
    pub name: &'static str,
    pub(crate) default: Option<ItemCapabilityDefault<T>>,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

/// What the game's answering provider needs from the place's read.
pub(crate) enum ItemCapabilityDefault<T: ?Sized> {
    Item(fn(&mut dyn ServerHost, &ItemPlace, ItemKey) -> Option<Box<T>>),
    Stack(fn(&mut dyn ServerHost, &ItemPlace, &ItemStack) -> Option<Box<T>>),
}

impl<T: ?Sized + 'static> ItemCapabilityKey<T> {
    /// A mod's own item capability, answered only by the parts registered for it.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            default: None,
            handler: PhantomData,
        }
    }
}

/// Items held by an item, as NeoForge 26.3's `Capabilities.Item.ITEM`: a handler for the item at
/// one place, whose changes rewrite the item there with an [`exchange`](ItemPlace::exchange) of
/// one, so from a stack of more than one the changed item splits off. The game answers it for an item with `item_container`
/// slots, kept in its `minecraft:container`, or `minecraft:bundle_contents`.
pub const ITEM_ITEMS: ItemCapabilityKey<dyn ResourceHandler<ItemResource>> = ItemCapabilityKey {
    name: "sunrise:item_items",
    default: Some(ItemCapabilityDefault::Stack(|host, place, stack| {
        let backend = backend_of(host)?;
        if backend.item_container_slots(stack).is_some() {
            Some(Box::new(GameItemItems(place.clone())))
        } else {
            backend
                .bundle_slots(stack, &crate::BundleConfig::vanilla())
                .ok()??;
            Some(Box::new(BundleItems::new(place.clone())))
        }
    })),
    handler: PhantomData,
};

/// The game's own [`ITEM_ITEMS`] for the item at `.0`, over the `item_container` helpers.
struct GameItemItems(ItemPlace);
impl GameItemItems {
    fn slot(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemStack> {
        let held = self.0.with_stack(host, |stack| stack.project()).flatten()?;
        backend_of(host)?
            .item_container_slots(&held)?
            .into_iter()
            .nth(index)
            .flatten()
    }
    /// Rewrites one item at `.0` as `changed`: `moved` when that goes through, else 0.
    fn rewrite(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        changed: &ItemStack,
        moved: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        Ok(
            match self.0.exchange(host, tx, &ItemResource::of(changed), 1)? {
                1 => moved,
                _ => 0,
            },
        )
    }
}
impl ResourceHandler<ItemResource> for GameItemItems {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| backend_of(host)?.item_container_slots(&held))
            .map_or(0, |slots| slots.len())
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        Some(ItemResource::of(&self.slot(host, index)?))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.slot(host, index).map_or(0, |stack| stack.count())
    }
    fn capacity(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        resource: Option<&ItemResource>,
    ) -> u32 {
        let Some(resource) = resource else {
            return if index < self.size(host) { 99 } else { 0 };
        };
        self.0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                Some(backend_of(host)?.item_container_capacity(&held, index, resource.as_stack()))
            })
            .unwrap_or(0)
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
        let Some(offer) = resource.to_stack(amount) else {
            return Ok(0);
        };
        let Some((went, changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| backend_of(host)?.item_container_insert_slot(&held, index, &offer))
        else {
            return Ok(0);
        };
        match went {
            0 => Ok(0),
            went => self.rewrite(host, tx, &changed, went.min(amount)),
        }
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
        let Some((Some(out), changed)) = self
            .0
            .with_stack(host, |stack| stack.project())
            .flatten()
            .and_then(|held| {
                let backend = backend_of(host)?;
                let slot = backend
                    .item_container_slots(&held)?
                    .into_iter()
                    .nth(index)??;
                if !resource.matches(&slot) {
                    return None;
                }
                backend.item_container_extract(&held, index, amount)
            })
        else {
            return Ok(0);
        };
        self.rewrite(host, tx, &changed, out.count().min(amount))
    }
}

/// `owner`'s handler at `at` answered more than it was asked: a mod bug, warned once per mod.
pub(crate) fn warn_broken(owner: Option<&'static str>, at: &dyn Debug) {
    let owner = owner.unwrap_or("the game");
    if crate::dispatch_scope::backend()
        .is_none_or(|backend| (backend.diagnostics.first_broken_warning)(owner))
    {
        log::warn!(
            "Sunrise: {owner}'s handler at {at:?} answered more than it was asked; the move it \
             was part of is undone. A handler moves at most what it is asked."
        );
    }
}

impl ItemPlace {
    /// The block container slot this place is, if it is one: [`StorageSlot`], the same value
    /// whichever side, half of a double chest or block of a multiblock reached it. A
    /// [`BlockEntity`](Self::BlockEntity) place asks its [`ITEMS`] handler
    /// ([`ResourceHandler::backing`]); a [`MenuSlot`](Self::MenuSlot) place is the slot its menu
    /// shows when that is a block entity's ([`MenuSlotBacking::BlockEntity`]). [`Strict`](Self::Strict)
    /// and [`OneByOne`](Self::OneByOne) are the place inside. Every other place, and a handler
    /// that gives no backing, is `None`.
    pub fn backing(&self, host: &mut dyn ServerHost) -> Option<StorageSlot> {
        let mut place = self;
        while let Self::Strict(inner) | Self::OneByOne(inner) = place {
            place = inner;
        }
        match place {
            Self::BlockEntity {
                level,
                pos,
                side,
                slot,
            } => level
                .capability(host, &ITEMS, *pos, *side)?
                .backing(host, *slot),
            Self::MenuSlot { menu, slot } => match menu.slot_backing(host, *slot) {
                Some(MenuSlotBacking::BlockEntity { level, pos, slot }) => {
                    Some(StorageSlot { level, pos, slot })
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Lend the stack here to `read` and return its result, or return `None` without calling it
    /// if the place is empty or unavailable. Use [`ItemStackView::project`] to inspect components
    /// or retain an owned stack. Native reads never load chunks or initialize a player; reading a
    /// chest vehicle's [`EntitySlot`](Self::EntitySlot) unpacks its pending loot table without a
    /// player and fills only empty slots (`ContainerEntity.getChestVehicleItem`, `LootTable.fill`).
    /// An item entity read sees live state, including earlier writes in the same transaction.
    /// Handler-backed and decoded places lend a view of their provider's or decoder's value.
    /// An occupied place can still refuse projection. To check occupancy without projection,
    /// use [`Self::amount`] and compare it with zero.
    pub fn with_stack<R>(
        &self,
        host: &mut dyn ServerHost,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        // A mod may nest these wrappers as deeply as it likes without growing the call stack.
        let mut place = self;
        let mut one = false;
        while let Self::Strict(inner)
        | Self::OneByOne(inner)
        | Self::InfiniteMaterials { inner, .. } = place
        {
            one |= matches!(place, Self::OneByOne(_));
            place = inner;
        }
        read_stack_view(
            |stack| {
                read(if one {
                    stack.with_count(NonZeroU32::MIN)
                } else {
                    stack
                })
            },
            |lend| {
                let (handler, slot) = match place {
                    Self::BlockEntity {
                        level,
                        pos,
                        side,
                        slot,
                    } => (level.capability(host, &ITEMS, *pos, *side)?, *slot),
                    Self::Nested { parent, slot } => (parent.capability(host, &ITEM_ITEMS)?, *slot),
                    Self::Mod { provider, key } => {
                        let (provider, _) = host.capability_providers().place_provider(provider)?;
                        let stack = provider.stack(host, *key)?;
                        lend(ItemStackView::from_stack(&stack));
                        return Some(());
                    }
                    Self::MenuSlot { menu, slot } => match menu.slot_backing(host, *slot) {
                        // Only the handler owns this stack. Its refusal must not read the mirror.
                        Some(MenuSlotBacking::Place(place)) => place.handler(host, menu.player)?,
                        _ => {
                            let stack = menu.slot(host, *slot)??;
                            lend(ItemStackView::from_stack(&stack));
                            return Some(());
                        }
                    },
                    _ => return backend_mut_of(host)?.game_stack(place, lend),
                };
                let resource = handler.resource(host, slot)?;
                let count = NonZeroU32::new(handler.amount(host, slot))?;
                lend(ItemStackView::from_stack(resource.as_stack()).with_count(count));
                Some(())
            },
        )
    }

    /// What is here, with its live components but without its count (`ItemAccess.getResource`).
    /// This includes a map's pending lock or scale even though it is not saved.
    pub fn resource(&self, host: &mut dyn ServerHost) -> Option<ItemResource> {
        self.with_stack(host, |stack| {
            stack.project().map(|stack| ItemResource::of(&stack))
        })
        .flatten()
    }

    /// How many are here (`ItemAccess.getAmount`), or zero if the place is empty or unavailable.
    /// Does not project the view to a complete stack.
    pub fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        self.with_stack(host, |stack| stack.count()).unwrap_or(0)
    }

    /// Puts up to `amount` of `resource` here, in transaction `tx`: how much went
    /// (`ItemAccess.insert`). What doesn't stack with what is here, or doesn't fit, goes where
    /// vanilla puts an item split off it: a player's into their inventory, else dropped at the
    /// root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `forInfiniteMaterials`); an item entity's dropped beside it at the root commit; a
    /// block entity's, an entity's or an item's slot's into that container's other slots
    /// (`forHandlerIndex`, refused instead around [`Strict`](Self::Strict)); a mod's place's
    /// where its [`PlaceProvider`] puts it; a [`Scratch`](Self::Scratch) stack's nowhere when lent
    /// by [`HostTransaction::scratch_stack`] and into its second slot when lent by
    /// [`HostTransaction::scratch_slots`]. A player-backed place whose
    /// player is in another level than the hook's (not a creative one) takes only what fits their
    /// inventory, and nothing is dropped: the rest stays the caller's, and [`exchange`](Self::exchange)
    /// rolls back when it cannot insert the whole replacement.
    /// A mismatched `tx` returns `Err(TransactionError::InvalidContext)`.
    pub fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        self.moved(host, tx, resource, amount, true)
    }

    /// Takes up to `amount` from here, in transaction `tx`, when the item and components
    /// match `resource` (`ItemAccess.extract`). Places the game handles directly compare
    /// decoded live components: an override equal to the item's default does not change
    /// the match, but a pending map lock or scale does. The resource need not be valid for
    /// insertion. A creative player's hand gives without losing any.
    /// A mismatched `tx` returns `Err(TransactionError::InvalidContext)`.
    pub fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        self.moved(host, tx, resource, amount, false)
    }

    /// Takes `amount` of what is here and puts as many of `resource` in their place
    /// (`ItemAccess.exchange`): from a bigger stack, they go where [`insert`](Self::insert) puts
    /// what doesn't stack, as vanilla's `ItemUtils.createFilledResult` puts a filled bucket. In a
    /// transaction of its own, nested in `tx`: all of it, or nothing and `Ok(0)`. Inside an item
    /// that shares its place with others like it, it's done in one of them: that one splits off,
    /// changed, and the others stay as they were. A mismatched `tx` returns
    /// `Err(TransactionError::InvalidContext)`.
    pub fn exchange(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let swap = |host: &mut dyn ServerHost, tx: &TransactionContext| {
            let old = self.resource(host).ok_or(None)?;
            let out = self.extract(host, tx, &old, amount).map_err(Some)?;
            if out != amount {
                return Err(None);
            }
            let put = self.insert(host, tx, resource, out).map_err(Some)?;
            match put == out {
                true => Ok(out),
                false => Err(None),
            }
        };
        match host.transaction(|host, tx| alone(self, host, tx, &mut |host, tx| swap(host, tx))) {
            Ok(done) => Ok(done),
            Err(TransactionError::Aborted(None)) => Ok(0),
            Err(TransactionError::Aborted(Some(error))) => Err(error),
            Err(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
            Err(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
            Err(TransactionError::CommitRefused) => Err(TransactionError::CommitRefused),
        }
    }

    /// Runs the item's `ItemStack.onCraftedBy` processing for a stack this place holds, as a
    /// server-side menu take does. This awards the player's `crafted` statistic, applies item
    /// post-processing such as a filled map's pending scale or lock, and runs the normal stat
    /// hook. Only a [`MenuSlot`](Self::MenuSlot) or [`PlayerCursor`](Self::PlayerCursor) is
    /// accepted; the changed stack and statistic are part of `tx`. Returns
    /// [`CraftedByError::CommitRefused`] without queueing the crafted statistic if the host rolls
    /// back the inner replacement commit.
    pub fn on_crafted_by(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        amount: i32,
    ) -> Result<(), crate::CraftedByError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(crate::CraftedByError::InvalidContext);
        }
        if !matches!(self, Self::MenuSlot { .. } | Self::PlayerCursor { .. }) {
            return Err(crate::CraftedByError::NotHeld);
        }
        backend_mut_of(host)
            .ok_or(crate::CraftedByError::Unavailable)?
            .game_on_crafted_by(self, tx, amount)
    }

    /// Edit only the components of the whole stack, without replacing the item, changing its
    /// count or splitting off one item. [`ItemComponentEditor::get`] reads pending overrides;
    /// [`ItemComponentEditor::original`] shows the stack before edits. The callback does not
    /// receive the host and cannot use the host borrowed by this call.
    ///
    /// For a player slot (including armour and off hand), hand, open menu cursor, or a mob's or
    /// armour stand's equipment, the game resolves the occupied slot before lending the editor.
    /// An unavailable backend, a mismatched `tx`, or an empty or unreadable slot refuses before
    /// the callback runs. The game validates the changed overrides together against the stack's
    /// effective limits before writing any of them. Only those changes are applied, in place
    /// like `ItemStack.set`; an unchanged batch needs no write. The result reaches clients
    /// through the usual menu, cursor, inventory or equipment sync, including synchronized
    /// component overrides, without replaying equip sounds, events or attribute effects.
    ///
    /// `Strict` preserves this contract. `OneByOne` and `InfiniteMaterials` refuse before
    /// calling the editor: they do not provide a synced whole-stack write. Other places use
    /// their ordinary exchange rules. A refusal leaves the place unchanged; aborting `tx` or an
    /// enclosing transaction restores a successful write.
    pub fn edit(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        edit: impl FnOnce(&mut ItemComponentEditor<'_>),
    ) -> Result<(), ItemPlaceEditError> {
        if !host.transaction_matches(Some(tx)) {
            return Err(ItemPlaceEditError::InvalidContext);
        }
        let mut place = self;
        while let Self::Strict(inner) = place {
            place = inner;
        }
        if matches!(place, Self::OneByOne(_) | Self::InfiniteMaterials { .. }) {
            return Err(ItemPlaceEditError::UnsupportedPlace);
        }
        if matches!(
            place,
            Self::PlayerSlot { .. }
                | Self::PlayerHand { .. }
                | Self::PlayerCursor { .. }
                | Self::Equipment { .. }
        ) {
            let mut edit = Some(edit);
            return backend_mut_of(host)
                .ok_or(ItemPlaceEditError::Unavailable)?
                .game_edit_stack(place, tx, &mut |editor| {
                    if let Some(edit) = edit.take() {
                        edit(editor);
                    }
                });
        }
        let before = place
            .with_stack(host, |stack| stack.project())
            .flatten()
            .ok_or(ItemPlaceEditError::NotHeld)?;
        let read_override = |id: &str| before.component_value(id).cloned();
        let edits = edit_item_components(ItemStackView::from_stack(&before), &read_override, edit);
        let mut stack = before.clone();
        edits.apply_to(&mut stack);
        if stack == before {
            return Ok(());
        }
        let count = stack.count();
        match self.exchange(host, tx, &ItemResource::of(&stack), count) {
            Ok(moved) if moved == count => Ok(()),
            Ok(_) => Err(ItemPlaceEditError::Unavailable),
            Err(TransactionError::InvalidContext) => Err(ItemPlaceEditError::InvalidContext),
            Err(_) => Err(ItemPlaceEditError::Unavailable),
        }
    }

    /// `insert` or `extract`: the wrappers undone, outermost first, in a loop rather than by
    /// recursion, so a mod may nest them as deep as it likes without the stack growing; then the
    /// place inside, as [`moved_at`](Self::moved_at) moves it. Moving nothing is nothing, in any
    /// place.
    fn moved(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
        insert: bool,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        if amount == 0 {
            return Ok(0);
        }
        let (mut place, mut amount, mut spills) = (self, amount, true);
        loop {
            place = match place {
                ItemPlace::Strict(inner) => {
                    spills = false;
                    inner
                }
                ItemPlace::OneByOne(inner) => {
                    amount = amount.min(1);
                    inner
                }
                ItemPlace::InfiniteMaterials { player, inner } => {
                    let infinite = host
                        .player(*player)
                        .and_then(|player| player.has_infinite_materials(host));
                    if infinite == Some(true) {
                        return infinite_moved(host, tx, *player, inner, resource, amount, insert);
                    }
                    inner
                }
                _ => break,
            };
        }
        place.moved_at(host, tx, resource, amount, insert, spills)
    }

    /// `insert` or `extract` at a place that wraps none, in a transaction of its own: a handler
    /// answering more than it was asked has it undone, and its mod named. An insert into a
    /// handler slot that `spills` sends what the slot doesn't take to the handler's other slots.
    fn moved_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
        insert: bool,
        spills: bool,
    ) -> Result<u32, TransactionError<Overanswered>> {
        let _ = tx;
        // Resolve the menu's backing once. Transfers to a handler-owned stack ask pickup
        // against the current handler contents, not the packet mirror.
        let (amount, backing) = if let ItemPlace::MenuSlot { menu, slot } = self {
            let Some(backing) = menu.slot_backing(host, *slot) else {
                return Ok(0);
            };
            let allowed = if insert {
                let holds = if matches!(backing, MenuSlotBacking::Place(_)) {
                    backend_of(host).and_then(|backend| {
                        backend.menu_slot_holds_with_stack(*menu, *slot, resource.as_stack())
                    })
                } else {
                    menu.slot_holds(host, *slot, resource.as_stack())
                };
                amount.min(holds.unwrap_or(0).saturating_sub(self.amount(host)))
            } else {
                let gives = if matches!(backing, MenuSlotBacking::Place(_)) {
                    self.with_stack(host, |view| view.project())
                        .flatten()
                        .and_then(|stack| {
                            backend_of(host)?.menu_slot_gives_with_stack(*menu, *slot, &stack)
                        })
                } else {
                    menu.slot_gives(host, *slot)
                };
                if gives == Some(true) { amount } else { 0 }
            };
            (allowed, Some(backing))
        } else {
            (amount, None)
        };
        if amount == 0 {
            return Ok(0);
        }
        // Through index `slot` of `handler`, then for an insert that `spills` the whole handler,
        // each in passes while one moves something (`Inventory.add`'s loop: a
        // handler may move less per call than it could): what `slot` doesn't take goes to the
        // other indices (`forHandlerIndex`).
        let through = |handler: Capability<dyn ResourceHandler<ItemResource>>,
                       host: &mut dyn ServerHost,
                       tx: &TransactionContext,
                       slot: usize,
                       spills: bool| {
            let mut left = amount;
            let passes: &[bool] = if insert && spills {
                &[false, true]
            } else {
                &[false]
            };
            for &anywhere in passes {
                while left > 0 {
                    let moved = match (insert, anywhere) {
                        (true, false) => handler.insert_at(host, tx, slot, resource, left),
                        (true, true) => handler.insert(host, tx, resource, left),
                        (false, _) => handler.extract_at(host, tx, slot, resource, left),
                    }
                    .map_err(Some)?;
                    match handler.checked(moved, left, self) {
                        Some(0) => break,
                        Some(moved) => left -= moved,
                        None => {
                            return Err(Some(TransactionError::Aborted(Overanswered {
                                owner: Some(handler.owner()),
                            })));
                        }
                    }
                }
            }
            Ok(amount - left)
        };
        let moved = host.transaction(|host, tx| match self {
            ItemPlace::BlockEntity {
                level,
                pos,
                side,
                slot,
            } => {
                let handler = level.capability(host, &ITEMS, *pos, *side).ok_or(None)?;
                through(handler, host, tx, *slot, spills)
            }
            ItemPlace::Nested { parent, slot } => {
                let handler = parent.capability(host, &ITEM_ITEMS).ok_or(None)?;
                through(handler, host, tx, *slot, spills)
            }
            // The game's own write spreads; a strict one goes through the entity's handler.
            ItemPlace::EntitySlot { entity, slot } if !spills => {
                let entity = host.entity(*entity).ok_or(None)?;
                let handler = entity.capability(host, &ENTITY_ITEMS, ()).ok_or(None)?;
                through(handler, host, tx, *slot, false)
            }
            ItemPlace::MenuSlot { menu, .. } => {
                let (handler, index) = match backing.ok_or(None)? {
                    MenuSlotBacking::Player(index) => {
                        let player = menu.player.entity();
                        (player.capability(host, &ENTITY_ITEMS, ()), index)
                    }
                    MenuSlotBacking::BlockEntity { level, pos, slot } => {
                        (level.capability(host, &ITEMS, pos, None), slot)
                    }
                    MenuSlotBacking::Entity { entity, slot } => {
                        (entity.capability(host, &ENTITY_ITEMS, ()), slot)
                    }
                    MenuSlotBacking::Place(place) => {
                        let (handler, index) = place.handler(host, menu.player).ok_or(None)?;
                        (Some(handler), index)
                    }
                    MenuSlotBacking::CraftResult => return Ok(0),
                    _ => {
                        return Ok(match insert {
                            true => backend_mut_of(host).map_or(0, |backend| {
                                backend.game_insert(self, resource, amount, tx).unwrap_or(0)
                            }),
                            false => backend_mut_of(host).map_or(0, |backend| {
                                backend
                                    .game_extract(self, resource, amount, tx)
                                    .unwrap_or(0)
                            }),
                        });
                    }
                };
                through(handler.ok_or(None)?, host, tx, index, false)
            }
            ItemPlace::Mod { provider, key } => {
                let (provider, owner) = host
                    .capability_providers()
                    .place_provider(provider)
                    .ok_or(None)?;
                let moved = match insert {
                    true => provider.insert(host, tx, *key, resource, amount),
                    false => provider.extract(host, tx, *key, resource, amount),
                }
                .map_err(|mut error| {
                    if let TransactionError::Aborted(over) = &mut error
                        && over.owner.is_none()
                    {
                        warn_broken(Some(owner), self);
                        over.owner = Some(owner.to_owned());
                    }
                    Some(error)
                })?;
                if moved > amount {
                    warn_broken(Some(owner), self);
                    return Err(Some(TransactionError::Aborted(Overanswered {
                        owner: Some(owner.to_owned()),
                    })));
                }
                Ok(moved)
            }
            _ => Ok(match insert {
                true => backend_mut_of(host).map_or(0, |backend| {
                    backend.game_insert(self, resource, amount, tx).unwrap_or(0)
                }),
                false => backend_mut_of(host).map_or(0, |backend| {
                    backend
                        .game_extract(self, resource, amount, tx)
                        .unwrap_or(0)
                }),
            }),
        });
        match moved {
            Ok(moved) => Ok(moved),
            Err(TransactionError::Aborted(None)) => Ok(0),
            Err(TransactionError::Aborted(Some(error))) => Err(error),
            Err(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
            Err(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
            Err(TransactionError::CommitRefused) => Err(TransactionError::CommitRefused),
        }
    }
}

/// `forInfiniteMaterials` while `player` has infinite materials, reaching `inner`: an extract
/// gives what `inner` holds without taking it; an insert is taken whole, and one of it goes to
/// the player only if their inventory has none (`ItemUtils.createFilledResult`). It goes in as
/// an insert at their slot 0 puts it, which `Inventory.add` places for an item they don't
/// have: the first empty slot, else dropped at the root commit.
fn infinite_moved(
    host: &mut dyn ServerHost,
    tx: &TransactionContext,
    player: u128,
    inner: &ItemPlace,
    resource: &ItemResource,
    amount: u32,
    insert: bool,
) -> Result<u32, TransactionError<Overanswered>> {
    if !insert {
        return Ok(inner
            .with_stack(host, |held| {
                if resource.matches_view(&held) {
                    amount.min(held.count())
                } else {
                    0
                }
            })
            .unwrap_or(0));
    }
    let had = host
        .player(player)
        .and_then(|player| {
            player.with_inventory(host, |inventory| {
                (0..36)
                    .filter_map(|slot| inventory.get(slot))
                    .any(|held| resource.matches_view(&held))
            })
        })
        .unwrap_or(false);
    if !had {
        ItemPlace::PlayerSlot { player, slot: 0 }.insert(host, tx, resource, 1)?;
    }
    Ok(amount)
}

/// Runs `f` with each item `place` is inside alone in its place, the outermost first
/// ([`one_of`] at every level): the writes `f` makes through them, however many, all reach the
/// same one, which alone changes.
fn alone(
    place: &ItemPlace,
    host: &mut dyn ServerHost,
    tx: &TransactionContext,
    f: &mut dyn FnMut(
        &mut dyn ServerHost,
        &TransactionContext,
    ) -> Result<u32, Option<TransactionError<Overanswered>>>,
) -> Result<u32, Option<TransactionError<Overanswered>>> {
    match place {
        ItemPlace::Nested { parent, .. } => alone(parent, host, tx, &mut |host, tx| {
            one_of(parent, host, tx, &mut *f)
        }),
        _ => f(host, tx),
    }
}

/// Runs `f` on one item of the stack at `parent` (NeoForge's `oneByOne`): from a stack of more,
/// the others are taken out first and put back after, and if `f` changed the one, it goes where
/// an exchange puts it. All written in place in `tx`, so an abort puts every level back.
fn one_of(
    parent: &ItemPlace,
    host: &mut dyn ServerHost,
    tx: &TransactionContext,
    f: impl FnOnce(
        &mut dyn ServerHost,
        &TransactionContext,
    ) -> Result<u32, Option<TransactionError<Overanswered>>>,
) -> Result<u32, Option<TransactionError<Overanswered>>> {
    let split = parent
        .with_stack(host, |held| {
            let others = held.count() - 1;
            if others == 0 {
                Ok::<_, Option<TransactionError<Overanswered>>>(None)
            } else {
                let stack = held.project().ok_or(None)?;
                Ok(Some((others, ItemResource::of(&stack))))
            }
        })
        .ok_or(None)??;
    let Some((others, resource)) = split else {
        return f(host, tx);
    };
    let all = |moved: u32, asked: u32| (moved == asked).then_some(()).ok_or(None);
    all(
        parent.extract(host, tx, &resource, others).map_err(Some)?,
        others,
    )?;
    let moved = f(host, tx)?;
    let changed = parent.resource(host).filter(|now| *now != resource);
    if let Some(changed) = &changed {
        all(parent.extract(host, tx, changed, 1).map_err(Some)?, 1)?;
    }
    all(
        parent.insert(host, tx, &resource, others).map_err(Some)?,
        others,
    )?;
    if let Some(changed) = &changed {
        all(parent.insert(host, tx, changed, 1).map_err(Some)?, 1)?;
    }
    Ok(moved)
}

impl ItemPlace {
    /// `key`'s handler for the item here (`ItemAccess.getCapability`): the first provider to
    /// answer, of those mods registered for its item `before_game`, the game's own, and the
    /// mods' others, each in registration order. Disabled mods' providers are skipped until
    /// their mod is re-enabled.
    pub fn capability<T: ?Sized + 'static>(
        &self,
        host: &mut dyn ServerHost,
        key: &ItemCapabilityKey<T>,
    ) -> Option<Capability<T>> {
        // Capture the item before dispatch. Only a stack-sensitive game default needs an
        // owned value across the mutable provider calls; don't read the place again on fallback.
        let (item, projected) = self
            .with_stack(host, |stack| {
                let projected = match &key.default {
                    Some(ItemCapabilityDefault::Stack(_)) => Some(stack.project()?),
                    _ => None,
                };
                Some((stack.item(), projected))
            })
            .flatten()?;
        let providers = crate::parts::installed()
            .and_then(|installed| installed.keys.item_capabilities.get(key.name))
            .and_then(|by_item| {
                let id = crate::content::registry_keys::item_id(item)?;
                by_item.get(id).map(Vec::as_slice)
            })
            .unwrap_or(&[]);
        let mut game = None;
        crate::answered(
            crate::ask_providers(
                host,
                providers.iter(),
                ("capability provider", "capability_provider"),
                |provider, host| {
                    provider
                        .get::<std::sync::Arc<crate::ItemProvider<T>>>()?
                        .provide(host, self)
                },
                |host| {
                    game = match &key.default {
                        Some(ItemCapabilityDefault::Item(default)) => default(host, self, item),
                        Some(ItemCapabilityDefault::Stack(default)) => projected
                            .as_ref()
                            .and_then(|stack| default(host, self, stack)),
                        None => None,
                    };
                    game.is_some()
                },
            ),
            game,
        )
    }
}

#[cfg(test)]
#[path = "item_place_wrapper_tests.rs"]
mod wrapper_tests;

#[cfg(test)]
#[path = "item_place_scratch_tests.rs"]
mod scratch_tests;
