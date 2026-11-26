//! The `entity_riding` area: vehicles and passengers. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_riding` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityRidingBackend {
    /// Side: both.
    /// What the entity rides (`Entity.getVehicle`): `None` when it rides
    /// nothing, its actual vehicle is retained off-level and has no resident handle,
    /// or the supplied handle is stale. `is_passenger` still reports the relationship.
    /// On a client, the vehicle the passenger has boarded from the server's list, if
    /// this window has a handle for it. Lists for unknown vehicles or entities that
    /// cannot accept passengers (markers and ominous item spawners) have no effect.
    fn entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef>;
    /// Side: both.
    /// Whether the entity rides another (`Entity.isPassenger`), including a retained
    /// off-level vehicle that cannot be returned by `vehicle`. On a client, whether the
    /// entity boarded from the server's passenger list. Stale or untracked handles return `None`.
    fn entity_is_passenger(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The lowest vehicle under the entity (`Entity.getRootVehicle`): the
    /// entity itself when it rides nothing. Traverses retained off-level intermediaries,
    /// but returns `None` if the actual root has no resident handle, the supplied handle
    /// is stale, or the relationship contains a cycle. Never substitutes a descendant.
    /// On a client, follows the server's passenger lists and returns the root only if
    /// this window has a handle for it.
    fn entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef>;
    /// Side: both.
    /// Visit the entities riding this one (`Entity.getPassengers`), in `addPassenger`'s order:
    /// each behind those aboard, except that a server player goes first when the first is no
    /// player. Only resident handles are visited; retained off-level passengers are omitted.
    /// Dying passengers stay aboard until removed. On a client, unknown passengers and
    /// links that would form a cycle do not board; the visit follows packet seat order and
    /// does not require a projected vehicle body. Return `ControlFlow::Break(())` to stop
    /// (`Ok(Break(()))`). `Ok(Continue(()))` means the visit completed, including when no
    /// passenger has a handle. A stale or untracked vehicle returns [`EntityReadError::Stale`];
    /// an unavailable world view or a parked server body returns [`EntityReadError::Unavailable`].
    /// A refusal never calls `visit`.
    fn entity_visit_passengers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, EntityReadError>;
    /// Side: both.
    /// Whether anything rides the entity (`Entity.isVehicle`), including retained
    /// off-level passengers omitted from `visit_passengers`. Dying is not removal.
    /// On a client, whether any passenger boarded from the server's list.
    /// Stale or untracked handles return `None`.
    fn entity_is_vehicle(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// Whether the HUD draws this entity's hearts in place of the food bar while the local
    /// player rides it (`Entity.showVehicleHealth`): true for a living entity
    /// (`LivingEntity`: a mob, a player, an armor stand or the dragon), false for anything
    /// else, such as a boat or a minecart. On a client, any entity the server shows this
    /// window answers from its type. `None` for a stale or untracked handle.
    fn entity_shows_vehicle_health(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Visit every entity riding this one, at any depth (`Entity.getIndirectPassengers`): each
    /// passenger in boarding order, followed by those riding it. Traverses retained off-level
    /// intermediaries without lending a handle for them; resident descendants are still visited.
    /// Return `ControlFlow::Break(())` to stop (`Ok(Break(()))`). `Ok(Continue(()))` means
    /// the visit completed, including when there are no resident descendants. A stale vehicle
    /// returns [`EntityReadError::Stale`]; a parked vehicle, a client or an unavailable world view
    /// returns [`EntityReadError::Unavailable`]. A refusal never calls `visit`.
    fn entity_visit_indirect_passengers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, EntityReadError>;
    /// Side: server.
    /// The first passenger, when it steers the entity (`Entity.getControllingPassenger`): a
    /// player in a boat, a player on a mount that is ready for one, or a mob on a mob with AI
    /// unless `#non_controlling_rider` names it. `None` when nothing steers it, or for a stale
    /// handle. The actual first passenger is chosen before filtering by residency:
    /// an off-level first passenger returns `None`, never a later resident substitute.
    fn entity_controlling_passenger(&self, entity: EntityRef) -> Option<LivingRef>;
    /// Side: server.
    /// Whether both entities share a root vehicle (`Entity.isPassengerOfSameVehicle`): true for
    /// an entity and itself, false for two distinct entities riding nothing. Compares
    /// actual object roots even when they are retained off-level and have no handle.
    /// `None` for stale handles or a cycle with no root.
    fn entity_is_passenger_of_same_vehicle(
        &self,
        entity: EntityRef,
        other: EntityRef,
    ) -> Option<bool>;
    /// Side: server.
    /// Where getting off this vehicle puts `passenger` as it stands now, riding it or not. This
    /// is the game's own placement, from the code `stop_riding` runs, so it can differ from
    /// `Entity.getDismountLocationForPassenger`. A player leaves a boat ahead of them or on top
    /// of it, a cart beside or on top of it, a strider by vanilla's search, a happy ghast on
    /// top, and any other mob beside or on top of it. A mob leaves a boat as a player does, a
    /// cart by vanilla's search, a horse, camel or llama at its side or feet, and any other mob
    /// on top: vanilla's searches for `Animal` and the strider do not reach mobs, and those
    /// for horses, `Animal` and the cart don't reach players. Nothing rides anything else, so
    /// there it is the top of the box. Reads only: no `mob_ride` or dismount veto is asked and
    /// the passenger's pose stays. `None` for a stale handle, a passenger other than a player
    /// or a mob, a player with no motion state, or a vehicle whose box Sunrise does not keep.
    fn entity_dismount_location(
        &self,
        vehicle: EntityRef,
        passenger: LivingRef,
    ) -> Option<[f64; 3]>;
}
/// The handles' writes of the `entity_riding` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityRidingBackendMut {
    /// Side: server.
    /// Attempts to board a vehicle through the shared server mount path, and names the cause of
    /// a refusal (`StartRidingRefusal`).
    ///
    /// Players, mobs and declared general entities can ride a mob, boat, minecart, declared
    /// general vehicle or generic seat (such as an armor stand or display). A general type with
    /// `NoSave` can ride but cannot carry a passenger, even when forced. A successful mob mount
    /// drops its lead as an item without a sound and emits `entity_unleash`; a refused mount
    /// leaves the lead alone. Mob capacity follows its species. Boats and minecarts keep their
    /// vehicle limits even when forced; other seats admit one passenger unless forced.
    /// Player boarding cooldown and the strider's
    /// eye-in-lava admission check are not represented by the game.
    /// A player vehicle is refused as `StartRidingRefusal::PlayerVehicle`, forced or not:
    /// vanilla's `Entity.startRiding` refuses a vehicle whose entity type is not saved with the
    /// level before it reads `force`, and the player type is one, as a player's save keeps no
    /// passengers. `entity_mount` fires only after every other check passed, and its cancel is
    /// `StartRidingRefusal::Canceled`. Only `entity_ride_player` seats a mob on a player.
    fn entity_start_riding(
        &mut self,
        entity: EntityRef,
        vehicle: EntityRef,
        force: bool,
    ) -> Result<(), StartRidingRefusal>;
    /// Side: server.
    /// Seats the mob `entity` on the server player `player`
    /// (`Entity.startRiding(player, force)`), as Tinkers' Construct's piggyback
    /// pack does: the mob sits at the player's passenger point (the top of its
    /// box for its pose, less the mob's own vehicle attachment), follows it
    /// every tick, and leaves a vehicle it rode first. Every client is told:
    /// those who track the player by the tracker, and the player's own client,
    /// which no tracker tells of its own entity, as the pack sends it
    /// `ClientboundSetPassengersPacket` itself. `force` bypasses `canRide` and
    /// the player's one-passenger limit (`Entity.canAddPassenger`), as in
    /// vanilla. `entity_mount` hears the mount, and the dismount from an old
    /// vehicle, and may cancel either. `entity_stop_riding` and
    /// `entity_eject_passengers` take it off again. It also gets off, with
    /// `entity_mount` told, at every save of the level or its entities and when
    /// the player logs out, dies or changes level, so it is saved standing where
    /// it is: a player's save keeps no passengers.
    fn entity_ride_player(
        &mut self,
        entity: EntityRef,
        player: EntityRef,
        force: bool,
    ) -> Result<(), RidePlayerRefusal>;
    /// Side: server.
    /// Gets the entity off its vehicle (`Entity.stopRiding`), where the
    /// vehicle lets it out, as `/ride ... dismount` does. False when it rode
    /// nothing, an `entity_mount` listener kept it on, or for a stale handle.
    fn entity_stop_riding(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Gets every passenger off the entity (`Entity.ejectPassengers`), last
    /// first as vanilla walks the list, so `mob_ride` hears them in its
    /// order. False when nothing rode it, or for a stale handle.
    fn entity_eject_passengers(&mut self, entity: EntityRef) -> bool;
}
