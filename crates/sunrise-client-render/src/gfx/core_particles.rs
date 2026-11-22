//! Polytone GPU particle types with their own core shader (wiki "GPU Particles", `shader`): each
//! type's live records go to its shader as the vertex format, the same record on the quad's four
//! corners, and the shader places, colours and kills them on the graphics card. Types without a
//! shader, and those whose shader doesn't build here or that the frame has no room for, are built
//! on the CPU with the engine's particles (`particles::polytone_gpu`).

use crate::particles::polytone_gpu::{
    FALLBACK_QUADS, GpuParticleRenderer, GpuShader, ShaderBatch, ShaderRequest, ShaderVertex,
};
use std::sync::Arc;
use sunrise_polytone::gpu_particle::{GPU_BLOCK, GPU_BLOCK_SIZE, Surface};
use sunrise_polytone::particles::RenderType;

/// Each vanilla block's offset in a pipeline's uniform buffer (`core_shader::BLOCKS` order), as
/// `core_lines::uniforms` lays them out, and their sizes.
const BLOCK_OFFSETS: [u64; 4] = [0, 256, 512, 768];
const BLOCK_SIZES: [u64; 4] = [48, 64, 160, 48];
/// Where the type's own blocks start (`polytone:gpu_particle.glsl`, then one per uniform), 256
/// bytes apart.
const OTHER_OFFSET: u64 = 1024;
/// A pack uniform's block: one float, padded as std140 pads a block.
const FLOAT_BLOCK: u64 = 16;
/// The heightmap `Sampler1` holds: this many columns a side around the camera.
pub const HEIGHTMAP_SIZE: u32 = 128;
/// The most quads a frame draws with the types' own shaders, all types together: a type's ring
/// of records times its `area`'s copies is the pack's to choose.
const MAX_QUADS: usize = 1 << 20;
/// A quad's bytes in the record buffer: the record on its four corners.
const QUAD_BYTES: u64 = std::mem::size_of::<ShaderVertex>() as u64 * 4;

/// The quads the types drawn on the CPU may add to a frame on a device whose buffers hold
/// `max_buffer_size` bytes: [`FALLBACK_QUADS`], or a quarter of the entity vertex buffer the
/// device allows, leaving the rest to the frame's other quads and the buffer's growth to a
/// power of two (`gfx::entity_buffer_vertices`).
pub(super) fn fallback_quads(max_buffer_size: u64) -> usize {
    let quad = std::mem::size_of::<crate::mesher::Vertex>() as u64 * 4;
    FALLBACK_QUADS.min((max_buffer_size / quad / 4) as usize)
}

/// The record as vertex attributes 0 to 5 (`polytone_gpu::GLUE`).
const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Float32x4,
    4 => Unorm8x4,
    5 => Sint32x2,
];

/// One type's pipeline over its shader, with its uniforms and its texture.
struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// Each block after the vanilla ones: the type's knobs, or the pack uniform of that name.
    other: Vec<Other>,
}

enum Other {
    Knobs,
    Uniform(String),
}

impl Other {
    fn size(&self) -> u64 {
        match self {
            Self::Knobs => GPU_BLOCK_SIZE as u64,
            Self::Uniform(_) => FLOAT_BLOCK,
        }
    }
}

/// A draw of this frame: a pipeline and its quads in the record buffer.
struct Draw {
    pipeline: usize,
    first: u32,
    quads: u32,
}

pub(crate) struct CoreParticles {
    /// By shader: `None` for one that doesn't build, or can't draw into this frame (logged once).
    pipelines: Vec<(Arc<GpuShader>, Option<Pipeline>)>,
    /// The frame the pipelines draw into: its format, and whether it holds linear light.
    target: (wgpu::TextureFormat, bool),
    /// The batches the game handed over for the frame it builds
    /// (`Renderer::set_polytone_batches`).
    batches: Vec<ShaderBatch>,
    vertices: wgpu::Buffer,
    /// The quads a frame's batches may hold together: [`MAX_QUADS`], or fewer when a vertex
    /// buffer can't hold them.
    budget: usize,
    heightmap: wgpu::Texture,
    heightmap_view: wgpu::TextureView,
    /// The snapshot `heightmap` holds, none until the first upload (a new renderer's texture
    /// is unfilled).
    uploaded: Option<Surface>,
    sampler: wgpu::Sampler,
    blank: wgpu::TextureView,
    draws: Vec<Draw>,
}

impl CoreParticles {
    /// For a world frame of `target` (format, linear light).
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: (wgpu::TextureFormat, bool),
    ) -> Self {
        let heightmap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Polytone GPU particle heightmap"),
            size: wgpu::Extent3d {
                width: HEIGHTMAP_SIZE,
                height: HEIGHTMAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let blank = texture(device, queue, 1, 1, &[255; 4]);
        Self {
            pipelines: Vec::new(),
            target,
            batches: Vec::new(),
            vertices: vertex_buffer(device, 1 << 16),
            budget: MAX_QUADS.min((device.limits().max_buffer_size / QUAD_BYTES) as usize),
            uploaded: None,
            heightmap_view: heightmap.create_view(&Default::default()),
            heightmap,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Polytone GPU particles"),
                ..Default::default()
            }),
            blank: blank.create_view(&Default::default()),
            draws: Vec::new(),
        }
    }

    /// The motion-blocking heights `Sampler1` holds: [`HEIGHTMAP_SIZE`] columns a side, x along
    /// a row and z down the rows, each the height above the world's bottom. Uploaded when it
    /// isn't the snapshot the texture already holds (whether it was).
    fn set_heightmap(&mut self, queue: &wgpu::Queue, surface: &Surface) -> bool {
        if surface.area().size != HEIGHTMAP_SIZE
            || self.uploaded.as_ref().is_some_and(|held| {
                held.area() == surface.area() && Arc::ptr_eq(held.heights(), surface.heights())
            })
        {
            return false;
        }
        // Wiki "GPU Particles": r is the low byte, g the high byte.
        let texels: Vec<u8> = surface
            .heights()
            .iter()
            .flat_map(|h| {
                let [low, high] = h.to_le_bytes();
                [low, high, 0, 255]
            })
            .collect();
        queue.write_texture(
            self.heightmap.as_image_copy(),
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(HEIGHTMAP_SIZE * 4),
                rows_per_image: Some(HEIGHTMAP_SIZE),
            },
            self.heightmap.size(),
        );
        self.uploaded = Some(surface.clone());
        true
    }

    /// The renderer's verdict on `requests` (the GPU particle types with live records) for the
    /// frame about to be built, before its quads are: a shader whose pipeline builds for a
    /// frame of `target` (format, linear light) is drawn when all of its type's quads fit in
    /// what the types before it left of the frame's budget, and its type leaves its quads out;
    /// any other keeps them. Pipelines are built the first time; a verdict is given again every
    /// frame, so a type's state is always this renderer's for this frame.
    pub(super) fn decide(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: (wgpu::TextureFormat, bool),
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
        requests: &[ShaderRequest],
    ) {
        if self.target != target {
            self.pipelines.clear();
            self.target = target;
        }
        // The shaders of packs no longer loaded: only this cache holds them.
        self.pipelines.retain(|(s, _)| Arc::strong_count(s) > 1);
        let mut room = self.budget;
        for ShaderRequest { shader, quads } in requests {
            let built = match self.pipelines.iter().find(|(s, _)| Arc::ptr_eq(s, shader)) {
                Some((_, pipeline)) => pipeline.is_some(),
                None => {
                    let pipeline = self.pipeline(device, queue, lightmap, shader);
                    let built = pipeline.is_some();
                    self.pipelines.push((shader.clone(), pipeline));
                    built
                }
            };
            let drawn = built && *quads <= room;
            if drawn {
                room -= quads;
            }
            shader.set_drawn(drawn);
        }
    }

    /// Takes the batches the game set into this frame (none on the title screen); `vanilla`
    /// gives the vanilla blocks as `core_lines::uniforms` lays them out, when any is drawn.
    pub(super) fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        title: bool,
        vanilla: impl FnOnce() -> [u8; 1024],
    ) {
        let mut batches = std::mem::take(&mut self.batches);
        if title {
            batches.clear();
        }
        let vanilla = if batches.is_empty() {
            [0; 1024]
        } else {
            vanilla()
        };
        self.prepare(device, queue, &batches, &vanilla);
    }

    /// This frame's batches, for the shaders [`Self::decide`] drew: their uniforms and records
    /// uploaded. `vanilla` is the four vanilla blocks.
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batches: &[ShaderBatch],
        vanilla: &[u8],
    ) {
        self.draws.clear();
        // What `decide` granted: never past the budget, whatever the batches hold.
        let mut room = self.budget;
        let mut vertices: Vec<ShaderVertex> = Vec::new();
        for batch in batches {
            let Some(index) = self
                .pipelines
                .iter()
                .position(|(s, p)| Arc::ptr_eq(s, &batch.shader) && p.is_some())
            else {
                continue;
            };
            let Some(pipeline) = &self.pipelines[index].1 else {
                continue;
            };
            let mut data = vec![0u8; OTHER_OFFSET as usize + 256 * pipeline.other.len()];
            let n = vanilla.len().min(OTHER_OFFSET as usize);
            data[..n].copy_from_slice(&vanilla[..n]);
            for (i, other) in pipeline.other.iter().enumerate() {
                let at = OTHER_OFFSET as usize + 256 * i;
                match other {
                    Other::Knobs => data[at..at + GPU_BLOCK_SIZE].copy_from_slice(&batch.block),
                    // An expression that failed, or a uniform the batch lacks, reads 0.
                    Other::Uniform(name) => {
                        let value = batch
                            .uniforms
                            .iter()
                            .find(|(n, _)| n == name)
                            .map_or(0.0, |(_, v)| *v);
                        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
            queue.write_buffer(&pipeline.uniform, 0, &data);
            let quads = batch.vertices.len() / 4;
            if quads == 0 || quads > room {
                continue;
            }
            room -= quads;
            self.draws.push(Draw {
                pipeline: index,
                first: (vertices.len() / 4) as u32,
                quads: quads as u32,
            });
            vertices.extend_from_slice(&batch.vertices[..quads * 4]);
        }
        if vertices.is_empty() {
            return;
        }
        let needed = std::mem::size_of_val(vertices.as_slice()) as u64;
        if self.vertices.size() < needed {
            let size = needed
                .next_power_of_two()
                .min(device.limits().max_buffer_size);
            self.vertices = vertex_buffer(device, size);
        }
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
    }

    /// Whether this frame has batches to draw.
    fn drawing(&self) -> bool {
        !self.draws.is_empty()
    }

    /// Draw this frame's batches in the world pass with the renderer's quad indices (`indices`,
    /// `index_quads` quads: two triangles over each four corners), which stay bound. The caller
    /// then sets its own pipeline, vertex buffer and group 0 again.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, indices: &wgpu::Buffer, index_quads: u32) {
        if index_quads == 0 {
            return;
        }
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        for draw in &self.draws {
            let Some(pipeline) = &self.pipelines[draw.pipeline].1 else {
                continue;
            };
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &pipeline.group, &[]);
            // The type's own records from its first: `gl_VertexIndex` counts from 0 in each
            // type, as a pack's shader spreads an `area`'s copies by it.
            let first = u64::from(draw.first) * QUAD_BYTES;
            pass.set_vertex_buffer(0, self.vertices.slice(first..));
            // More quads than indices: the same indices over the type's next corners.
            let mut done = 0;
            while done < draw.quads {
                let n = (draw.quads - done).min(index_quads);
                pass.draw_indexed(0..n * 6, (done * 4) as i32, 0..1);
                done += n;
            }
        }
    }

    /// The pipeline for `shader` in this renderer's frame, or `None` (logged) when it can't
    /// draw there.
    fn pipeline(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
        shader: &GpuShader,
    ) -> Option<Pipeline> {
        let (format, linear) = self.target;
        // A pack's GLSL writes vanilla's gamma: a frame of linear light keeps the type's quads,
        // as `Renderer::set_lines_shader` keeps the built-in outline.
        if linear {
            log::info!(
                "custom particle shader {} is not used on a frame of linear light",
                shader.id
            );
            return None;
        }
        self.build(device, queue, format, lightmap, shader)
    }

    fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        lightmap: (&wgpu::TextureView, &wgpu::Sampler),
        shader: &GpuShader,
    ) -> Option<Pipeline> {
        let program = &shader.program;
        let Some(other) = program
            .other
            .iter()
            .map(|name| {
                if name == GPU_BLOCK {
                    Some(Other::Knobs)
                } else if shader.uniforms.iter().any(|(n, _)| n == name) {
                    Some(Other::Uniform(name.clone()))
                } else {
                    None
                }
            })
            .collect::<Option<Vec<Other>>>()
        else {
            log::warn!(
                "custom particle shader {} is not used: it reads a block or sampler that is \
                 neither polytone:gpu_particle.glsl's nor one of its uniforms",
                shader.id
            );
            return None;
        };
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Polytone GPU particle (vertex)", &program.vertex);
        let fragment = module("Polytone GPU particle (fragment)", &program.fragment);
        let buffer_entry = |binding, size| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        // Vanilla's blocks and Sampler0 to 2 (`core_shader::BLOCKS`, `SAMPLERS`), then the
        // type's own blocks.
        let mut entries: Vec<wgpu::BindGroupLayoutEntry> = (0..4)
            .map(|b| buffer_entry(b, BLOCK_SIZES[b as usize]))
            .collect();
        for (_, texture) in crate::core_shader::SAMPLERS {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: texture,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: texture + 1,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        for (i, o) in other.iter().enumerate() {
            entries.push(buffer_entry(crate::core_shader::other_binding(i), o.size()));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Polytone GPU particle"),
            entries: &entries,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Polytone GPU particle uniforms"),
            size: OTHER_OFFSET + 256 * other.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let own = shader.texture.as_ref().map(|(w, h, pixels)| {
            texture(device, queue, *w, *h, pixels).create_view(&Default::default())
        });
        let buffer = |binding, offset, size| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &uniform,
                offset,
                size: wgpu::BufferSize::new(size),
            }),
        };
        let mut resources: Vec<wgpu::BindGroupEntry> = (0..4)
            .map(|i| buffer(i as u32, BLOCK_OFFSETS[i], BLOCK_SIZES[i]))
            .collect();
        let [(_, sampler0), (_, sampler1), (_, sampler2)] = crate::core_shader::SAMPLERS;
        for (binding, view, sampler) in [
            (sampler0, own.as_ref().unwrap_or(&self.blank), &self.sampler),
            (sampler1, &self.heightmap_view, &self.sampler),
            (sampler2, lightmap.0, lightmap.1),
        ] {
            resources.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            resources.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        for (i, o) in other.iter().enumerate() {
            resources.push(buffer(
                crate::core_shader::other_binding(i),
                OTHER_OFFSET + 256 * i as u64,
                o.size(),
            ));
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Polytone GPU particle"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Polytone GPU particle"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // Wiki "GPU Particles": the two translucent types blend (the additive one src alpha
        // and one); anything else is alpha cutout, the shader's own discard, with depth write.
        let (blend, depth_write) = match shader.render_type {
            RenderType::Translucent => (Some(wgpu::BlendState::ALPHA_BLENDING), false),
            RenderType::AdditiveTranslucent => (
                Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::OVER,
                }),
                false,
            ),
            _ => (None, true),
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Polytone GPU particle"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ShaderVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRIBUTES,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    // docs/hdr-passes.md: the frame's alpha (HDR output's marks) is left alone.
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("custom particle shader {} is not used: {e}", shader.id);
            return None;
        }
        Some(Pipeline {
            pipeline,
            group,
            uniform,
            other,
        })
    }
}

/// The game's side: the verdict it asks for before building the frame's quads, the heightmap
/// and batches it hands over.
impl GpuParticleRenderer for super::Renderer {
    fn polytone_fallback_quads(&self) -> usize {
        fallback_quads(self.device.limits().max_buffer_size)
    }

    fn decide_polytone_shaders(&mut self, requests: &[ShaderRequest]) {
        let frame = &self.output.frame;
        self.core_particles.decide(
            &self.device,
            &self.queue,
            (frame.format, frame.linear_light),
            (&self.lightmap_view, &self.lightmap_sampler),
            requests,
        );
    }

    fn set_polytone_heightmap(&mut self, surface: &Surface) {
        self.core_particles.set_heightmap(&self.queue, surface);
    }

    fn set_polytone_batches(&mut self, batches: Vec<ShaderBatch>) {
        self.core_particles.batches = batches;
    }
}

/// The world frame's preparing and drawing of the batches.
impl super::Renderer {
    /// Whether this frame has GPU particle batches to draw.
    pub(super) fn core_particles_drawing(&self) -> bool {
        self.core_particles.drawing()
    }

    /// Draws this frame's batches into the world pass (the caller then sets its pipeline,
    /// vertex buffer and groups again).
    pub(super) fn draw_core_particles(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.core_particles
            .draw(pass, &self.indices, self.index_quads);
    }
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Polytone GPU particle records"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> wgpu::Texture {
    wgpu::util::DeviceExt::create_texture_with_data(
        device,
        queue,
        &wgpu::TextureDescriptor {
            label: Some("Polytone GPU particle texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        pixels,
    )
}

#[cfg(test)]
#[path = "core_particles_tests.rs"]
mod tests;
