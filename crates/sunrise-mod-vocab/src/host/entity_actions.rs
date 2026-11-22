//! The `entity_actions` area: doing something as an entity: commands, sounds and events. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' writes of the `entity_actions` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityActionsBackendMut {
    /// Side: server.
    /// Runs the command `line` (a leading slash allowed) with the entity as
    /// its source, as vanilla runs a portal's or a sign's command:
    /// `ServerPlayer.createCommandSourceStack` (another entity:
    /// `Entity.createCommandSourceStackForNameResolution`), `.withPermission(level)`,
    /// then `Commands.performPrefixedCommand`. `level` is the mod's choice, taken
    /// as given whether it is above or below the entity's own: the line is parsed
    /// and checked at it. `@s` is the entity, and the command runs from its
    /// position and rotation in its level. A player is answered as for their own
    /// command: its feedback and failures reach their chat, and the other
    /// operators hear of it (`broadcastToAdmins`). Another entity's source is
    /// silent: nothing reaches anyone (`CommandSource.NULL`). A command a mod
    /// registered hears a player as `CommandSource::Player` and another entity as
    /// `CommandSource::Console`, both at `level`. A teleport to another level
    /// happens at the next tick's start, as `Commands::run_command`'s. Whether
    /// the command ran: false for a line that fails to parse or to run, and,
    /// with nothing run, for a stale or removed entity, an entity outside the
    /// level in place (a player in another level included: that level is
    /// parked while the hook runs), and on a client.
    /// During a guest's turn, live players (the acting one included) can run it
    /// and are reachable; displaced outer players are neither, as on
    /// `Commands::run_command`.
    fn entity_run_command(&mut self, entity: EntityRef, line: &str, level: PermissionLevel)
    -> bool;
    /// Side: server.
    /// Runs the command exactly as `EntityRef::run_command` does, at the supplied
    /// `level`, returning whether it ran and the success and failure lines its
    /// source received, in order, as plain text. The player's chat still receives
    /// those lines and operators are notified under the same rules. Another
    /// entity's silent source returns no lines. A stale, removed or unavailable
    /// entity returns unsuccessful with no lines. Cross-level teleports retain
    /// the same next-tick handling. Pass the player's permission level to run
    /// with their own authority; the supplied level is not capped to it.
    fn entity_run_command_with_output(
        &mut self,
        entity: EntityRef,
        line: &str,
        level: PermissionLevel,
    ) -> CommandResult;
    /// Side: server.
    /// Drops `stack` at the entity's feet, `y_offset` above
    /// (`Entity.spawnAtLocation`), or 0.15 out along a frame or painting's
    /// horizontal facing (`HangingEntity.spawnAtLocation`), moving as a new item does, with the
    /// default pickup delay: the item entity. None for an empty stack or an
    /// unknown item, a stale handle, a player in another level, or an item a
    /// mod kept out.
    /// Inside a transaction its join waits for the root commit, so a mod keeping the item out is not
    /// known at the call: the handle is returned and reads stale from the commit on.
    fn entity_spawn_at_location(
        &mut self,
        entity: EntityRef,
        stack: ItemStack,
        y_offset: f32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `Entity.playSound(sound, volume, pitch)`: plays sound event `sound` at
    /// the entity under its sound source, for the players within range. A
    /// silent mob plays nothing, as vanilla's check is `isSilent`; a player
    /// (`Player.playSound`) doesn't hear it themself, their own client plays
    /// it. True means queued, not heard; false for an unknown `minecraft:`
    /// event or a volume or pitch that isn't a finite non-negative number.
    /// Inside an open transaction in the active level, `play_level_sound_at_position` runs
    /// synchronously at the call. The resolved sound waits for the owning level's outermost
    /// commit, in journal order, and is discarded on abort. Outside a transaction, a queued sound
    /// draws its seed at the call even if a later hook cancels it. Inside one, it draws a seed only
    /// when the resolved sound is published.
    fn entity_play_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// `Entity.gameEvent(event)`: game event `event` (`minecraft:step`...)
    /// at the entity's position with the entity as its source, which sculk
    /// sensors, shriekers, wardens and allays in range hear. A player or mob
    /// is the event's source entity; other entities retain their own id.
    /// False for an event vanilla doesn't have, a dead guest and a guest while
    /// another one acts. An accepted game event is posted at the call even in a transaction.
    fn entity_game_event(&mut self, entity: EntityRef, event: &str) -> bool;
    /// Side: server.
    /// `Level.broadcastEntityEvent(entity, id)`: sends entity event `id` (the
    /// byte `Entity.handleEntityEvent` reads: `2` is a hurt flash, `3` a
    /// death, `35` a totem...) to the players tracking the entity and to the
    /// entity itself. True means queued; false for an entity outside the
    /// level in place.
    /// Inside an open transaction, delivery waits for the outermost commit for the owning level
    /// (the level active at the call), in journal order, and is discarded on abort.
    fn entity_broadcast_event(&mut self, entity: EntityRef, id: u8) -> bool;
    /// Side: server.
    /// `Level.playSound(null, entity, sound, source, volume, pitch)`: plays
    /// sound event `sound` bound to the entity, so that it follows it, for
    /// the players within range (16 blocks, times the volume above 1) of
    /// where it is now, the entity's own player too. True means queued, not
    /// heard; false for an entity outside the level in place.
    /// Inside an open transaction in the active level, `play_level_sound_at_entity` runs
    /// synchronously at the call. The resolved sound waits for the owning level's outermost
    /// commit, in journal order, and is discarded on abort. Outside a transaction, a queued sound
    /// draws its seed at the call even if a later hook cancels it. Inside one, it draws a seed only
    /// when the resolved sound is published.
    fn entity_play_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
}
