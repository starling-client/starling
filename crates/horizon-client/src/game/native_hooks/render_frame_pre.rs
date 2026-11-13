use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once a frame, in a world or not, after Horizon's extras and just
    /// before the frame is rendered (`RenderFrameEvent.Pre`).
    pub(in crate::game) fn native_render_frame_pre(&mut self, elapsed: Duration) {
        self.fire_render_frame_pre(elapsed);
    }
}
