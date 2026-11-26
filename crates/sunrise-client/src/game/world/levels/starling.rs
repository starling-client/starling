//! Starling's part of levels.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Levels {
    /// Starling: the save folders of the levels not in place, to wait for their writes.
    pub(in super::super) fn parked_saves(
        &self,
    ) -> Vec<std::sync::Arc<sunrise_client::save::SaveDir>> {
        self.parked.values().map(|p| p.save.clone()).collect()
    }
}
