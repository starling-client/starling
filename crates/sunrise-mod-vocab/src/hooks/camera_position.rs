//! The `camera_position` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla Camera.setPosition; NeoForge has no camera position event)",
        cancellable = false
    )]
    /// Render-only world-space camera position after `Camera.alignWithEntity` places the camera
    /// (`Camera.setPosition`).
    ///
    /// Starts from the frame's placed world position in blocks: after vanilla entity alignment
    /// (including an interpolating minecart rider) or from the studio's selected position.
    /// Read the previous listener's accepted position with `position.coordinates()`.
    /// Pin individual axes with `position.set([Some(x), None, None])`; a non-finite edit
    /// returns `false` and leaves the last accepted position intact.
    ///
    /// The render camera and its audio listener move; the player's position, eye and aim do not.
    /// A perspective request during this callback re-aligns an entity-bound camera, keeping only
    /// the explicitly pinned coordinates, even if unchanged. It does not reposition a custom camera.
    fn camera_position(partial: f32, position: &mut mod_api::CameraPosition);
}
