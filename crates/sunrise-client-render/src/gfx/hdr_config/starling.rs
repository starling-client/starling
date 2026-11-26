//! Starling's part of hdr_config.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Renderer {
    /// Removes `marker` (`graphics::RENDERER_MARKER`, written before this renderer opened its
    /// device) once a frame has been presented: the start went through.
    pub fn forget_start_marker(&mut self, marker: std::path::PathBuf) {
        self.start_marker = Some(marker);
    }

    /// Keeps the pipelines the driver compiled for the next start (`vulkan_pipeline_cache`): on
    /// Android's Vulkan, written on a thread of its own; elsewhere nothing.
    pub fn save_pipeline_cache(&self) {
        #[cfg(target_os = "android")]
        if self.info.backend == wgpu::Backend::Vulkan {
            wgpu::hal::vulkan::disk_cache::save();
        }
    }
}
