//! The `camera_submersion` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Override the client view's submersion. Camera fluid reads (`Camera.getFluidInCamera`)
    /// start with the game's fog type after `state_at_viewpoint`. The first-person water overlay
    /// instead asks at the local player's eye with `Water` if the player's tick sampled water
    /// there, or `Atmospheric` otherwise. The sample comes from death tick 20 once the local
    /// player is removed, until respawn. Setting `Water` enables the overlay; any other value
    /// suppresses it. Set `Atmospheric` to clear submersion or a fluid to force it. Later listeners
    /// see the last successful value. A panic disables only that mod's listener for the session
    /// and is reported in the log and the client error toast.
    ///
    /// The fluid field of view, the frame's fog, the sky pass and its dark disc, water and conduit
    /// vision, and a shader pack's `isEyeInWater` also read the result. The hook fires once per
    /// reader, at that reader's position; the input can differ between readers, even at the same
    /// point. The position can't be changed. Server and gameplay fluid checks do not read it.
    /// Client-side. Beyond NeoForge: there is no counterpart.
    /// Diverges from upstream: new hook.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla Camera.getFluidInCamera; NeoForge adds no event for it)",
        version = 2
    )]
    fn camera_submersion(camera: [f64; 3], fog_type: &mut mod_api::client::fog::FogType);
}
