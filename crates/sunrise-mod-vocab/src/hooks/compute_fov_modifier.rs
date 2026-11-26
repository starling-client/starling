//! The `compute_fov_modifier` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ComputeFovModifierEvent`: the effective camera player's FOV scale (the local
    /// player, or the player being spectated), before camera easing. `player` is that player's
    /// handle, `raw` is `AbstractClientPlayer.getFieldOfViewModifier`'s product before its
    /// final `Mth.lerp`, and `effect_scale` is the Field of View Effects setting the lerp uses.
    /// `modifier` starts as `Mth.lerp(effect_scale, 1, raw)`, the value vanilla uses; write it
    /// to change the field of view. At `effect_scale` 0 that start is 1 whatever `raw` is.
    /// The hook doesn't fire for a non-player camera, for a player with no handle (a detached
    /// spectated snapshot), or while the camera player scopes with a spyglass in first person
    /// (`AbstractClientPlayer.getFieldOfViewModifier` returns `0.1` before the event).
    /// Invalid values or values outside 0.01..=10 revert to the start.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ComputeFovModifierEvent",
        cancellable = false,
        version = 3
    )]
    fn compute_fov_modifier(
        player: mod_api::PlayerRef,
        raw: f32,
        effect_scale: f32,
        modifier: &mut f32,
    );
}
