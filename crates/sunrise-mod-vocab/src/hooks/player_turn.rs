//! The `player_turn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Relative turn delta about to be applied to the local player (`Entity.turn`), in
    /// `Entity.turn` units where `1.0` equals `0.15` degrees (`x` is yaw, `y` is pitch).
    /// Fires on the client for every relative look source that turns the player (mouse after
    /// `calculate_player_turn`, controller stick, arrow keys and control-socket `turn`),
    /// skipping freecam and direct `ClientHost::set_local_rotation`. Edit `x` and `y` to
    /// scale or adjust the turn; an axis whose `(delta as f32) * 0.15` is not finite logs a
    /// warning and keeps its original delta.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla Entity.turn; CalculatePlayerTurnEvent covers mouse sensitivity/cinematic state only)",
        cancellable = false
    )]
    #[notes = r#"### Local player relative turn (`player_turn`)

- `player_turn(player_entity_id, player_uuid, x, y)` runs on the client immediately before the
  local player's `Entity.turn(xo, yo)` applies a relative look delta, whether it came from the
  mouse (after `calculate_player_turn` and sensitivity/cinematic smoothing), a controller stick,
  the keyboard turn keys, or `sunrise-ctl turn`. Freecam and direct `ClientHost::set_local_rotation`
  skip it.
- `x` (yaw) and `y` (pitch) are in vanilla `Entity.turn` units (`(delta as f32) * 0.15` degrees),
  matching `Entity.turn(double xo, double yo)`. Listeners run in hook order and chain changes;
  if an axis's `(delta as f32) * 0.15` is not finite after dispatch, Sunrise logs a warning and
  reverts that axis to its incoming value.
- Use `calculate_player_turn` when you specifically need mouse sensitivity or cinematic camera
  smoothing (`MouseHandler.turnPlayer`); use `player_turn` when a turn scaling or adjustment
  should apply across mouse, controller and keyboard look alike (for example, slowing look speed
  while zoomed)."#]
    fn player_turn(player_entity_id: i32, player_uuid: u128, x: &mut f64, y: &mut f64);
}
