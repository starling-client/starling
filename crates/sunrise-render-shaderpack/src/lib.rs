//! Shader pack pipeline implementation for Sunrise (OptiFine / Iris).
//!
//! Provides GPU timing, atlas stitching, passes, and shaderpack uniforms.

pub mod joml;
pub mod pack_atlas;
pub mod pack_center_depth;
pub mod pack_draw;
pub mod pack_entities;
pub mod pack_hand;
pub mod pack_images;
pub mod pack_lines;
pub mod pack_post;
pub mod pack_shadow;
pub mod pack_sky;
pub mod pack_textures;
pub mod pack_timing;
pub mod pack_uniforms;
pub mod pack_weather;
pub(crate) mod timing;

/// The client's loss-aware `Device::poll` (`gfx::in_flight::poll_live`), which turns the panic a
/// wait on a lost device can raise into the loss. Every wait this crate makes takes it.
pub type PollLive =
    fn(&wgpu::Device, &std::sync::atomic::AtomicBool, wgpu::PollType) -> Result<(), String>;

/// What a celestial quad is (the client's `gfx::CelestialBody`), for the pack's sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CelestialBody {
    Sun,
    Moon,
    /// The End's flash (`SkyRenderer.renderEndFlash`): neither the sun nor the moon.
    EndFlash,
}

#[cfg(test)]
pub(crate) mod test_gpu;
