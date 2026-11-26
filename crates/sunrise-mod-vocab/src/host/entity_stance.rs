//! The `entity_stance` area: how an entity holds itself: spectating, crouching, sprinting, swimming, its pose. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_stance` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityStanceBackend {
    /// Side: both.
    /// `Entity.isSpectator`: true for a player in spectator mode, false for
    /// every other entity. On a client, a player's game mode as its `PlayerInfo` says
    /// (`AbstractClientPlayer.gameMode`), the local player's and every tracked player's;
    /// false while that `PlayerInfo` is missing. None for a stale handle.
    fn entity_spectator(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `isSecondaryUseActive`: the local client's live semantic sneak state (not the
    /// physical key binding or last tick's crouching pose); the server's shift-key flag.
    /// False for non-player entities, None for a stale or unreachable handle.
    fn entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity crouches (`Entity.isCrouching`: the crouching
    /// pose). On a client the local player answers `LocalPlayer.isCrouching`, its own crouching
    /// flag computed before its input each tick, which can differ from its pose for a tick (and
    /// stays false while flying); a kept entity (a tracked player or mob) answers from the pose
    /// the server last sent. None for a stale handle or an entity whose body the client doesn't
    /// keep.
    fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// Whether the entity sprints (`Entity.isSprinting`): a player, or a
    /// cat or ocelot dashing after its target (their shared sprinting flag);
    /// false for the other kinds. On a client, the local player's own predicted state, or a kept
    /// entity's (a tracked player or mob) last client update. None for a stale handle or an entity
    /// whose body the client doesn't keep.
    fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The player's swimming flag (`Entity.isSwimming`), not the swimming pose
    /// (which also covers crawling). On a client, the local player's own state,
    /// not an authoritative server answer. None for other players (a client keeps them as
    /// tracked bodies without the flag), other kinds and stale handles.
    fn entity_is_swimming(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// An armor stand's pose rotations (`ArmorStand.getHeadPose` ... `getRightLegPose`). On a
    /// client, the synced `DATA_*_POSE` the server last sent, so another player's or a
    /// command's edit shows as soon as it arrives. None for an entity that isn't an armor stand
    /// and for a stale handle.
    fn entity_armor_stand_pose(&self, entity: EntityRef) -> Option<ArmorStandPose>;
    /// Side: both.
    /// An armor stand's client flags (`isSmall`, `showArms`, `showBasePlate`, `isMarker`). On a
    /// client, the synced `DATA_CLIENT_FLAGS` the server last sent. None for an entity that isn't
    /// an armor stand and for a stale handle.
    fn entity_armor_stand_flags(&self, entity: EntityRef) -> Option<ArmorStandFlags>;
    /// Side: both.
    /// The entity's pose (`Entity.getPose`) as `Pose` names it: `standing`,
    /// `crouching`, `swimming`, `sleeping`, a warden's `roaring`... Kinds
    /// with no pose stand. On a client, the local player's own predicted pose, or the synched
    /// `DATA_POSE` the server last sent for a kept entity (a tracked player or mob). None for a
    /// stale handle or an entity whose body the client doesn't keep.
    fn entity_pose(&self, entity: EntityRef) -> Option<&'static str>;
    /// Side: both.
    /// The arm the entity's main hand is on (`LivingEntity.getMainArm`): a player's or a
    /// mannequin's main hand (`Avatar.DATA_PLAYER_MAIN_HAND`; on a server, the Main Hand the
    /// player's client reported), a mob's left-handed flag, the right arm for an armor stand.
    /// On a client, every entity it tracks, as the server last sent it, and the local player's
    /// own Main Hand option. Sunrise keeps no left-handed flag for the ender dragon, which reads
    /// right. None for an entity that isn't living and for a stale handle.
    fn entity_main_arm(&self, entity: EntityRef) -> Option<MainHand>;
    /// Side: both.
    /// Whether the entity sits, by its own vanilla test: `Panda.isSitting`,
    /// `Camel.isCamelSitting` (a camel husk too), `Fox.isSitting` and
    /// `TamableAnimal.isInSittingPose` (wolf, cat, parrot, nautilus, zombie nautilus). A mod
    /// entity answers by its template's test. On a client, the synced flags the server last sent
    /// for a kept mob. None for a kind with no sitting test (players included), a stale handle, or
    /// an entity whose body the client doesn't keep.
    fn entity_is_sitting(&self, entity: EntityRef) -> Option<bool>;
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
