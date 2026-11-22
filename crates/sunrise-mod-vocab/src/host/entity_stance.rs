//! The `entity_stance` area: how an entity holds itself: spectating, crouching, sprinting, swimming, its pose. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_stance` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityStanceBackend {
    /// Side: server.
    /// `Entity.isSpectator`: true for a player in spectator mode, false for
    /// every other entity.
    fn entity_spectator(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `isSecondaryUseActive`: the local client's live semantic sneak state (not the
    /// physical key binding or last tick's crouching pose); the server's shift-key flag.
    /// False for non-player entities, None for a stale or unreachable handle.
    fn entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity crouches (`Entity.isCrouching`: the crouching
    /// pose).
    fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity sprints (`Entity.isSprinting`): a player, or a
    /// cat or ocelot dashing after its target (their shared sprinting flag);
    /// false for the other kinds.
    fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The player's swimming flag (`Entity.isSwimming`), not the swimming pose
    /// (which also covers crawling). On a client, the local player's own state,
    /// not an authoritative server answer. None for other players (a client keeps them as
    /// tracked bodies without the flag), other kinds and stale handles.
    fn entity_is_swimming(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The entity's pose (`Entity.getPose`) as `Pose` names it: `standing`,
    /// `crouching`, `swimming`, `sleeping`, a warden's `roaring`... Kinds
    /// with no pose stand.
    fn entity_pose(&self, entity: EntityRef) -> Option<&'static str>;
}
/// The handles' writes of the `entity_stance` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityStanceBackendMut {
    /// Side: server.
    /// Sets whether the entity sprints (`Entity.setSprinting`): a player's flag with the
    /// movement speed modifier `LivingEntity.setSprinting` adds or removes, or a mob's flag,
    /// which the mob's movement speed reads as that same modifier (a mob's own AI, such as a
    /// cat's, may set it again on its next step). It is not the player's sprint key: the
    /// player's client sets its own flag again from its input. False for a stale handle, on a
    /// client, and for kinds with no sprinting flag.
    fn entity_set_sprinting(&mut self, entity: EntityRef, sprinting: bool) -> bool;
}
