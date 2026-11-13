use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once a frame, in a world or not, right after the frame was rendered
    /// (`RenderFrameEvent.Post`).
    pub(in crate::game) fn native_render_frame_post(&mut self, elapsed: Duration) {
        self.fire_render_frame_post(elapsed);
    }
}
