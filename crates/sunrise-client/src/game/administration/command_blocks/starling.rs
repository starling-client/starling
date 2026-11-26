//! Starling's part of command_blocks.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: Enter, as Done: the command block set, the screen closed.
    pub(in super::super) fn command_block_done(&mut self) {
        if let Some(s) = self.command_block_screen.take() {
            self.set_command_block(&s);
        }
        self.close_command_block();
    }
    /// Starling: the command box, for the on-screen keyboard's editor (touch/text_mirror.rs).
    pub(in super::super) fn command_box(
        &mut self,
    ) -> Option<&mut sunrise_client::screen_draw::TextField> {
        self.command_block_screen
            .as_mut()
            .map(|s| &mut s.command.input)
    }
    /// Starling: the on-screen keyboard's editor's text and selection on the command box, through
    /// the box's own typing (its filter and limit hold) and with its suggestions following.
    pub(in super::super) fn command_block_set_text(
        &mut self,
        text: &str,
        cursor: usize,
        anchor: usize,
    ) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if let Some(s) = self.command_block_screen.as_mut() {
            s.command.set_text(&m, &c, text, cursor, anchor);
        }
    }
}
