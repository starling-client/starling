//! The night sky's stars, built once into a static vertex buffer as
//! `SkyRenderer.buildStars` does, and drawn each frame with one small uniform
//! (the sky's rotation and the star brightness) instead of being regenerated
//! and uploaded every frame.
use glam::Mat4;
use wgpu::util::DeviceExt;

/// `SkyRenderer.buildStars` with its fixed seed: up to 1500 quads on a
/// sphere of radius 100, each turned to face the centre and spun about its
/// own axis, as triangles (two per quad).
pub fn star_triangles() -> Vec<[f32; 3]> {
    super::pack_sky::star_triangles()
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    clip_from_sky: [[f32; 4]; 4],
    color: [f32; 4],
    hdr_settings: [f32; 4],
    hdr_gains: [f32; 4],
    hdr_light: [f32; 4],
}

/// The stars' pipeline descriptor, handed to `make`.
fn with_descriptor<R>(
    (layout, shader, format, linear): (
        &wgpu::PipelineLayout,
        &wgpu::ShaderModule,
        wgpu::TextureFormat,
        bool,
    ),
    (label, vertex, depth_compare): (&str, &str, wgpu::CompareFunction),
    make: impl FnOnce(&wgpu::RenderPipelineDescriptor<'_>) -> R,
) -> R {
    let attributes = wgpu::vertex_attr_array![0 => Float32x3];
    make(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(depth_compare),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_stars"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &super::linear_blend::frame_constants(format, linear),
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // RenderPipelines.STARS: the celestial blend (src alpha, one), with the
                // colour premultiplied in fs_stars, so its alpha can carry HDR's mark.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) struct Stars {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    count: u32,
    /// Whether the frame holds linear light.
    linear: bool,
}

impl Stars {
    /// The stars onto a frame of `format`, holding linear light when `linear`.
    pub(super) fn new(
        device: &wgpu::Device,
        (format, linear): (wgpu::TextureFormat, bool),
    ) -> Self {
        let triangles = star_triangles();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Stars vertex buffer"),
            contents: bytemuck::cast_slice(&triangles),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Stars rotation and brightness"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Stars bindings"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Stars resources"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Stars shader"),
            source: wgpu::ShaderSource::Wgsl(
                (include_str!("../hdr_light.wgsl").to_owned() + include_str!("stars.wgsl")).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Stars layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = with_descriptor(
            (&pipeline_layout, &shader, format, linear),
            ("Stars", "vs_stars", wgpu::CompareFunction::Always),
            |descriptor| device.create_render_pipeline(descriptor),
        );
        Self {
            pipeline,
            layout: pipeline_layout,
            shader,
            group,
            uniform,
            vertices,
            count: triangles.len() as u32,
            linear,
        }
    }

    /// This frame's sky rotation (camera-relative world from star space,
    /// times the view projection) and star brightness.
    pub(super) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        clip_from_sky: Mat4,
        brightness: f32,
        globals: &super::Globals,
    ) {
        let b = brightness.clamp(0.0, 1.0);
        let uniform = Uniform {
            clip_from_sky: clip_from_sky.to_cols_array_2d(),
            color: [b; 4],
            hdr_settings: globals.hdr_settings,
            hdr_gains: globals.hdr_gains,
            hdr_light: globals.hdr_light,
        };
        uploads.buffer(encoder, &self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// The stars at the far plane, for the sky drawn after solid terrain (`late_sky`). None
    /// where the device refuses it.
    pub(super) fn late_pipeline(
        &self,
        kept: &super::reuse::On<'_>,
        format: wgpu::TextureFormat,
    ) -> Option<wgpu::RenderPipeline> {
        with_descriptor(
            (&self.layout, &self.shader, format, self.linear),
            (
                "Late stars",
                "vs_stars_late",
                wgpu::CompareFunction::GreaterEqual,
            ),
            |descriptor| kept.try_create_render_pipeline(descriptor),
        )
    }

    #[sunrise_mod_api_macros::replaceable]
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_through(pass, &self.pipeline);
    }

    pub(super) fn draw_through(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
    ) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_lie_on_the_sky_sphere_as_whole_quads() {
        let triangles = star_triangles();
        // About 52% of the cube's random points fall inside the unit ball.
        assert!((700 * 6..1500 * 6).contains(&triangles.len()));
        assert_eq!(triangles.len() % 6, 0);
        for p in &triangles {
            let r = glam::Vec3::from_array(*p).length();
            assert!((99.9..100.1).contains(&r), "{r}");
        }
        assert_eq!(triangles, star_triangles());
    }
}
