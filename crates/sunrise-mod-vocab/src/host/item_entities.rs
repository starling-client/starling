//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `item_entities` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ItemEntitiesHost {
    /// Side: both.
    /// Narrows an entity to a dropped item, or None for another kind, a
    /// stale handle, a client, or an item temporarily taken out for ticking.
    fn item_entity(&self, entity: EntityRef) -> Option<ItemEntityRef>;
}
/// The handles' reads of the `item_entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ItemEntitiesBackend {
    /// Side: server.
    /// Lend `ItemEntity.getItem` once, as in [`ItemEntityRef::with_stack`].
    #[manual_handle]
    fn item_entity_stack(
        &self,
        item: ItemEntityRef,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()>;
    /// Side: server.
    /// The signed pickup delay (`ItemEntity.pickupDelay`). `32767` is vanilla's
    /// infinite-delay sentinel (`set_never_pickup`). `None` when the handle is
    /// unavailable or outside a server.
    fn item_entity_pickup_delay(&self, item: ItemEntityRef) -> Option<i32>;
    /// Side: server.
    /// `ItemEntity.getAge`, including negative lifetime sentinels.
    fn item_entity_age(&self, item: ItemEntityRef) -> Option<i32>;
    /// Side: server.
    /// The UUID set by `ItemEntity.setTarget`, limiting pickup to that player;
    /// None also means an unavailable item. This is not the thrower.
    fn item_entity_target(&self, item: ItemEntityRef) -> Option<u128>;
    /// Side: server.
    /// `ItemEntity.getOwner`: the thrower as a resolved entity, not the pickup
    /// target. None for no thrower, an unloaded or different-level thrower,
    /// an unavailable handle, or one Sunrise cannot resolve.
    fn item_entity_owner(&self, item: ItemEntityRef) -> Option<EntityRef>;
}
/// The handles' writes of the `item_entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ItemEntitiesBackendMut {
    /// Side: server.
    /// Writes a stack using `ItemEntity.setItem`. Passing `None` or an air stack
    /// stores an empty stack, which normal `ItemEntity.tick` discards on its next
    /// tick (use the removal handle to discard immediately). `Ok(false)` for an
    /// unavailable item or outside a server.
    ///
    /// The new stack is visible immediately. This write joins the item's level's
    /// innermost transaction, if any; abort restores the old stack for the same resident
    /// item. A block-entity tick does not open a transaction automatically;
    /// use the tick scope's `transaction` to commit this write and a subject
    /// edit together.
    fn item_entity_set_stack(
        &mut self,
        item: ItemEntityRef,
        stack: Option<ItemStack>,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// `ItemEntity.setPickUpDelay`. An open transaction restores the previous delay on abort.
    fn item_entity_set_pickup_delay(&mut self, item: ItemEntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// `ItemEntity.setDefaultPickUpDelay`: ten ticks.
    fn item_entity_set_default_pickup_delay(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setNoPickUpDelay`.
    fn item_entity_set_no_pickup_delay(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setNeverPickUp`: the infinite-delay sentinel 32767.
    fn item_entity_set_never_pickup(&mut self, item: ItemEntityRef) -> bool;
    /// Side: server.
    /// `ItemEntity.setTarget`; None allows any player to pick it up.
    /// An open transaction restores the previous target on abort.
    fn item_entity_set_target(&mut self, item: ItemEntityRef, target: Option<u128>) -> bool;
}
