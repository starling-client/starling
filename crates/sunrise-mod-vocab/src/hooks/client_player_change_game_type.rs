//! The `client_player_change_game_type` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ClientPlayerChangeGameTypeEvent`: observe any `PlayerInfo` game type change
    /// before the new mode is stored and after `LocalPlayer.onGameModeChanged` runs.
    /// `player_id` is `Some` when the profile has a player entity in the client level.
    #[hook(
        dispatch = "flat",
        side = "client",
        contract = "listen",
        version = 2,
        neoforge = "ClientPlayerChangeGameTypeEvent",
        cancellable = false
    )]
    fn client_player_change_game_type(
        player_id: Option<i32>,
        player_uuid: u128,
        current_game_type: i32,
        new_game_type: i32,
    );
}
