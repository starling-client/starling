use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// The normal menu blur request, before finite/range validation. None disables it.
    pub(in crate::game) fn native_menu_background(&self, radius: &mut Option<f32>) {
        self.fire_menu_background(radius);
    }
}
