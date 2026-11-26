//! The `player_modes` area: game mode and abilities, and what they allow. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_modes` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerModesBackend {
    /// Side: server.
    /// `Player.hasInfiniteMaterials`: the `instabuild` ability, which
    /// creative mode gives.
    fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `Player.canUseGameMasterBlocks`: creative and a permission level of
    /// at least 2, as command blocks, structure blocks and jigsaws ask.
    fn player_can_use_game_master_blocks(&self, player: PlayerRef) -> Option<bool>;
    /// Side: server.
    /// `ServerPlayerGameMode.getPreviousGameModeForPlayer`: the mode the
    /// player last left (`previousPlayerGameType`, saved with the player), or
    /// `Some(None)` when they never changed mode. None for a stale or
    /// unavailable player.
    fn player_previous_game_mode(&self, player: PlayerRef) -> Option<Option<GameMode>>;
    /// Side: both.
    /// `Player.isSecondaryUseActive`: whether the player uses items and
    /// blocks as with sneak held, as vanilla reads it from the shift-key
    /// flag (the same read as `EntityRef::is_shift_key_down`). None for a
    /// stale or unavailable player.
    fn player_is_secondary_use_active(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `ServerPlayerGameMode.getGameModeForPlayer` on a server; `AbstractClientPlayer.gameMode`
    /// (the entity's resolved `PlayerInfo`) on a client. Paper `HumanEntity#getGameMode`.
    /// None for stale, offline or unavailable players (including displaced outer players), and on
    /// a client before `ADD_PLAYER` resolves the entity's `PlayerInfo`. Distinct from
    /// `player_abilities`, which reads `LocalPlayer.abilities`.
    fn player_game_mode(&self, player: PlayerRef) -> Option<GameMode>;
    /// Side: both.
    /// `Player.isCreative`: `gameMode() == GameType.CREATIVE` (authoritative on a server;
    /// resolved `PlayerInfo` on a client, false before `ADD_PLAYER`, distinct from
    /// `player_abilities`' `instabuild`). Paper `HumanEntity#getGameMode`.
    fn player_is_creative(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.isSpectator`: `gameMode() == GameType.SPECTATOR` (authoritative on a server;
    /// resolved `PlayerInfo` on a client, false before `ADD_PLAYER`, distinct from
    /// `MultiPlayerGameMode` controller state and `player_abilities`). Paper `HumanEntity#getGameMode`.
    fn player_is_spectator(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.mayBuild`; Paper `HumanEntity#getGameMode`.
    fn player_may_build(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.getAbilities`; Paper `Player#getAllowFlight/isFlying/getFlySpeed/getWalkSpeed`.
    /// On a client, reads `LocalPlayer.abilities` (flying is the local body's current prediction;
    /// server ability/mode packets supply the other flags and speeds), independent of
    /// `PlayerInfo`'s entity mode. None for any other player.
    fn player_abilities(&self, player: PlayerRef) -> Option<PlayerAbilities>;
    /// Side: both.
    /// `IPlayerExtension.mayFly` (NeoForge): whether the player may start flying, by game mode
    /// or ability. Read this rather than `player_abilities`' `may_fly`, which a creative-flight
    /// attribute would not show; Sunrise has no such attribute yet, so today the two agree.
    /// None for a stale or unavailable player.
    fn player_may_fly(&self, player: PlayerRef) -> Option<bool>;
}
/// The handles' writes of the `player_modes` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerModesBackendMut {
    /// Side: server.
    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. The two speed values are stored as supplied, like
    /// `Abilities.setFlyingSpeed` and `setWalkingSpeed`. Returns false for a stale,
    /// unavailable or displaced outer player.
    fn player_set_abilities(&mut self, player: PlayerRef, abilities: PlayerAbilities) -> bool;
    /// Side: server.
    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, an active
    /// outer guest, no change, or a canceled change.
    fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool;
}
