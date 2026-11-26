//! The `calculate_player_turn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The mouse sensitivity and cinematic camera option a grabbed player's
    /// mouse turn reads this frame (`MouseHandler.turnPlayer`). Change either
    /// for this turn only; the options are unchanged. Client-side. This shapes how far the
    /// mouse turns the view; `player_turn` adjusts the relative `Entity.turn` delta across all
    /// look devices (mouse, controller and keyboard), and `ClientHost::set_local_rotation`
    /// sets the view directly. All three meet in the same angles the tick's move packet sends.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "CalculatePlayerTurnEvent",
        cancellable = false
    )]
    fn calculate_player_turn(
        player_entity_id: i32,
        player_uuid: u128,
        mouse_sensitivity: &mut f64,
        cinematic_camera_enabled: &mut bool,
    );
}
