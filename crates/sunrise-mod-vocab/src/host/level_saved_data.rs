//! The `level_saved_data` area: data the world saves: raids, maps, the respawn data, and saving itself. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_saved_data` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSavedDataBackend {
    /// Side: server.
    /// `Raids.get(id)`: the raid saved under `id` in the requested level, if
    /// `Raids` still holds it (a stopped raid goes at its next tick).
    fn level_raid(&self, level: LevelRef, id: i32) -> Option<RaidRef>;
    /// Side: server.
    /// `ServerLevel.getRaidAt(pos)`: the closest active raid whose centre is
    /// less than 96 blocks away (`Raids.getNearbyRaid`, 9216 squared).
    fn level_raid_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<RaidRef>;
    /// Side: server.
    /// Every raid `Raids` holds, in its iteration order, stopped ones
    /// included until their next tick. None for a closed or stale level.
    fn level_raids(&self, level: LevelRef) -> Option<Vec<RaidRef>>;
    /// Side: server.
    /// The world's respawn data (`LevelData.getRespawnData`): the world spawn, the level it is
    /// in and the way a player appearing there faces, as `/setworldspawn` sets them. The world
    /// has one, so every level of a world answers the same. None on a client, for a stale
    /// handle, and before the world has chosen its spawn.
    fn level_respawn_data(&self, level: LevelRef) -> Option<RespawnData>;
    /// Side: server.
    /// Map `id`'s saved data (`ServerLevel.getMapData(new MapId(id))`): read from memory, or
    /// from `data/minecraft/maps/<id>.dat` when nothing has loaded it yet, without keeping it.
    /// Maps belong to the world, not to a level. None on a client, for a stale handle and for
    /// a map id nothing ever made.
    fn level_map_data(&self, level: LevelRef, id: i32) -> Option<MapState>;
}
/// The handles' writes of the `level_saved_data` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelSavedDataBackendMut {
    /// Side: server.
    /// `LevelData.setRespawnData(RespawnData.of(level, ..))`, as `/setworldspawn` run in this
    /// level does: the world spawn moves to `pos` in this handle's level, facing `yaw` (wrapped
    /// to -180..180) and `pitch` (clamped to -90..90), a player whose own respawn point was the
    /// old world spawn follows it, and `spawn_change` fires. False for a stale handle, a client,
    /// a non-finite angle and a position outside the spawnable bounds.
    fn level_set_respawn_data(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        yaw: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// Repaint map `id` with `colors` (128 by 128 colour ids, row by row), pixel by pixel as
    /// `MapItemSavedData.setColor` does: each changed pixel marks the map to save, to redraw and
    /// to send to the players carrying or framing it. True when a pixel changed. False for
    /// another length, a stale handle, a client and a map id nothing made.
    fn level_set_map_data(&mut self, level: LevelRef, id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// Ask for the level to be saved: its level data, chunks and entities, as the autosave and
    /// `/save-all` save a level. The save runs at the start of the next server tick, not inside
    /// the call; multiple requests before then save it once. True when the request was taken,
    /// not when the save succeeds. `level_save` fires with the level's dimension after the save
    /// pass. Individual record failures are logged but do not fail the pass. A failed awaited
    /// storage flush does fail the pass and suppresses `level_save`. False for a stale handle
    /// and on a client.
    fn level_request_save(&mut self, level: LevelRef) -> bool;
}
