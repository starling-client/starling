//! Starling's part of region.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl RegionWriter {
    /// Finish into an upgrade staging path reserved by the caller. Android's
    /// shared storage does not support hard links, including within one folder.
    pub(crate) fn finish_staged(self) -> io::Result<()> {
        self.finish_with(|temporary, destination| fs::rename(temporary, destination))
    }
}
