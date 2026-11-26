//! Starling's part of server_packs.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl ServerPackState {
    /// Starling: the prompt shows: it takes the back key and the touch controls' screen state.
    pub(in super::super) fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
}
