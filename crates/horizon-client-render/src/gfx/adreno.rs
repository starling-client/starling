// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// SPDX-License-Identifier: MIT
// Adapted from Vasysik/horizon-android; see THIRD_PARTY_NOTICES.md and LICENSE.
//! What only Adreno on Android's Vulkan takes: the GPU Vasysik's Android fork measured these
//! paths on (an Adreno 710). Every other GPU keeps the usual pipelines and draw order, but for the
//! sky after solid terrain, which every GPU behind Android's Vulkan takes (`late_sky::wanted`).

/// Adreno on Android's Vulkan.
pub(super) fn adreno_vulkan(info: &wgpu::AdapterInfo) -> bool {
    cfg!(target_os = "android")
        && info.backend == wgpu::Backend::Vulkan
        && info.name.contains("Adreno")
}

/// What these paths' pipelines of terrain.wgsl share. Unlike `Renderer::new`'s own, they may fail:
/// the frame then keeps the usual pipeline.
pub(super) struct TerrainModule<'a> {
    pub kept: &'a super::reuse::On<'a>,
    pub shader: &'a wgpu::ShaderModule,
    pub format: wgpu::TextureFormat,
    /// For both stages.
    pub constants: &'a [(&'a str, f64)],
}

impl TerrainModule<'_> {
    /// A pipeline testing depth, as `pipeline_in` in `Renderer::new` builds one.
    #[allow(clippy::too_many_arguments)]
    pub fn pipeline(
        &self,
        layout: &wgpu::PipelineLayout,
        label: &str,
        vertex: &str,
        fragment: &str,
        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
        cull: Option<wgpu::Face>,
        depth_write: bool,
        blend: Option<wgpu::BlendState>,
    ) -> Option<wgpu::RenderPipeline> {
        let options = || wgpu::PipelineCompilationOptions {
            constants: self.constants,
            ..Default::default()
        };
        self.kept
            .try_create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: self.shader,
                    entry_point: Some(vertex),
                    compilation_options: options(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: self.shader,
                    entry_point: Some(fragment),
                    compilation_options: options(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "android"))]
    #[test]
    fn no_gpu_takes_these_paths_off_android() {
        let info = wgpu::AdapterInfo {
            name: "Adreno (TM) 710".into(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::IntegratedGpu,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        };
        assert!(!super::adreno_vulkan(&info));
    }
}
