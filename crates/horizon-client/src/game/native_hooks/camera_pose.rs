use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The camera this frame is drawn from, `partial` ticks past the last tick, after vanilla
    /// placed it (`Camera.setup`): its position, and its yaw and pitch in degrees. Set them to
    /// draw the world from anywhere: a cinematic or replay camera. Only the picture, the sound's
    /// listener and the billboards facing the camera follow it; the player, where they aim, the
    /// first-person hand and which chunks load stay the player's. Values that aren't finite keep
    /// vanilla's. Not called on the title screen's panorama. API 402.
    pub(in crate::game) fn native_camera_pose(
        &self,
        partial: f32,
        position: &mut [f64; 3],
        yaw: &mut f32,
        pitch: &mut f32,
    ) {
        self.fire_camera_pose(partial, position, yaw, pitch);
    }
}
