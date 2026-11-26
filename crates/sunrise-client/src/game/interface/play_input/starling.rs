//! Starling's part of play_input.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: a key from the touch controls, the on-screen keyboard or a custom binding. These
    /// come with no window event, which queues a tick mapping's click by its slot
    /// (`mapped_key_event`), so the click of the key's mapping (drop, swap hands, inventory, the
    /// hotbar...) is queued here, as `key_pressed` did for them before the sync with Sunrise.
    pub(in crate::game) fn mobile_mapped_key(
        &mut self,
        key: KeyCode,
        down: bool,
        raw: KeyCode,
    ) -> Result<()> {
        self.mapped_key(key, down, false, raw, Source::Keyboard)?;
        if down
            && !self.mod_mappings_screen_open()
            && !self.replay.playing()
            && let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
            && Self::tick_mapping(slot.definition().0)
        {
            self.vanilla_mappings.click(slot);
        }
        Ok(())
    }
}
