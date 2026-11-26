//! The `player_world` area: where a player is, where it respawns, and moving it between places. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_world` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerWorldBackend {
    /// Side: both.
    /// The support block retained from the player's last move (`Entity.mainSupportingBlockPos`),
    /// not a new collision query or a read of the current ground flag. `Ok(None)` if no support
    /// was retained, the handle is stale, or this side does not keep the player's motion.
    /// On the client, reads the local player's prediction.
    /// `Err(LevelReadError::Unavailable)` if the host cannot read player motion.

    fn player_supporting_block(
        &self,
        player: PlayerRef,
    ) -> Result<Option<BlockPos>, LevelReadError>;
    /// Side: both.
    /// `Entity.getOnPos()` for this player: the support retained from its last move, otherwise
    /// the block at its feet with vanilla's `1.0E-5F` subtracted from Y. A server guest's feet
    /// come from the authoritative position even before the first motion is stored.
    /// `Ok(None)` for a stale handle or a player with no readable position on this side.
    /// `Err(LevelReadError::Unavailable)` if the host cannot read player motion.

    fn player_on_block(&self, player: PlayerRef) -> Result<Option<BlockPos>, LevelReadError>;
    /// Side: both.
    /// The level the online player is in (`ServerPlayer.level`): on an integrated
    /// server or dedicated server, the player's current dimension; on a client,
    /// the client's level. None for a stale or unavailable player.
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef>;
    /// Side: both.
    /// The dimension the online player is in (`ServerPlayer.level().dimension()`):
    /// `minecraft:overworld`, `minecraft:the_nether`, ... None for a stale
    /// or unavailable player.
    fn player_dimension(&self, player: PlayerRef) -> Option<Identifier>;
    /// Side: both.
    /// The collision flags the player's last move left (`Entity.horizontalCollision`,
    /// `minorHorizontalCollision`, `verticalCollision`, `verticalCollisionBelow`).
    /// On the client, reads the local player's prediction. `Ok(None)` for a stale
    /// or untracked handle and for a connected server guest: the server does not
    /// yet retain collision flags from its position reports.
    /// `Err(LevelReadError::Unavailable)` if the host cannot read player motion.

    fn player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> Result<Option<CollisionFlags>, LevelReadError>;
    /// Side: both.
    /// The dimension and block of the player's last death
    /// (`Player.getLastDeathLocation`).
    fn player_last_death_location(&self, player: PlayerRef) -> Option<GlobalPos>;
    /// Side: server.
    /// `ServerPlayer.getRespawnConfig`: where the player comes back after dying: a bed, a
    /// respawn anchor or a `/spawnpoint`, with the yaw they face and whether it is forced.
    /// `Some(None)` when they have none and respawn at the world spawn. None for a stale or
    /// unavailable player.
    fn player_respawn_config(&self, player: PlayerRef) -> Option<Option<PlayerRespawnConfig>>;
}
/// The handles' writes of the `player_world` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerWorldBackendMut {
    /// Side: server.
    /// `ServerPlayer.setRespawnPosition`, as `/spawnpoint` sets it: the player's respawn point
    /// becomes `config`, or is forgotten for None (they then respawn at the world spawn). The
    /// yaw wraps to -180..180 (`RespawnData.of`) and the `player_set_spawn` hook may cancel the
    /// change, which refuses it. Kept with the player's record. False for a stale or unavailable
    /// player, a dead one, a dimension the game doesn't have, a yaw that is not finite and a
    /// canceled change. During `player_respawn_position` only, the removed original may set
    /// this point even after death, so it can be copied to the replacement for future respawns.
    fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool;
    /// Side: server.
    /// `Player.setLastDeathLocation`: stores where the player died, or
    /// forgets it for None. The client hears of it with the player's next
    /// respawn, as in vanilla. A resident dead player can still be edited.
    /// False if the server player's state is unavailable.
    fn player_set_last_death_location(
        &mut self,
        player: PlayerRef,
        location: Option<GlobalPos>,
    ) -> bool;
    /// Side: server.
    /// The calling mod's chunk areas for this player, in place of the ones it asked before; an
    /// empty slice withdraws them, and other mods' areas stay. The engine keeps each area's
    /// chunks loaded with a ticket of its own (`Loaded`, or `Ticking` as a player's simulation
    /// ticket), generates what is missing, and sends a level besides the player's own to a client
    /// that can view it. In the player's own level, a Sunrise client of this build is sent an
    /// area's chunks past its view too, and they count as tracked (their block changes, entities
    /// and passages go with them, and a passage may lead into them); to vanilla's, nothing past
    /// its view is sent. A chunk no area covers any more is forgotten 52 ticks later. An area
    /// counts as no player for spawning, sleeping, raids or any other rule. Areas are never saved
    /// and end when the player leaves. A call that breaks a limit changes nothing and names it.
    fn player_set_chunk_areas(
        &mut self,
        player: PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal>;
    /// Side: both.
    /// Simulate one client movement tick for this server player without changing the player.
    /// Refuses stale, absent, dead, spectator or riding players and invalid input.
    fn player_simulate_move(
        &mut self,
        player: PlayerRef,
        input: PlayerInput,
    ) -> Option<SimulatedMove>;
    /// Side: server.
    /// Respawns a dead player as their client's Respawn button does
    /// (`ServerGamePacketListenerImpl.handleClientCommand`'s `PERFORM_RESPAWN`, then
    /// `PlayerList.respawn`): at their respawn point, else the world spawn, with
    /// `player_respawned` heard. Call outside any player's hook. A request takes effect only on root transaction commit and may
    /// wait for the destination chunks before the player is alive again. Each refusal is named by
    /// `RespawnRefusal`, checked in this order: `Client`, `InPlayerHook` (a player's state is
    /// swapped in; call it from `server_tick`), `Stale` (no player on this server behind the
    /// handle), `NoConnection` (a fake player, or the world's own player in a single-player
    /// world) and `Alive` (health above 0 and the game not won: the Respawn button
    /// ignores exactly that player; one who has won the game and is watching the End poem is
    /// respawned even alive, as the button does).
    fn player_respawn(&mut self, player: PlayerRef) -> Result<(), RespawnRefusal>;
    /// Side: server.
    /// Where the player would respawn now, without respawning them, as vanilla's own
    /// `EndPortalBlock` asks it (`ServerPlayer.findRespawnPositionAndUseSpawnBlock(false,
    /// DO_NOTHING)`): at their respawn point's bed or charged anchor, standing beside it and
    /// facing it (`ServerPlayer.findRespawnAndUseSpawnBlock`, with a block's
    /// `respawn_position` answer, a forced point where it is free); else, when the point no
    /// longer gives a position, at the world spawn with `missing_respawn_block`
    /// (`TeleportTransition.missingRespawnBlock`); else, with no point or one in a level the
    /// world lacks, at the world spawn (`TeleportTransition.createDefault`): the free column
    /// the spawn search finds around it (`PlayerSpawnFinder`), facing the world spawn's angle.
    /// It has no side effect on the world: an anchor keeps its charge, nothing is told
    /// (`player_respawn_position` is the respawn's), and the player stays where they are. Like
    /// vanilla's read, it loads the respawn point's chunk to FULL, blocking the tick, and the
    /// spawn search asks for the chunk it waits for (`TicketType.SPAWN_SEARCH`). It may put
    /// another level in place for the read, and puts the caller's back before it returns. The
    /// refusals are named by `RespawnTargetUnavailable`, checked in this order: `Stale` (no
    /// player on this server behind the handle), `InPlayerHook` and `Loading`.
    fn player_respawn_target(
        &mut self,
        player: PlayerRef,
    ) -> Result<RespawnTarget, RespawnTargetUnavailable>;
}

/// The handles' client writes of the `player_world` area, reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `ClientBackendMut`; mods call the handle
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerWorldClientBackendMut {
    /// Side: client.
    /// Sets the local player's current body yaw in degrees (`LivingEntity.setYBodyRot`).
    /// The view, head, previous body angle and packets are unchanged. `Ok(false)` for a stale,
    /// nonlocal, or unavailable player. Returns `Err(LevelWriteError::Unavailable)` when the
    /// host cannot write living state. Does not change frame extraction order.
    fn player_set_body_yaw(&mut self, player: PlayerRef, yaw: f32)
    -> Result<bool, LevelWriteError>;
}
