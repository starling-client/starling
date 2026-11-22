use super::*;

impl Game<'_> {
    /// `Hud.getGuiTicks`, independent of the server and visual clocks.
    pub(in crate::game) fn gui_ticks(&self) -> i32 {
        self.gui_ticks
    }
}
