/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"a player's respawn point: `player_respawn_position` is heard once per death respawn or after the End credits (`from_end_fight`), before the replacement is created (NeoForge's `PlayerRespawnPositionEvent`, Bukkit's `PlayerRespawnEvent.setRespawnLocation`), with their bed, anchor, forced point or the world spawn. A changed level or position sets the final placement without another block check; yaw and pitch set their look. This does not undo a spawn block already used to resolve the original transition. An unknown dimension or a non-finite value leaves that part unchanged. Bundles that use it need `"hooks": @API:player_respawn_position@`."###]
    fn player_respawn_position();
}
