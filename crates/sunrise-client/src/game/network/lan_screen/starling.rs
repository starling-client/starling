//! Starling's part of lan_screen.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: the game rules' screen while it is open, for the keyboard's editor on Android
    /// (touch/text_mirror.rs).
    pub(in super::super) fn world_rules(&mut self) -> Option<&mut RulesScreen> {
        self.world_options.as_mut()?.rules.as_mut()
    }
    /// Starling: the LAN port's box while it takes the keys, for the keyboard's editor on Android.
    pub(in super::super) fn world_options_port(&mut self) -> Option<&mut EditBox> {
        match self.world_options.as_mut() {
            Some(o) if o.port.focused && o.view == View::Options => Some(&mut o.port),
            _ => None,
        }
    }
}
