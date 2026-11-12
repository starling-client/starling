// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/horizon-android; see THIRD_PARTY_NOTICES.md and LICENSE.
//! The vanilla sky after solid terrain, in the same pass and before entities and translucent
//! surfaces. At the far plane it reads the depth solids left, without writing any, so the sky's
//! fragment shaders run only where no terrain covers them. Its colours, fog and rotation are the
//! usual ones. The fork measured less GPU time on an Adreno 710 (14.73 to 13.59 ms standing,
//! 16.5 to 13.5 ms turning), with the same pixels. Shader packs, the End sky, the water mirror
//! and occlusion culling keep the usual order.
//!
//! Taken on every GPU behind Android's Vulkan (`wanted`), as the fork takes it. `horizon-ctl
//! sky-order early` brings the usual order back for an A/B check.
use super::adreno::TerrainModule;

/// Whether frames on this GPU may draw the sky after solid terrain: any GPU behind Android's
/// Vulkan. Adapted from Vasysik/horizon-android (`late_sky` in its `gfx.rs`, on any Android
/// Vulkan GPU), MIT, copyright (c) 2026 Ludovic Marie and the Horizon contributors.
pub(super) fn wanted(info: &wgpu::AdapterInfo) -> bool {
    cfg!(target_os = "android") && info.backend == wgpu::Backend::Vulkan
}

pub(super) struct LateSky {
    pub sky: wgpu::RenderPipeline,
    pub sunrise: wgpu::RenderPipeline,
    pub celestial: wgpu::RenderPipeline,
    pub stars: wgpu::RenderPipeline,
    pub dark_disc: wgpu::RenderPipeline,
}

impl LateSky {
    /// `celestial`: the usual pipeline's descriptor. None where the device refuses one of them:
    /// the sky keeps its usual order.
    pub fn new(
        terrain: &TerrainModule<'_>,
        layout: &wgpu::PipelineLayout,
        sunrise: (&[Option<wgpu::VertexBufferLayout<'_>>], wgpu::BlendState),
        celestial: &wgpu::RenderPipelineDescriptor<'_>,
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
            celestial: terrain.kept.try_create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("Late celestial bodies"),
                    vertex: wgpu::VertexState {
                        entry_point: Some("vs_entity_sky"),
                        ..celestial.vertex.clone()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        ..celestial.depth_stencil.clone()?
                    }),
                    ..celestial.clone()
                },
            )?,
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
