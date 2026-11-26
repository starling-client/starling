//! Starling's part of chat.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// The whole input from the on-screen keyboard's editor (Android, game/touch/text_mirror.rs).
    pub(in super::super) fn chat_set_text(&mut self, text: &str, cursor: usize, anchor: usize) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.set_text(&m, &c, text, cursor, anchor);
        }
    }

    /// The chat bar's Paste (touch.rs): the clipboard's text at the cursor, as Ctrl+V.
    pub(in super::super) fn chat_paste(&mut self) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            let paste = sunrise_client::clipboard::EditShortcut::Paste;
            screen.shortcut(&m, &c, paste, &mut self.clipboard);
        }
    }

    /// The pointer where the chat lays its parts out: over the on-screen keyboard the chat is
    /// drawn higher (`lift_over_keyboard`), so the point under a finger is that much lower in it.
    pub(super) fn chat_pointer(&self) -> (f32, f32) {
        let scale = self.renderer.as_ref().map_or(1.0, |r| {
            self.interface_batch([r.config.width, r.config.height])
                .scale
        });
        (self.cursor.0, self.cursor.1 + self.chat_lift(scale))
    }
}
