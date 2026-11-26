use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Before pass-bearing world render stages execute (NeoForge's `PrepareRenderBuffersEvent`).
    pub(in crate::game) fn native_prepare_render_buffers(
        &self,
        view: &mod_api::WorldRenderView<'_>,
        surface: &mut crate::world_geometry::WorldGeometry,
    ) {
        self.fire_prepare_render_buffers(view, surface);
    }
}
