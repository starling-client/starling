//! The `render_view_transform` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla GameRenderer.bobHurt/bobView; NeoForge adds no event for it)",
        cancellable = false
    )]
    /// Presentation-only view transform applied at vanilla's `GameRenderer.bobHurt` / `bobView`
    /// stage, after hurt tilt and view bobbing and before rendering the world. It also runs when
    /// a screen has input focus and this world render stage still runs.
    ///
    /// `transform` starts at identity each frame. Listeners compose local
    /// rotations, translations, and scales onto it in dependency order (for example, camera shake
    /// scaled by [`ClientGuiOptions::screen_effect_scale`], recoil kicks, or lean tilts). Sunrise
    /// multiplies the composed matrix directly into the view-bob matrix with no artificial angle or
    /// offset caps. An operation returns `false` and keeps the current transform if its proposal
    /// or any affected frame composition cannot be inverted finitely.
    /// The camera position and angles, the audio listener, the player position and aim are unchanged.
    fn render_view_transform(partial: f32, transform: &mut mod_api::ViewTransform<'_>);
}
