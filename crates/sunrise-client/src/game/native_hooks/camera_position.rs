use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Render-only world-space camera position after `Camera.alignWithEntity` places the camera
    /// (`Camera.setPosition`).
    pub(in crate::game) fn native_camera_position(
        &self,
        partial: f32,
        position: &mut mod_api::CameraPosition,
    ) {
        self.fire_camera_position(partial, position);
    }
}
