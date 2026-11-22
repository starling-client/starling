//! The `player_set_spawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `PlayerSetSpawnEvent`: a server player's respawn point is about to be
    /// set (`ServerPlayer.setRespawnPosition`), once for each call vanilla makes: every sleep
    /// in a bed (the same bed too, which refreshes its angle), an anchor that is a new
    /// point, `/spawnpoint`, and the copy to the replacement after `player_clone` when
    /// `player_respawn_position` allows copying (by default unless the saved block was
    /// missing). `pos` is the block position and `dimension` its level; `pos` is none
    /// when no point is set. `forced` is `/spawnpoint`'s flag. Cancel to keep the
    /// current point, including a point already set on the replacement by a clone
    /// listener. `player_entity_id` is the server's entity id for the player. Server-side.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        version = 2,
        neoforge = "PlayerSetSpawnEvent",
        cancellable = true
    )]
    fn player_set_spawn(
        player: u128,
        player_entity_id: i32,
        name: &str,
        dimension: &str,
        pos: Option<[i32; 3]>,
        forced: bool,
        cancel: &mut bool,
    );
}
