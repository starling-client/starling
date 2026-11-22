//! GPU timing marker shader and helper types for pass timing.

/// The frame's target and its format.
pub type Target<'a> = (&'a wgpu::TextureView, wgpu::TextureFormat);

pub(crate) const MARKER: &str =
    "@vertex fn cover(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
}
@fragment fn nothing() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
@compute @workgroup_size(1) fn mark() {}";

/// The end marker's pipeline for a `format` target, every write masked.
pub(crate) fn end_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Frame timestamp"),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("cover"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("nothing"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
