//! Scoped views and sequential host loans for entity-scoped and item-entity-scoped hooks.
use crate::{
    ClientHost, EntityRef, EntityTypeKey, ItemEntityRef, ItemStack, ItemStackView, ServerHost,
    StackWriteError, Transaction, WorldEpoch,
};

/// Live resident view of a server general entity row during `general_entity_tick`.
/// Keep an [`EntityRef`] from [`Self::handle`] if you need to reference it after the call
/// or through `host()`.
// exhaustive: only the engine constructs a view.
// no wire: an exclusive loan, not a payload.
pub struct EntitySubjectView<'a> {
    subject: EntityRef,
    data: &'a mut dyn ResidentGeneralEntity,
}

impl<'a> EntitySubjectView<'a> {
    // hidden: the engine builds the view after checking the resident general row.
    #[doc(hidden)]
    pub fn new(subject: EntityRef, data: &'a mut dyn ResidentGeneralEntity) -> Self {
        Self { subject, data }
    }

    /// A handle for this entity to keep across calls or pass to Host operations.
    pub fn handle(&self) -> EntityRef {
        self.subject
    }
    /// The entity's runtime id within its world epoch.
    pub fn id(&self) -> u32 {
        self.subject.id()
    }
    /// The world epoch this entity belongs to.
    pub fn epoch(&self) -> WorldEpoch {
        self.subject.epoch()
    }
    /// The general entity's registered type id (for example `"mymod:robot"`).
    pub fn kind(&self) -> &str {
        self.data.kind()
    }
    /// The entity's current position in blocks (`Entity.position`).
    pub fn position(&self) -> [f64; 3] {
        self.data.position()
    }
    /// The block containing the entity's feet (`Entity.blockPosition`).
    pub fn block_position(&self) -> [i32; 3] {
        let [x, y, z] = self.data.position();
        [x.floor() as i32, y.floor() as i32, z.floor() as i32]
    }
    /// The entity's current velocity in blocks per tick.
    pub fn velocity(&self) -> [f64; 3] {
        self.data.velocity()
    }
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`).
    pub fn rotation(&self) -> [f32; 2] {
        self.data.rotation()
    }
    /// The entity's persistent UUID.
    pub fn uuid(&self) -> u128 {
        self.data.uuid()
    }

    /// Set the entity's velocity in blocks per tick. Returns `false` if any component is non-finite.
    pub fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        self.data.set_velocity(velocity)
    }
    /// Turn the entity to `yaw` and `pitch` in degrees; `pitch` wraps and clamps to `-90..=90`.
    /// Returns `false` if either angle is non-finite.
    pub fn set_rotation(&mut self, yaw: f32, pitch: f32) -> bool {
        self.data.set_rotation(yaw, pitch)
    }
    /// Forget accumulated fall distance (`Entity.resetFallDistance`).
    pub fn reset_fall_distance(&mut self) {
        self.data.reset_fall_distance();
    }
}

/// Sequential access to a ticking server general entity and the server Host.
///
/// ```compile_fail,E0501
/// use mod_vocab::GeneralEntityTickEvent;
/// fn tick(event: &mut GeneralEntityTickEvent<'_>) {
///     event.subject.with_view(|entity| {
///         let _host = event.subject.host();
///         let _pos = entity.position();
///     });
/// }
/// ```
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the sequential loans cannot escape their call.
pub struct EntityServerScope<'a>(&'a mut dyn EntityServerAccess);

impl<'a> EntityServerScope<'a> {
    // hidden: the engine supplies the loan backend for this general entity tick.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn EntityServerAccess) -> Self {
        Self(access)
    }

    /// Lend the live general entity row for `f`. Once `f` returns, you can use the Host
    /// and ask for another loan afterwards; each loan re-checks residency and returns
    /// `None` without running `f` if the entity was removed or left the level.
    pub fn with_view<R>(&mut self, f: impl FnOnce(EntitySubjectView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the server Host for attachments, other objects or world operations.
    /// No view is live while this borrow is held; reborrow the view afterwards to re-check it.
    pub fn host(&mut self) -> &mut dyn ServerHost {
        self.0.host()
    }

    /// Commit the view's velocity, rotation and fall-distance writes together with other
    /// journaled Host writes on `Ok`. Returning `Err` restores those writes.
    pub fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut Self, &Transaction) -> Result<T, E>,
    ) -> Result<T, E> {
        crate::item::transaction::run_transaction_on(
            self,
            |scope| scope.0.host().transaction_open(),
            |scope, seal, commit| scope.0.host().transaction_close(seal, commit),
            f,
        )
    }
}

/// Read-only live view of a client-side entity during an entity-scoped client hook
/// (`can_rider_interact`).
// exhaustive: only the engine constructs a view.
// no wire: a borrowed view, not a payload.
pub struct EntityClientView<'a> {
    subject: EntityRef,
    data: &'a dyn ResidentClientEntity,
}

impl<'a> EntityClientView<'a> {
    // hidden: the engine builds the view after checking the client entity.
    #[doc(hidden)]
    pub fn new(subject: EntityRef, data: &'a dyn ResidentClientEntity) -> Self {
        Self { subject, data }
    }

    /// A handle for this client entity to keep across calls or pass to Host operations.
    pub fn handle(&self) -> EntityRef {
        self.subject
    }
    /// The entity's runtime id within its world epoch.
    pub fn id(&self) -> u32 {
        self.subject.id()
    }
    /// The world epoch this entity belongs to.
    pub fn epoch(&self) -> WorldEpoch {
        self.subject.epoch()
    }
    /// The entity's resolved type key, if registered.
    pub fn kind(&self) -> Option<EntityTypeKey> {
        self.data.kind()
    }
    /// The client entity's position in blocks.
    pub fn position(&self) -> [f64; 3] {
        self.data.position()
    }
    /// The block containing the client entity's position.
    pub fn block_position(&self) -> [i32; 3] {
        let [x, y, z] = self.data.position();
        [x.floor() as i32, y.floor() as i32, z.floor() as i32]
    }
    /// The client entity's yaw and pitch in degrees.
    pub fn rotation(&self) -> [f32; 2] {
        self.data.rotation()
    }
    /// The entity's UUID if known on this client.
    pub fn uuid(&self) -> Option<u128> {
        self.data.uuid()
    }
}

/// Sequential access to a client-side entity and the mutable client Host.
///
/// ```compile_fail,E0501
/// use mod_vocab::CanRiderInteractEvent;
/// fn check(event: &mut CanRiderInteractEvent<'_>) {
///     event.subject.with_view(|entity| {
///         let _host = event.subject.host();
///         let _pos = entity.position();
///     });
/// }
/// ```
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the sequential loans cannot escape their call.
pub struct EntityClientScope<'a>(&'a mut dyn EntityClientAccess);

impl<'a> EntityClientScope<'a> {
    // hidden: the engine supplies the loan backend for this client entity dispatch.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn EntityClientAccess) -> Self {
        Self(access)
    }

    // hidden: dispatch routes entity parts from the live subject without a separate Host.
    #[doc(hidden)]
    pub fn route_kind(&self) -> Option<&str> {
        self.0
            .route_kind()
            .and_then(crate::content::registry_keys::entity_type_name)
    }

    /// Lend the client entity view for `f`. Returns `None` if the entity is no longer resident.
    pub fn with_view<R>(&mut self, f: impl FnOnce(EntityClientView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the mutable client Host. No view is live while this borrow is held.
    pub fn host(&mut self) -> &mut dyn ClientHost {
        self.0.host()
    }
}

/// Live resident view of a server dropped-item entity during `on_entity_item_update`.
/// Keep an [`ItemEntityRef`] from [`Self::handle`] if you need to reference it after the call
/// or through `host()`.
// exhaustive: only the engine constructs a view.
// no wire: an exclusive loan, not a payload.
pub struct ItemEntitySubjectView<'a> {
    subject: ItemEntityRef,
    data: &'a mut dyn ResidentItemEntity,
}

impl<'a> ItemEntitySubjectView<'a> {
    // hidden: the engine builds the view after checking the resident dropped item.
    #[doc(hidden)]
    pub fn new(subject: ItemEntityRef, data: &'a mut dyn ResidentItemEntity) -> Self {
        Self { subject, data }
    }

    /// A typed dropped-item handle to keep across calls or pass to Host operations.
    pub fn handle(&self) -> ItemEntityRef {
        self.subject
    }
    /// The underlying entity handle (`EntityRef`).
    pub fn entity(&self) -> EntityRef {
        self.subject.entity()
    }
    /// The dropped item's position in blocks.
    pub fn position(&self) -> [f64; 3] {
        self.data.position()
    }
    /// The block containing the dropped item's position.
    pub fn block_position(&self) -> [i32; 3] {
        let [x, y, z] = self.data.position();
        [x.floor() as i32, y.floor() as i32, z.floor() as i32]
    }
    /// The dropped item's velocity in blocks per tick.
    pub fn velocity(&self) -> [f64; 3] {
        self.data.velocity()
    }
    /// Lend the live item stack without copying components unless projected (`ItemEntity.getItem`).
    pub fn with_stack<R>(&self, f: impl FnOnce(ItemStackView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.data.with_stack(&mut |stack| {
            if let Some(f) = f.take() {
                result = Some(f(stack));
            }
        });
        result
    }
    /// The signed pickup delay; the infinite sentinel reads as vanilla's `32767`.
    /// `None` if a stored value is outside `i32`.
    pub fn pickup_delay(&self) -> Option<i32> {
        self.data.pickup_delay()
    }
    /// `ItemEntity.getAge`, including negative lifetime sentinels.
    pub fn age(&self) -> i32 {
        self.data.age()
    }
    /// The UUID set by `ItemEntity.setTarget`, limiting pickup to that player.
    pub fn target(&self) -> Option<u128> {
        self.data.target()
    }
    /// `ItemEntity.getOwner`: the thrower as a resolved entity, if resident in this level.
    pub fn owner(&self) -> Option<EntityRef> {
        self.data.owner()
    }

    /// Replace the item entity's stack (`ItemEntity.setItem`), journaled under any open
    /// transaction. Empty stacks are refused and return `Ok(false)`.
    pub fn set_stack(&mut self, stack: ItemStack) -> Result<bool, StackWriteError> {
        self.data.set_stack(stack)
    }
    /// `ItemEntity.setPickUpDelay`.
    pub fn set_pickup_delay(&mut self, ticks: i32) -> bool {
        self.data.set_pickup_delay(ticks)
    }
    /// `ItemEntity.setDefaultPickUpDelay`: ten ticks.
    pub fn set_default_pickup_delay(&mut self) -> bool {
        self.data.set_pickup_delay(10)
    }
    /// `ItemEntity.setNoPickUpDelay`.
    pub fn set_no_pickup_delay(&mut self) -> bool {
        self.data.set_pickup_delay(0)
    }
    /// `ItemEntity.setNeverPickUp`: the infinite-delay sentinel `32767`.
    pub fn set_never_pickup(&mut self) -> bool {
        self.data.set_pickup_delay(i32::from(i16::MAX))
    }
    /// `ItemEntity.setTarget`; `None` allows any player to pick it up.
    pub fn set_target(&mut self, target: Option<u128>) -> bool {
        self.data.set_target(target)
    }
    /// Set the dropped item's velocity in blocks per tick. Returns `false` if non-finite.
    pub fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        self.data.set_velocity(velocity)
    }
}

/// Sequential access to a ticking server dropped-item entity and the server Host.
///
/// ```compile_fail,E0501
/// use mod_vocab::OnEntityItemUpdateEvent;
/// fn update(event: &mut OnEntityItemUpdateEvent<'_>) {
///     event.subject.with_view(|item| {
///         let _host = event.subject.host();
///         let _age = item.age();
///     });
/// }
/// ```
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the sequential loans cannot escape their call.
pub struct ItemEntityServerScope<'a> {
    access: &'a mut dyn ItemEntityServerAccess,
    item_id: &'a str,
}

impl<'a> ItemEntityServerScope<'a> {
    // hidden: the engine supplies the loan backend and the item ID at dispatch entry.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn ItemEntityServerAccess, item_id: &'a str) -> Self {
        Self { access, item_id }
    }

    pub(crate) fn route_item(&self) -> &str {
        self.item_id
    }

    /// Lend the live dropped-item entity for `f`. Once `f` returns, you can use the Host
    /// and ask for another loan afterwards; each loan re-checks residency and returns
    /// `None` without running `f` if the item was removed.
    pub fn with_view<R>(&mut self, f: impl FnOnce(ItemEntitySubjectView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.access.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the server Host for attachments, removal or other world operations.
    /// No view is live while this borrow is held; reborrow the view afterwards to re-check it.
    pub fn host(&mut self) -> &mut dyn ServerHost {
        self.access.host()
    }

    /// Commit the view's stack, pickup-delay, target and velocity writes together with other
    /// journaled Host writes on `Ok`. Returning `Err` restores those writes.
    pub fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut Self, &Transaction) -> Result<T, E>,
    ) -> Result<T, E> {
        crate::item::transaction::run_transaction_on(
            self,
            |scope| scope.access.host().transaction_open(),
            |scope, seal, commit| scope.access.host().transaction_close(seal, commit),
            f,
        )
    }
}

/// Engine backing for a resident general entity row loan.
// hidden: the engine owns row access; listeners use EntitySubjectView.
#[doc(hidden)]
pub trait ResidentGeneralEntity {
    fn kind(&self) -> &str;
    fn position(&self) -> [f64; 3];
    fn velocity(&self) -> [f64; 3];
    fn rotation(&self) -> [f32; 2];
    fn uuid(&self) -> u128;
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool;
    fn set_rotation(&mut self, yaw: f32, pitch: f32) -> bool;
    fn reset_fall_distance(&mut self);
}

/// Engine backing for sequential general-entity view and server Host loans.
// hidden: the engine owns alternating view and Host loans; listeners receive EntityServerScope.
#[doc(hidden)]
pub trait EntityServerAccess {
    fn with_view(&mut self, f: &mut dyn FnMut(EntitySubjectView<'_>));
    fn host(&mut self) -> &mut dyn ServerHost;
}

/// Engine backing for a read-only resident client entity view.
// hidden: the engine owns client entity reads; listeners use EntityClientView.
#[doc(hidden)]
pub trait ResidentClientEntity {
    fn kind(&self) -> Option<EntityTypeKey>;
    fn position(&self) -> [f64; 3];
    fn rotation(&self) -> [f32; 2];
    fn uuid(&self) -> Option<u128>;
}

/// Engine backing for sequential client entity view and mutable client Host loans.
// hidden: the engine owns alternating view and Host loans; listeners receive EntityClientScope.
#[doc(hidden)]
pub trait EntityClientAccess {
    fn route_kind(&self) -> Option<EntityTypeKey>;
    fn with_view(&mut self, f: &mut dyn FnMut(EntityClientView<'_>));
    fn host(&mut self) -> &mut dyn ClientHost;
}

/// Engine backing for a resident dropped-item entity loan.
// hidden: the engine owns dropped-item access and journaling; listeners use ItemEntitySubjectView.
#[doc(hidden)]
pub trait ResidentItemEntity {
    fn position(&self) -> [f64; 3];
    fn velocity(&self) -> [f64; 3];
    fn with_stack(&self, f: &mut dyn for<'a> FnMut(ItemStackView<'a>));
    fn pickup_delay(&self) -> Option<i32>;
    fn age(&self) -> i32;
    fn target(&self) -> Option<u128>;
    fn owner(&self) -> Option<EntityRef>;
    fn set_stack(&mut self, stack: ItemStack) -> Result<bool, StackWriteError>;
    fn set_pickup_delay(&mut self, ticks: i32) -> bool;
    fn set_target(&mut self, target: Option<u128>) -> bool;
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool;
}

/// Engine backing for sequential dropped-item view and server Host loans.
// hidden: the engine owns alternating view and Host loans; listeners receive ItemEntityServerScope.
#[doc(hidden)]
pub trait ItemEntityServerAccess {
    fn with_view(&mut self, f: &mut dyn FnMut(ItemEntitySubjectView<'_>));
    fn host(&mut self) -> &mut dyn ServerHost;
}
