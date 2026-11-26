//! Starling's part of capture.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// Starling: whether the device times frames (`GpuTimer`), for the log that says whether
    /// it ever timed one.
    pub fn gpu_timer_on(&self) -> bool {
        self.gpu_timer.is_some()
    }
}
