//! The `player_fakes` area: connectionless fake players and the actions they perform. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_fakes` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerFakesBackend {
    /// Side: server.
    /// Whether a live player handle names a connectionless actor.
    fn player_is_fake(&self, player: PlayerRef) -> Option<bool>;
}
/// The handles' writes of the `player_fakes` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerFakesBackendMut {
    /// Side: server.
    /// A fake player acts as a player does, through the game's own path for the action
    /// (`ServerGamePacketListenerImpl.handleUseItemOn` / `handleUseItem` / `handleInteract`'s
    /// ATTACK, and `ServerPlayerGameMode.destroyBlock`), as its packet would run it: reach,
    /// protection and the border, then the action. Once it runs, an effect a fake can't take
    /// part in (a menu, sleep, a ride, another dimension) is stopped where it would start, and
    /// the answer says which; the action's own side effects (the swing, the hooks before it,
    /// vanilla's result) have still happened then. The fake is found once, and its turn
    /// starts at that place in the list. The acting fake's body can obstruct its
    /// own block placement, just as a connected player's can.
    fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused>;
    /// Side: server.
    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    /// For placement, choose an eye position that keeps the temporary standing body
    /// clear of the block's collision shape.
    fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused>;
    /// Side: server.
    /// Remove a fake without treating it as a disconnect or saving it.
    fn player_remove_fake(&mut self, player: PlayerRef) -> bool;
}
