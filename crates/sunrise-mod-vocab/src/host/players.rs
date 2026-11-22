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
}
<<<<<<< sync-sunrise-1
/// The handles' writes of the `players` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayersBackendMut {
    /// Side: server.
    /// Set a registered custom book's authoritative pair and synchronize an
    /// accepted change. Returns false for an unavailable player, unknown key
    /// or exhausted storage. An unchanged pair succeeds without notification.
    #[access(WorldOnly)]
    fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
    /// Side: server.
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_tell(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Disconnects that player with `reason`, as `/kick` does
    /// (`ServerGamePacketListenerImpl.disconnect`): their client shows it, the log's "lost
    /// connection" line names it, `player_kicked` hears it and a dedicated server records a
    /// `kick` event. The connection closes when the server next polls it, so the calling hook
    /// finishes first; works for the acting player inside their own hook, and without the
    /// world published. False for a stale or unavailable player, a fake player, the
    /// single-player owner (`/kick` refuses the owner) and a player whose disconnect is already
    /// pending (a second call, also from `player_kicked`: the first reason stands).
    /// A second disconnect on a guest already leaving is a no-op and keeps the first reason.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn player_disconnect(&mut self, player: PlayerRef, reason: Text)
    -> Result<bool, TextFormError>;
    /// Side: server.
    /// Whether that player counts for `players_sleeping_percentage`: an
    /// ignored player who isn't in bed is left out of the players the night
    /// needs asleep, as a spectator is (an away-from-keyboard mod). Kept
    /// until a mod sets it back or the server stops, also if they leave and
    /// come back. False when the server list is unavailable, the handle is
    /// stale or the player isn't online.
    #[access(WorldOnly)]
    fn player_set_sleep_ignored(&mut self, player: PlayerRef, ignored: bool) -> bool;
    /// Side: server.
    /// `ServerPlayer.setRespawnPosition`, as `/spawnpoint` sets it: the player's respawn point
    /// becomes `config`, or is forgotten for None (they then respawn at the world spawn). The
    /// yaw wraps to -180..180 (`RespawnData.of`) and the `player_set_spawn` hook may cancel the
    /// change, which refuses it. Kept with the player's record. False for a stale or unavailable
    /// player, a dead one, a dimension the game doesn't have, a yaw that is not finite and a
    /// canceled change.
    #[access(WorldOnly)]
    fn player_set_respawn_config(
        &mut self,
        player: PlayerRef,
        config: Option<PlayerRespawnConfig>,
    ) -> bool;
    /// Side: server.
    /// Sets the player's `sleepCounter`: 0 to 109, the values the game reaches (see
    /// `player_sleep_timer`). A player in bed at 100 counts as slept long enough for the night
    /// skip. False for a value past 109, a stale or unavailable player, a fake player and a
    /// player in another level than the one in place.
    #[access(WorldOnly)]
    fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Shows `nickname` in place of that player's name in chat (their
    /// messages, `/me`, `/msg`) and in the Tab list; None shows their name
    /// again. Hovering the shown name gives the real one, which stays in the
    /// log, commands, selectors and the server's events. Kept until a mod
    /// sets it again or the server stops (also if they leave and come back).
    /// Plain text: the mod checks it (`Messages::text_allowed`, its length,
    /// other players' names). False when the server list is unavailable,
    /// the handle is stale or the player isn't online.
    #[access(WorldOnly)]
    fn player_set_nickname(&mut self, player: PlayerRef, nickname: Option<&str>) -> bool;
    /// Side: server.
    /// Hides that player (a vanish) from the players below permission
    /// `level`, or shows them again (None): from the next tick those players
    /// lose them from the Tab list and the world (their client stops
    /// tracking the player, as it does a spectator), and don't read their
    /// joining or leaving; players of `level` or more still see them, and
    /// a player made an operator meanwhile sees them again. Kept by UUID, even
    /// after this handle disconnects (the player rejoins hidden), until a mod sets it again
    /// or the server stops. Works inside a player's hook too; false for a stale
    /// world epoch, on a client and in a world nobody else can join.
    #[access(WorldOnly)]
    fn player_hide(&mut self, player: PlayerRef, level: Option<u8>) -> bool;
    /// Side: server.
    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    #[access(WorldOnly)]
    fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Opens content mod menu type `menu` for a connected player from anywhere, with no
    /// block (`ServerPlayer.openMenu(provider, extraDataWriter)`): an item's use, an entity,
    /// a command. The menu replaces the one they had open. Its container slots show empty
    /// and their clicks go to the mods (`native_menu_clicked`), as `open_menu`'s do. `extra`
    /// (at most 32 KiB) is what the client's `screen_extra_data` reads, and `pos`, when
    /// given, its `screen_pos`. The old menu closes on the server as a close does (its
    /// `removed`, `player_container_close`, the cursor stack and crafting grids back to the
    /// inventory) before the new one is built; `replace` says whether the client is first sent
    /// a close for it, as for `open_menu`. `MenuReplace::InPlace` sends none: the client keeps
    /// its screen and mouse until the new screen replaces it. The window id, or Ok(None) when
    /// a mod cancelled the open. Errors: `NotModMenu`, `PlayerUnavailable` (stale, fake or
    /// vanilla client), `TooLarge`, or `Title` for a title that could not cross; a cancelled or
    /// refused open leaves the old menu as it was.
    #[access(WorldOnly)]
    fn player_open_mod_menu(
        &mut self,
        player: PlayerRef,
        menu: MenuKey,
        title: Text,
        extra: Vec<u8>,
        pos: Option<[i32; 3]>,
        replace: MenuReplace,
    ) -> Result<Option<i32>, OpenMenuError>;
    /// Side: server.
    /// Backs the container slots of the mod menu `window` that `player_open_mod_menu` opened
    /// for the player with storage that is not a block entity (#3385; NeoForge's
    /// `StackCopySlot` and `ResourceHandlerSlot`): `places[n]` is what slot `n` shows. From then
    /// on a click or shift-click on those slots, and an `ItemPlace::MenuSlot` write, go through
    /// that storage's own handler, journalled; the client sees ordinary synced slots, mirrored
    /// after each click and each write through an `ItemPlace` of that storage, never each tick.
    /// A slot past `places` stays the mods' own (`native_menu_clicked`). The menu closes once a
    /// place stops resolving (the backpack left the hand, the entity is gone) or a held stack is
    /// no longer the one bound when the slots were backed, checked before each click and at
    /// each change notice. A `MenuSlotPlace::HeldItem` locks the inventory slot holding that
    /// stack while the menu is open, so it can't go into itself. Backing again replaces the
    /// places; a slot no longer backed (an empty list, a shorter one) is emptied, never left
    /// holding the copy it showed. A [`MenuSlotBackRefusal`], and nothing changes, for a player
    /// not online or a window that is not their open menu, a menu that is not a blockless mod
    /// menu, more places than its container slots, or a place that doesn't resolve now.
    #[access(WorldOnly)]
    fn player_back_mod_menu_slots(
        &mut self,
        player: PlayerRef,
        window: i32,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal>;
    /// Side: server.
    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    #[access(WorldOnly)]
    fn player_send_map(&mut self, player: PlayerRef, map_id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// Stores a server player's `Abilities` and tells their client
    /// (`ServerPlayer.onUpdateAbilities`). They stay as written until the
    /// player's game mode changes (`GameType.updatePlayerAbilities`), and are
    /// saved with the player. A speed that is not finite refuses the write
    /// (vanilla's setters take any float). Returns false for a
    /// stale or unavailable player, a displaced outer player, and a speed
    /// that is not finite.
    #[access(WorldOnly)]
    fn player_set_abilities(&mut self, player: PlayerRef, abilities: PlayerAbilities) -> bool;
    /// Side: server.
    /// A fake player acts as a player does, through the game's own path for the action
    /// (`ServerGamePacketListenerImpl.handleUseItemOn` / `handleUseItem` / `handleInteract`'s
    /// ATTACK, and `ServerPlayerGameMode.destroyBlock`), as its packet would run it: reach,
    /// protection and the border, then the action. Once it runs, an effect a fake can't take
    /// part in (a menu, sleep, a ride, another dimension) is stopped where it would start, and
    /// the answer says which; the action's own side effects (the swing, the hooks before it,
    /// vanilla's result) have still happened then. The fake is found once, and its turn
    /// starts at that place in the list.
    #[access(WorldOnly)]
    fn player_perform_action(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
    ) -> Result<ActionOutcome, ActionRefused>;
    /// Side: server.
    /// A machine's one use on the persistent actor, with its private state put back even
    /// when the action refuses. Only the existing guest turn runs gameplay and its hooks.
    #[access(WorldOnly)]
    fn player_perform_action_scoped(
        &mut self,
        player: PlayerRef,
        action: PlayerAction,
        setup: PlayerActionSetup,
    ) -> Result<ScopedActionResult, ActionRefused>;
    /// Side: server.
    /// `ItemCooldowns.addCooldown(group, ticks)`: starts (or restarts) the
    /// group's cooldown, and the player's client draws it on every item of the
    /// group (`ServerItemCooldowns.onCooldownStarted`).
    #[access(WorldOnly)]
    fn player_set_item_cooldown(&mut self, player: PlayerRef, group: &str, ticks: i32) -> bool;
    /// Side: server.
    /// `ItemCooldowns.removeCooldown(group)`: ends the group's cooldown now,
    /// on the client too (`ServerItemCooldowns.onCooldownEnded`).
    #[access(WorldOnly)]
    fn player_remove_item_cooldown(&mut self, player: PlayerRef, group: &str) -> bool;
    /// Side: server.
    /// Sets `Player.takeXpDelay`: no orb is taken for `ticks` ticks. The
    /// change is not reported to `player_exp_cooldown_change`.
    #[access(WorldOnly)]
    fn player_set_exp_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Sets the player's `WardenSpawnTracker.cooldownTicks`: sculk shriekers
    /// they set off don't warn for `ticks` ticks. The level's players share
    /// one tracker here, the integrated server owner's (the shriekers read it
    /// for whoever sets them off), so this is every player's.
    /// False, changing nothing, for anyone but the tracker's owner until each player has their own.
    #[access(WorldOnly)]
    fn player_set_warden_warning_cooldown(&mut self, player: PlayerRef, ticks: u32) -> bool;
    /// Side: server.
    /// Fires the mod's criteria trigger `trigger` (`mymod:pipe_connected`)
    /// for `player`: vanilla's
    /// `SimpleCriterionTrigger.trigger`. Each criterion of theirs with that
    /// trigger, not yet granted, of an advancement not yet done, is tested
    /// by `test` and then by its `player` predicate; those that pass are
    /// granted, and an advancement they complete is awarded with its toast,
    /// chat line and rewards, as the game's own triggers award.
    ///
    /// The acting player is reached inside their hook; another live player
    /// is reached in their own level, including during a guest's turn. An
    /// outer player displaced by a nested turn is unavailable. `Ok(())`
    /// means the player was reached, whether or not anything was granted.
    /// A stale/offline/displaced handle, a remote client, or a `minecraft:`
    /// trigger or empty id is refused with a typed reason.
    ///
    /// `test` is lent to this call only. A failed test stops further testing
    /// and reports `Test`; earlier matching criteria may already be granted.
    /// Wasm predicates run synchronously, including callbacks into the group
    /// firing the trigger, within that call's remaining execution and memory limits.
    #[access(WorldOnly)]
    fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: Arc<dyn CriterionTest>,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `default_block_use`, without a tool.
    /// The engine fires `any_block_use` once when the enclosing block use consumes.
    /// It sees the hand after the use; a replacement stack is visible, unlike vanilla's original object.
    /// Evaluates vanilla predicates and rewards in the player's own level; never reads inventory.
    /// `Ok(())` means reached, even if no criterion matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    #[access(WorldOnly)]
    fn player_trigger_block_use(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `item_used_on_block` with `original`.
    /// `original` is copied after a before-block PASS and before block/item effects.
    /// The stack is not read from inventory. The engine fires `any_block_use`
    /// once when the enclosing block use consumes, including a consuming cancel.
    /// Call this helper only when the mod also intends the inner criterion.
    /// An invalid stack returns `Stack(StackWriteError)` before the trigger fires.
    /// Evaluates vanilla predicates and rewards in the player's own level.
    /// `Ok(())` means reached, even if nothing matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    #[access(WorldOnly)]
    fn player_trigger_item_used_on_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        original: &ItemStack,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server.
    /// `Player.setLastDeathLocation`: stores where the player died, or
    /// forgets it for None. The client hears of it with the player's next
    /// respawn, as in vanilla. False in the cases a read gives None.
    #[access(WorldOnly)]
    fn player_set_last_death_location(
        &mut self,
        player: PlayerRef,
        location: Option<GlobalPos>,
    ) -> bool;
    /// Side: server.
    /// `FoodData.eat(nutrition, saturationModifier)`: adds `nutrition` to
    /// the food level (0 to 20) and `nutrition * modifier * 2` to the
    /// saturation, which stays at most the food level.
    #[access(WorldOnly)]
    fn player_feed(&mut self, player: PlayerRef, nutrition: i32, saturation_modifier: f32) -> bool;
    /// Side: server.
    /// `Player.causeFoodExhaustion`: adds exhaustion (at most 40), as
    /// sprinting or jumping does. An invulnerable player (creative) or a
    /// spectator gets none, and the call still returns true.
    #[access(WorldOnly)]
    fn player_add_exhaustion(&mut self, player: PlayerRef, amount: f32) -> bool;
    /// Side: server.
    /// `FoodData.setFoodLevel`: sets the food level as it is, unclamped,
    /// as vanilla does.
    #[access(WorldOnly)]
    fn player_set_food_level(&mut self, player: PlayerRef, level: i32) -> bool;
    /// Side: server.
    /// `FoodData.setSaturation`: sets the saturation as it is, unclamped.
    #[access(WorldOnly)]
    fn player_set_saturation(&mut self, player: PlayerRef, saturation: f32) -> bool;
    /// Side: server.
    /// Sets the exhaustion as it is (Paper's `HumanEntity#setExhaustion`;
    /// vanilla's `FoodData` only adds to it).
    #[access(WorldOnly)]
    fn player_set_exhaustion(&mut self, player: PlayerRef, exhaustion: f32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperiencePoints`: sets the progress toward the next
    /// level to `points` of the points it needs, kept below a full bar. The
    /// level and the total are kept.
    #[access(WorldOnly)]
    fn player_set_experience_points(&mut self, player: PlayerRef, points: i32) -> bool;
    /// Side: server.
    /// `ServerPlayer.setExperienceLevels`: sets the level, keeping the
    /// progress and the total.
    #[access(WorldOnly)]
    fn player_set_experience_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// Changes a player's game mode as `ServerPlayer.setGameMode` and
    /// `/gamemode` do, including the cancellable change hook and ability
    /// updates. Returns false for a stale or unavailable player, an active
    /// outer guest, no change, or a canceled change.
    #[access(WorldOnly)]
    fn player_set_game_mode(&mut self, player: PlayerRef, mode: GameMode) -> bool;
    /// Side: server.
    /// `Inventory.setSelectedSlot`: makes `slot` (0 to 8) the hotbar slot the player holds and
    /// tells their client (`ClientboundSetHeldSlot`), as `/clear`-style server code does. A new
    /// slot ends a main-hand use in progress, as the client's own switch does, and asks no
    /// `player_switch_hotbar_slot_pre`: that hook is for the player's own input. Returns false for
    /// a slot past 8, a stale or unavailable player, and a dead one; true when the player already
    /// held `slot`.
    #[access(WorldOnly)]
    fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool;
    /// Side: server.
    /// `Player.drop(stack, false, thrown_by_player)`: `stack` is thrown from the player's eyes
    /// along their view as an item entity, with the main hand swinging and `item_toss` asked
    /// first. Nothing is taken from an inventory: the stack is the caller's, taken out through
    /// the item places first, which is what separates it from giving or dropping into the
    /// inventory. `thrown_by_player` names the player as the item's thrower, as the drop key
    /// does. `Ok(None)` when the stack is in the world; otherwise `Ok(Some(stack))` comes back, untouched, for
    /// the caller to put back: an empty stack, a stale or dead player, or a toss a mod canceled
    /// (`item_toss`), so a refused drop never loses items. Invalid API stack conversion returns
    /// `StackWriteError`, as other checked stack writes do.
    #[access(WorldOnly)]
    fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Side: server.
    /// Remove a fake without treating it as a disconnect or saving it.
    #[access(WorldOnly)]
    fn player_remove_fake(&mut self, player: PlayerRef) -> bool;
    /// Side: server.
    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may rewrite or cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    #[access(WorldOnly)]
    fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
        amount: i32,
    ) -> bool;
    /// Side: server.
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    #[access(WorldOnly)]
    fn player_reset_stat(&mut self, player: PlayerRef, stat_type: StatType, key: &str) -> bool;
    /// Side: server.
    /// `ServerPlayer.awardRecipes`, as `/recipe give` does: the recipes
    /// unlock in the player's book, with their toasts and the
    /// `player_recipe_discovered` hook, and the client hears of them.
    /// Unknown ids are skipped. The number of recipe displays added, None
    /// for a player it can't reach.
    #[access(WorldOnly)]
    fn player_award_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
    /// Side: server.
    /// `ServerPlayer.resetRecipes`, as `/recipe take` does. The number of
    /// recipe displays taken out, None for a player it can't reach.
    #[access(WorldOnly)]
    fn player_reset_recipes(&mut self, player: PlayerRef, recipes: &[&str]) -> Option<i32>;
    /// Side: server.
    /// `ExperienceOrb.playerTouch`: Mending repairs the player's damaged
    /// items with `points` first (`ExperienceOrb.repairPlayerItems`), then
    /// only the rest goes to their experience
    /// (`Player.giveExperiencePoints`), with the level-up sound. No orb
    /// pickup sound. False, and nothing given, as the other writes; `points`
    /// above `i32::MAX` is refused before Mending repairs anything.
    #[access(WorldOnly)]
    fn player_give_experience_points(&mut self, player: PlayerRef, points: u32) -> bool;
    /// Side: server.
    /// `Player.giveExperienceLevels`: adds `levels`, or takes them off when
    /// negative, as the anvil and the enchanting table charge with. Below 0
    /// the level, the total and the progress toward the next level are all 0.
    #[access(WorldOnly)]
    fn player_give_levels(&mut self, player: PlayerRef, levels: i32) -> bool;
    /// Side: server.
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. True means sent.
    #[access(WorldOnly)]
    fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool;
    /// Side: server.
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// level random's `nextLong`, drawn once per call. True means sent, not
    /// heard.
    #[access(WorldOnly)]
    fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// The calling mod's chunk areas for this player, in place of the ones it asked before; an
    /// empty slice withdraws them, and other mods' areas stay. The engine keeps each area's
    /// chunks loaded with a ticket of its own (`Loaded`, or `Ticking` as a player's simulation
    /// ticket), generates what is missing, and sends a level besides the player's own to a client
    /// that can view it. A chunk no area covers any more is forgotten 52 ticks later. An area
    /// counts as no player for spawning, sleeping, raids or any other rule. Areas are never saved
    /// and end when the player leaves. A call that breaks a limit changes nothing and names it.
    #[access(WorldOnly)]
    fn player_set_chunk_areas(
        &mut self,
        player: PlayerRef,
        areas: &[ChunkArea],
    ) -> Result<(), ChunkAreaRefusal>;
    /// Side: both.
    /// Simulate one client movement tick for this server player without changing the player. Refuses stale, absent, dead, spectator or riding players and invalid input.
    #[access(WorldOnly)]
    fn player_simulate_move(
        &mut self,
        player: PlayerRef,
        input: PlayerInput,
    ) -> Option<SimulatedMove>;
    /// Side: server.
    /// Numbers: 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand,
    /// 41 body, 42 saddle. ItemPlace uses 100..103 for armour and -106 for offhand;
    /// InventoryMenu has no 41 or 42.
    /// `Inventory.setItem(slot, stack)` by `player_inventory_slots`' numbers: `stack` (None
    /// empties it) goes into that slot as `/item replace entity <player> <slot>` puts it,
    /// without the slot's item check, so a worn slot takes any item; the equip sound and
    /// equipment changes follow for 36 to 42. The client is sent the slot with the tick's
    /// inventory sync (`ContainerSetSlot`), as vanilla's `broadcastChanges`. Each refusal
    /// writes nothing and is named by `PlayerDataRefusal`: `Stale` (no online player behind
    /// the handle), `Client`, `Dead`, `SlotOutOfRange` (a slot past 42, never a panic) and
    /// `Stack` (an item the world lacks, a count or damage the item does not allow).
    /// An open transaction journals this write; an abort restores the previous value.
    /// Equipment effects wait for the outermost commit.
    #[access(WorldOnly)]
    fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// `PlayerEnderChestContainer.setItem(slot, stack)`: `stack` (None empties it) in ender
    /// chest slot `slot` (0 to 26). A player looking into their ender chest is sent the slot
    /// with the open menu's sync. Refuses as `player_set_inventory_slot` does, with
    /// `SlotOutOfRange { last: 26 }` for a slot past 26 (never a panic).
    /// An open transaction journals this write; an abort restores the previous value.
    #[access(WorldOnly)]
    fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Gives `stack` as `/give` does, components and all: `Inventory.add` fills matching
    /// stacks, then empty slots (the selected one first); what does not fit is thrown from
    /// the player's eyes with no pickup delay and only they may pick it up; a stack that went
    /// in whole plays the pickup sound. Refuses, with nothing given, with `Stale`, `Client`,
    /// `Dead` or `Stack`, as `player_set_inventory_slot` does.
    /// An open transaction journals this write; an abort restores the previous value.
    /// Overflow drops and pickup sounds wait for the outermost commit.
    #[access(WorldOnly)]
    fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Replaces the compound mods keep on a server player (`player_persistent_data`) with
    /// `data`; an empty compound takes it away. It is saved with the player and never sent to
    /// a client; changing it raises no notification, so a mod that wants to hear of changes
    /// keeps its own. On a death respawn and on a player's first exit from the End (before
    /// the credits) only the `PlayerPersisted` sub-compound is carried to the new player, as
    /// NeoForge does, and every other key is lost; dimension changes, later End exits and
    /// teleports keep the whole compound. For ports: new mods use attachments. Refuses,
    /// writing nothing: `Stale`, `Client`, `Dead`, and `TooDeep` for data that would sit 512
    /// levels deep or more in the player's file.
    /// An open transaction journals this write; an abort restores the previous value.
    #[access(WorldOnly)]
    fn player_set_persistent_data(
        &mut self,
        player: PlayerRef,
        data: Compound,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Breaks the block at `pos` in the player's level as the player breaks it
    /// (`ServerPlayerGameMode.destroyBlock`), as a vein miner breaks each block of a vein. The
    /// player's own rules apply: a spectator, an adventure player whose tool may not break
    /// it, a held item that breaks no blocks (a sword in creative), a game master block
    /// without the permission and a dedicated server's spawn protection keep the block.
    /// `break_block` is asked and may cancel it; then the break runs as the player's own:
    /// the block's effects, events, particles and sound, `player_destroyed_block`, the loot of
    /// the held item and its enchantments, `block_drops` with this player breaking, the
    /// item's wear, the mined statistic and the exhaustion. With `into_inventory` the drops
    /// `block_drops` leaves go into the player's inventory, and what doesn't fit is dropped
    /// from them (`Inventory.placeItemBackInInventory`); what else the break sets loose (a
    /// torch losing its support, a container's contents) still drops in the world. The
    /// player needn't reach or look at the block. Breaks started from `break_block` or
    /// `block_drops` are heard there too, so a mod keeps its own guard; one started from
    /// inside eight others is refused. True when the block was removed; false, with nothing
    /// broken, for a stale handle, a player who isn't here or is dead, air, an unloaded
    /// block, a block no survival player can break (bedrock), a canceled break, and off a
    /// server.
    #[access(WorldOnly)]
    fn player_destroy_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        into_inventory: bool,
    ) -> bool;
    /// Side: server.
    /// Writes `state` at `pos` in `level` for the player: a state the mod already chose (a
    /// builder's wand, a schematic). It is `level_set_block` with a placement's flags
    /// (`NEIGHBORS | CLIENTS`, and `KNOWN_SHAPE` when `update_neighbour_shapes` is false; with
    /// it the neighbours' shapes update, so fences, walls and stairs connect), then
    /// `Block.setPlacedBy` of the block standing there, in the player's turn: a bed places its
    /// head, a door its upper half. Nothing else of `BlockItem.place` runs: no check that the
    /// block may stand there, no `placed_block` trigger or statistic, no sound, no game
    /// event, no item used, and `setPlacedBy` has no placer. The state is written as given.
    /// Refuses, writing nothing, with [`PlayerWorldRefusal`]: `Client`, `Stale`, `Dead`,
    /// `StaleLevel`, `OtherLevel` (not the level the player is in), `UnknownState`,
    /// `OutOfWorld`, `NotLoaded` and `Unchanged` (the state already stands there, or the
    /// level refused the write as `level_set_block` returns false).
    #[access(WorldOnly)]
    fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        update_neighbour_shapes: bool,
    ) -> Result<(), PlayerWorldRefusal>;
}
=======
>>>>>>> sunrise/main
