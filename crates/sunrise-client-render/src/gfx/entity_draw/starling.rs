//! Starling's part of entity_draw.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// Whether this renderer's device was armed for hardware ray tracing (ray queries):
    /// the adapter offered `EXPERIMENTAL_RAY_QUERY` and Ray Tracing (experimental) was
    /// switched on when the device opened (#470).
    pub fn ray_tracing(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
    }
}
