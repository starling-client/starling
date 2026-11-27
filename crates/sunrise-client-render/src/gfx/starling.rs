//! Starling's part of gfx.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The present mode a new window's swapchain starts with, until the game applies VSync and the
/// present-mode setting (`Renderer::set_present_mode`): Mailbox where the surface offers it.
#[cfg(not(target_os = "android"))]
pub(super) fn first_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

/// The present mode a new window's swapchain starts with, until the game applies VSync and the
/// present-mode setting (`Renderer::set_present_mode`): Fifo on Android, where a Mali driver was
/// seen stalling for seconds inside a Mailbox present before the first world frame, long enough
/// for Android to report the app as not responding. Adapted from Vasysik/sunrise-android
/// (`Renderer::new_with_context`), MIT, copyright (c) 2026 Ludovic Marie and the Horizon
/// contributors.
#[cfg(target_os = "android")]
pub(super) fn first_present_mode(_modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    wgpu::PresentMode::Fifo
}

/// Whether sections are drawn with indirect multi-draws where the device takes them: not on
/// Android, which draws each section with a call of its own instead (as OpenGL without indirect
/// draws does), as Vasysik's Android fork does on every Android GPU (its commit 77b9e17f76,
/// `multi_draw`): it measured per-section indirect draws at 46.05 ms of GPU against about 22 ms
/// for direct ones on an Adreno 710. Frames drawn flat (`flat`: Adreno's Vulkan from 512
/// sections, PowerVR's always) are not per-section draws at all. `sunrise-ctl section-draws
/// indirect` brings them back for an A/B check. Adapted from Vasysik/sunrise-android, MIT,
/// copyright (c) 2026 Ludovic Marie and the Horizon contributors.
pub(super) fn indirect_draws_wanted() -> bool {
    !cfg!(target_os = "android")
}

/// Two solid passes only where a discard costs the GPU its hidden surface removal: Apple's tiled
/// GPUs, behind Metal, and Adreno (Vasysik's Android fork measured 25-27% less GPU time on an
/// Adreno 710). A Mali-G57 gained no GPU time from it, and PowerVR hasn't been measured. Elsewhere
/// the second pass would double the solid draw calls and vertex work for nothing.
pub(super) fn splits_solid(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Metal || adreno::adreno_vulkan(info)
}

/// "; memory: process 412 MiB, ..." for a log line (`system_stats::memory_note`), or nothing.
pub(super) fn memory_note() -> String {
    crate::system_stats::memory_note().map_or_else(String::new, |m| format!("; memory: {m}"))
}

/// What a shader pack's shadow camera draws (`cull`: its view-projection, wgpu depth). With depth
/// clamping (the shadow programs' pipelines when the device has it), everything between the
/// light and the camera's near plane too, drawn at that plane: the camera sits 128 blocks toward
/// the light, and with the sun low the ground over a cave is farther than that along its rays;
/// cut off, it let the sun into the cave.
#[cfg(test)]
pub(super) fn light_frustum(cull: Mat4, clamped: bool) -> Frustum {
    let mut planes = frustum(cull);
    if clamped {
        planes[4] = Vec4::W;
    }
    Frustum::from_planes(planes)
}

impl Renderer {
    /// Builds pending ray-tracing BLASes and rebuilds the TLAS (Advanced
    /// graphics' Ray Tracing, while a ray-tracing pack runs).
    pub(super) fn rt_sync(&mut self, camera: DVec3) {
        if self.rt_enabled() {
            self.rt.sync(&self.device, &self.queue, camera);
        }
    }

    /// Whether the world builds ray-tracing structures this frame: an armed
    /// device plus a running ray-tracing pack. Without a pack reading it the
    /// world builds no structures at all.
    pub(super) fn rt_enabled(&self) -> bool {
        self.ray_tracing() && self.native.as_ref().is_some_and(|n| n.ray_tracing)
    }
}
