use super::*;

/// Starling: hook points Horizon's game had, which mixin mods made for Horizon target. Each does nothing.
impl Game<'_> {
    /// One HUD layer has drawn (vanilla's or a mod's), before its `native_gui_layer_post`: a mixin
    /// adds to the layer `layer` here (Trailgrid Maps' map background).
    pub(in crate::game) fn native_gui_layer_draw(&self, layer: &str, draw: &mut HudBatch) {
        let _ = (layer, draw);
    }
}
