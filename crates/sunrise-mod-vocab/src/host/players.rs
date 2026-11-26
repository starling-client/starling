//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `players` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersHost {
    /// Side: both.
    /// `ItemCooldowns.getCooldownGroup(stack)`: the group `stack`'s
    /// cooldown is filed under, the group its `use_cooldown` component names
    /// (its own, else its item's default), else its item id.
    fn item_cooldown_group(&self, stack: &ItemStack) -> String;
    /// Side: both.
    /// Resolve a real online player by UUID, never a fake. A fake is reached
    /// through the handle returned by its factory. During a nested player swap,
    /// outer players whose records are displaced return None.
    fn player(&self, uuid: u128) -> Option<PlayerRef>;
}
/// The server host's methods of the `players` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersServerHost {
    /// Side: server.
    /// Create or retrieve a connectionless player for the level currently in place.
    fn fake_player(&mut self, level: LevelRef, uuid: u128, name: &str) -> Option<PlayerRef>;
    /// Side: server.
    /// The most players who may be on at once (`PlayerList.getMaxPlayers`,
    /// `max-players`), as the server list shows it. Logins at or above the limit
    /// are refused unless they bypass it. None where no server is open to players
    /// (a client, or a server nobody can join).
    fn server_max_players(&self) -> Option<u32>;
    /// Side: server.
    /// Sets the player limit from the next login and server list ping on
    /// (Paper's `Server.setMaxPlayers`); players already on stay, even when
    /// they are more than the new limit. The existing login bypass still applies
    /// (dedicated operators flagged `bypassesPlayerLimit`). `server.properties`
    /// is not changed, so a restart goes back to its `max-players`.
    /// Writes nothing and refuses with `PlayerLimitRefusal::NotOpen` where no server is open to players,
    /// and with `TooHigh { maximum }` for a limit above `i32::MAX`, as vanilla
    /// keeps the limit in an `int` (Paper's `setMaxPlayers` takes one). Zero is
    /// taken: it refuses all new logins that do not bypass the limit.
    fn server_set_max_players(&mut self, max_players: u32) -> Result<(), PlayerLimitRefusal>;
}
/// The client host's methods of the `players` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientHost {
    /// Side: client.
    /// For a client mod: sets the local player's fall-flying flag (`LivingEntity.startFallFlying`)
    /// from `player_tick_pre`, so the tick's own glide detection sends `START_FALL_FLYING` as a
    /// jump-started glide does. This is the raw flag write, not `tryToStartFallFlying`: it does
    /// not check the glider, the ground, levitation or a liquid. `false`, and nothing changes,
    /// with no player of its own.
    fn start_local_player_gliding(&mut self) -> bool;
    /// Side: client.
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone.
    fn push_player(&mut self, kick: [f32; 3]);
    /// Side: client.
    /// For a client mod: chooses the local player's hotbar slot, `slot` (0 to 8), as
    /// `Inventory.setSelectedSlot` does for a number key. It sends nothing itself: the game
    /// sends the carried-item packet at the next tick's start at the latest, and before the
    /// packets of an attack, a use, an interact, a release, a drop and a continued block break
    /// (not before a block break start). Refused above 8 with `OutOfRange`, without a client
    /// world with `NoWorld`, and for a spectator with `Refused` (its number keys open the
    /// spectator menu), leaving the selection as it was.
    fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError>;
    /// Side: client.
    /// For a client mod: the local player's collision exclusions, the client half of
    /// `EntityRef::set_collision_exclusions`, with the same boxes, rules and refusals (a client
    /// with no player of its own answers `Stale`). The client moves its player itself, so a mod
    /// sets the same boxes on both sides: the engine sends neither set to the other.
    fn set_collision_exclusions(
        &mut self,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError>;
    /// Side: client.
    /// For a client mod: sets the local player's velocity (`Entity.setDeltaMovement`), in blocks
    /// per tick. Write it from `player_tick_pre`: the tick's own move then integrates it, and
    /// the position report the move produces is checked by the server as any report is, so a
    /// velocity the server's speed test or collision check refuses is corrected there. The write
    /// replaces the velocity outright, as the client's handling of a motion packet does, and
    /// nothing is sent. `false`, and nothing changes, with no player of its own and for a value
    /// that is not finite.
    fn set_local_player_velocity(&mut self, velocity: [f64; 3]) -> bool;
    /// Side: client.
    /// For a client mod: sets the local player's on-ground flag. Write it from `player_tick_pre`:
    /// this tick's jump decision reads it, and the move that follows sets it again from the
    /// collision, so the report to the server carries the flag the move left. `false`, and
    /// nothing changes, with no player of its own.
    fn set_local_player_on_ground(&mut self, on_ground: bool) -> bool;
    /// Side: client.
    /// Moves the local player by `delta` in blocks, as `Entity.move(MoverType.SELF, delta)`.
    /// The move uses the player's usual collisions and step height, and updates its ground
    /// and fall state, including the usual effects of collisions and blocks on velocity.
    /// Available to client hooks with world access. For per-tick movement call from the local
    /// player's client-side `player_tick_pre`, before its own movement. This sends no packet
    /// and fires no `entity_move` hook (that hook is server-only). A section change is
    /// reported through `entity_entering_section` at the position write, before the move
    /// updates collision flags, ground, fall distance and velocity.
    /// The normal position report carries the movement and ground flag, and the server checks
    /// the report as usual.
    /// Returns the displacement actually made, or `Ok(None)` without moving if there is no
    /// local player, the player is dead, sleeping or riding, or a component is non-finite or
    /// outside ±64 blocks. `Err(LevelWriteError::Unavailable)` means the call is outside a
    /// world phase or the engine couldn't finish the move.
    fn move_local_player_by(
        &mut self,
        delta: [f64; 3],
    ) -> Result<Option<[f64; 3]>, LevelWriteError>;
    /// Moves the window's player through `passage` (#3775), one its server told it of, as
    /// Immersive Portals' client does: its feet, where it was a tick ago (so the frames
    /// between two ticks do not jump), its velocity and its look come out at the far side,
    /// turned with the passage, and the server is told the passage and where the feet were.
    /// Nothing else of the player changes: on ground, fall distance, sprinting, what it
    /// uses. The server checks the move to there as any move, then answers with a position
    /// packet that changes nothing (all relative and zero), so the player keeps its velocity;
    /// as any position packet does, it stops mining (a passage crossed while mining aborts
    /// the dig). A refused crossing puts the player back where the server has it
    /// ([`Self::last_passage_refusal`] says why). One crossing at a time: the next waits
    /// for the server's answer. Refused, with nothing moved or sent, when not connected to
    /// a server, with no player, while riding, for a passage the window does not hold, when
    /// the feet are more than 20 blocks from the opening, and when a mod's
    /// `client_packet_send` cancels the crossing's payload.
    fn cross_passage(&mut self, passage: PassageId) -> Result<(), PassageCrossError>;
    /// Side: client.
    /// The server's refusal of the window's last crossing ([`Self::cross_passage`]) and the
    /// passage it named, until the next crossing; None when it was not refused or no answer
    /// came yet.
    fn last_passage_refusal(&self) -> Option<(PassageId, PassageCrossRefusal)>;
}
/// The handles' client reads of the `players` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientBackend {
    /// Side: client.
    /// The local player's last authoritative custom-book pair. None before
    /// negotiation, while resynchronizing, or for a nonlocal/stale handle.
    fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings>;
    /// Side: client.
    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    fn player_block_breaking(&self, player: PlayerRef) -> Option<BlockBreakingState>;
    /// Side: client.
    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32>;
}
/// Client-only player writes, reached through player handles.
// hidden: mods call PlayerRef methods, never the generated backend trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientBackendMut {
    /// Side: client.
    /// Clears the local player's accumulated fall distance (`Entity.resetFallDistance`).
    /// This is client prediction only and sends no packet. Returns `Ok(false)` for a
    /// stale or non-local player or if no local player is in the world, and
    /// `Err(LevelWriteError::Unavailable)` without writable client-world access,
    /// such as in pack discovery or while a section crossing borrows the world.

    fn player_reset_fall_distance(&mut self, player: PlayerRef) -> Result<bool, LevelWriteError>;
    /// Side: client.
    /// Request a new pair for the local player's negotiated book. True means
    /// queued, not accepted; only an authoritative reply changes saved state.
    fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
    /// Side: client.
    /// Creative mode only: puts `stack`, components and all, into the local player's
    /// inventory as `Inventory.add` does: onto matching stacks (the selected slot, the off
    /// hand, the hotbar, then the main inventory), then into empty slots, up to the stack's
    /// effective maximum (`minecraft:max_stack_size` included). Every slot it changed goes to
    /// the server at once as `MultiPlayerGameMode.handleCreativeModeItemAdd` does
    /// (`ServerboundSetCreativeModeSlotPacket`; the off hand is slot 45), even while a
    /// container menu is open. `Ok(Some(remainder))` is what did not fit; nothing is dropped.
    /// Refuses outside a client world (`Unavailable`), for a stale or non-local handle
    /// (`Stale`), outside creative mode (`NotCreative`), or for a stack the item boundary
    /// refuses (`Stack`). A [`CreativeGiveError`] gives nothing (including `FeatureDisabled` when
    /// the connected server does not enable the item's required feature flags).
    #[forward_api_inputs]
    #[forward_api_output]
    fn player_give_creative(
        &mut self,
        player: PlayerRef,
        stack: &ItemStack,
    ) -> Result<Option<ItemStack>, CreativeGiveError>;
}
