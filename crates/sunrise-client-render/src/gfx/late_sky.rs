// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/sunrise-android; see THIRD_PARTY_NOTICES.md and LICENSE.
//! The vanilla sky after solid terrain, in the same pass and before entities and translucent
//! surfaces. At the far plane it reads the depth solids left, without writing any, so the sky's
//! fragment shaders run only where no terrain covers them. Its colours, fog and rotation are the
//! usual ones. The fork measured less GPU time on an Adreno 710 (14.73 to 13.59 ms standing,
//! 16.5 to 13.5 ms turning), with the same pixels. Shader packs, the End sky, the water mirror
//! and occlusion culling keep the usual order.
//!
//! Built on every GPU behind Android's Vulkan (`wanted`), as the fork takes it, and on there by
//! default except on a Mali (`on_by_default`). `sunrise-ctl sky-order early` brings the usual
//! order back for an A/B check, and the Diagnostics page lets the player pick either.
use super::adreno::TerrainModule;

/// Whether frames on this GPU may draw the sky after solid terrain: any GPU behind Android's
/// Vulkan. Adapted from Vasysik/sunrise-android (`late_sky` in its `gfx.rs`, on any Android
/// Vulkan GPU), MIT, copyright (c) 2026 Ludovic Marie and the Horizon contributors.
pub(super) fn wanted(info: &wgpu::AdapterInfo) -> bool {
    cfg!(target_os = "android") && info.backend == wgpu::Backend::Vulkan
}

/// Whether frames take the late sky until asked otherwise (`Renderer::set_late_sky`, the
/// Diagnostics page's sky choice, `sunrise-ctl sky-order`), on the GPU `name`d. Not on a Mali:
/// the 0.8.1 test 2's guided test found the usual order faster there (median 4% over 144 Malis,
/// 8 to 11% on the Mali-G57), while Adreno came out even and the fork measured the late sky
/// ahead on an Adreno 710.
pub(super) fn on_by_default(name: &str) -> bool {
    !name.to_ascii_lowercase().contains("mali")
}

pub(super) struct LateSky {
    pub sky: wgpu::RenderPipeline,
    pub sunrise: wgpu::RenderPipeline,
    pub celestial: wgpu::RenderPipeline,
    pub stars: wgpu::RenderPipeline,
    pub dark_disc: wgpu::RenderPipeline,
}

impl LateSky {
    /// `celestial`: the bodies' pipeline at the far plane (`Celestial::late`). None where the device
    /// refuses one of them: the sky keeps its usual order.
    pub fn new(
        terrain: &TerrainModule<'_>,
        layout: &wgpu::PipelineLayout,
        sunrise: (&[Option<wgpu::VertexBufferLayout<'_>>], wgpu::BlendState),
        celestial: Option<wgpu::RenderPipeline>,
        stars: &super::stars::Stars,
    ) -> Option<Self> {
        let sky = |label, vertex, fragment, buffers, blend| {
            terrain.pipeline(layout, label, vertex, fragment, buffers, None, false, blend)
        };
        Some(Self {
            sky: sky("Late sky", "vs_sky", "fs_sky_base", &[], None)?,
            sunrise: sky(
                "Late sunrise fan",
                "vs_sunrise_late",
                "fs_sunrise",
                sunrise.0,
                Some(sunrise.1),
            )?,
            celestial: celestial?,
            stars: stars.late_pipeline(terrain.kept, terrain.format)?,
            dark_disc: sky(
                "Late dark disc",
                "vs_sky",
                "fs_dark_disc",
                &[],
                Some(wgpu::BlendState::ALPHA_BLENDING),
            )?,
        })
    }
}

#[cfg(test)]
mod tests;
