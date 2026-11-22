//! Device-sized vertex uploads for explicit particle quads. A mod can submit any
//! number of quads; only the GPU buffer size limits an individual draw.
use super::{Vertex, entity_draw::EntityPipelineOptions, upload::Uploads};

/// The particle shader path, including the back-face test in improved transparency.
pub(crate) const EXPLICIT_PARTICLE_OVERLAY: u32 = 47;

pub(super) struct ParticlePipelines {
    pub opaque: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    pub mark: wgpu::RenderPipeline,
}

impl ParticlePipelines {
    pub(super) fn new(build: impl Fn(EntityPipelineOptions<'_>) -> wgpu::RenderPipeline) -> Self {
        // RenderPipelines.OPAQUE_PARTICLE and TRANSLUCENT_PARTICLE share their
        // face and depth rules and alpha cut; only the latter blends.
        let options = EntityPipelineOptions {
            label: "Opaque provider particles",
            cull_mode: Some(wgpu::Face::Back),
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: wgpu::BlendState::REPLACE,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity",
            write_mask: wgpu::ColorWrites::ALL,
        };
        Self {
            opaque: build(options),
            translucent: build(EntityPipelineOptions {
                label: "Translucent provider particles",
                blend: wgpu::BlendState::ALPHA_BLENDING,
                ..options
            }),
            mark: build(EntityPipelineOptions {
                label: "HDR provider particle mark",
                depth_write: false,
                fragment: "fs_hdr_mark",
                write_mask: wgpu::ColorWrites::ALPHA,
                ..options
            }),
        }
    }
}

#[derive(Default)]
pub(super) struct ParticleDraws {
    buffers: Vec<Batch>,
    active: usize,
}

struct Batch {
    vertices: wgpu::Buffer,
    quads: u32,
}

impl ParticleDraws {
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        vertices: &[Vertex],
        max_buffer_size: u64,
    ) -> u32 {
        self.active = 0;
        if vertices.is_empty() {
            return 0;
        }
        let vertex_stride = size_of::<Vertex>() as u64;
        // Six u32 indices per quad. The renderer's shared index capacity grows in
        // powers of two; the four vertices are larger even than twice the indices.
        let max_quads = (max_buffer_size / (4 * vertex_stride)).min(u64::from(u32::MAX / 6));
        assert!(max_quads != 0, "the renderer needs space for one quad");
        // Uploads rounds its staging allocation up to a power of two.
        let staging_size = 1u64 << device.limits().max_buffer_size.ilog2();
        let mut largest = 0;
        for (batch, vertices) in vertices.chunks(max_quads as usize * 4).enumerate() {
            let quads = (vertices.len() / 4) as u32;
            let bytes = bytemuck::cast_slice(vertices);
            let size = bytes.len() as u64;
            if batch == self.buffers.len() || size > self.buffers[batch].vertices.size() {
                let new = Batch {
                    vertices: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Explicit particle vertices"),
                        size: size.next_power_of_two().min(max_buffer_size),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    quads,
                };
                if batch == self.buffers.len() {
                    self.buffers.push(new);
                } else {
                    self.buffers[batch] = new;
                }
            }
            let batch = &mut self.buffers[batch];
            batch.quads = quads;
            for (index, upload) in bytes.chunks(staging_size as usize).enumerate() {
                uploads.buffer(
                    encoder,
                    &batch.vertices,
                    index as u64 * staging_size,
                    upload,
                );
            }
            self.active += 1;
            largest = largest.max(quads);
        }
        largest
    }

    pub(super) fn any(&self) -> bool {
        self.active != 0
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        for batch in &self.buffers[..self.active] {
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            pass.draw_indexed(0..batch.quads * 6, 0, 0..1);
        }
    }
}
