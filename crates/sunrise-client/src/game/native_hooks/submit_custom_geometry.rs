use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// After particle submission, before the renderer executes world draws.
    pub(in crate::game) fn native_submit_custom_geometry(
        &self,
        view: &mod_api::WorldRenderView,
        draw: &mut sunrise_client::world_geometry::WorldGeometry,
    ) {
        self.fire_submit_custom_geometry(view, draw);
    }
}
