//! Starling's part of world_entry.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Waits until the world resources the title's start loads on a worker (`rest_loader`) are
    /// in: the items, mobs and recipes `Resources::rest` holds.
    pub(in super::super) fn wait_world_rest(&mut self) -> Result<()> {
        while self.rest_loader.is_some() {
            self.poll_world_rest()?;
            if self.rest_loader.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        Ok(())
    }
}
