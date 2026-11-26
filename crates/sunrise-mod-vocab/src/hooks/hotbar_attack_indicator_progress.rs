//! Declaration of the hook that edits the stock hotbar attack indicator.

pub trait Hooks {
    /// Edit the stock hotbar's attack indicator for this draw only (`Hud.extractItemHotbar`).
    /// After item extraction, `event.progress` starts at the local player's attack strength
    /// at partial tick zero, even if the camera follows another player. This runs at full
    /// charge too: `1.0` hides the indicator, while a lower value makes it visible.
    ///
    /// Each listener receives the last accepted value. Progress must be finite and in
    /// `0.0..=1.0` (`Player.getAttackStrengthScale`); a refused write leaves the accepted
    /// value in place without disabling the listener. This hook cannot cancel the layer
    /// or change the crosshair indicator, attack charge or combat.
    ///
    /// Runs with a read-only client Host, only in Hotbar mode with a visible HUD and a
    /// player camera, outside spectator mode. It does not run if the original hotbar body
    /// is cancelled, replaced or skipped by a wrapper.
    #[hook(
        receiver = "read",
        side = "client",
        contract = "listen",
        output_validator = "crate::client::hud_layout::validate_hotbar_attack_indicator_progress"
    )]
    fn hotbar_attack_indicator_progress(progress: &mut f32);
}
