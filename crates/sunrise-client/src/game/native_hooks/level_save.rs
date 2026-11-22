use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// The end of `dimension`'s level save pass (NeoForge's `LevelEvent.Save`
    /// in `ServerLevel.save`), for autosave, `/save-all`, pause and shutdown.
    /// Individual record failures are reported and do not suppress this hook;
    /// a failed awaited storage flush does. Server-side.
    pub(in crate::game) fn native_level_save(&mut self, dimension: &str) {
        self.fire_level_save(dimension);
    }
}
