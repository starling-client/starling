//! Native mods' world primitives (`native_world_primitives`, docs/modding/src/how-to/native-mods.md): lines, boxes,
//! beams and text a mod hands the game each frame, drawn here with Sunrise's own pipelines in
//! the world pass at the stages they were given (`render_level_stage`), the last one after the
//! entities and before the first-person hand. They draw
//! into the world's targets, so they take the world's reversed depth, its draw resolution and
//! TAA's jitter; a mod never touches wgpu for them.

use super::Vertex;
use glam::Mat4;

/// Initial per-view line and quad pre-reservation hint; buffers grow dynamically as needed.
pub const DEFAULT_CAPACITY: u32 = 4096;

/// The pass-bearing standard world stages a frame draws primitives at, in frame order
/// (`RenderLevelStage` pass stages, plus `STAGE_CUSTOM_GEOMETRY` at the end of the world pass).
pub const STAGES: usize = 7;
pub const STAGE_AFTER_OPAQUE_BLOCKS: usize = 0;
pub const STAGE_AFTER_OPAQUE_FEATURES: usize = 1;
pub const STAGE_AFTER_TRANSLUCENT_FEATURES: usize = 2;
pub const STAGE_AFTER_TRANSLUCENT_BLOCKS: usize = 3;
pub const STAGE_AFTER_TRANSLUCENT_PARTICLES: usize = 4;
pub const STAGE_AFTER_WEATHER: usize = 5;
pub const STAGE_CUSTOM_GEOMETRY: usize = 6;

/// The three OIT stages (`DepthBounds`, `Transmittance`, `Accumulate`).
pub const OIT_STAGES: usize = 3;

/// world_primitives.wgsl after what it shares with the frame's other shaders (`frame_stored`).
pub(super) fn shader() -> String {
    include_str!("../hdr_light.wgsl").to_owned() + include_str!("world_primitives.wgsl")
}

/// world_primitives.wgsl combined with `oit.wgsl` and the six OIT fragment entry points.
pub(super) fn oit_shader() -> String {
    include_str!("../hdr_light.wgsl").to_owned()
        + include_str!("../oit.wgsl")
        + include_str!("world_primitives.wgsl")
        + r#"
@fragment fn fs_oit_quad_bounds(in: QuadOut) -> @location(0) vec4<f32> {
    let color = quad_color(in);
    if color.a <= 0.0 { discard; }
    return oit_bounds_fragment(color.a, in.clip);
}
@fragment fn fs_oit_quad_transmittance(in: QuadOut) -> OitCoefficients {
    let color = quad_color(in);
    if color.a <= 0.0 { discard; }
    return oit_transmittance_fragment(color.a, in.clip);
}
@fragment fn fs_oit_quad_accumulate(in: QuadOut) -> @location(0) vec4<f32> {
    let color = quad_color(in);
    if color.a <= 0.0 { discard; }
    return oit_accumulate_fragment(color, in.clip);
}
@fragment fn fs_oit_line_bounds(in: LineOut) -> @location(0) vec4<f32> {
    let color = in.color;
    if color.a <= 0.0 { discard; }
    return oit_bounds_fragment(color.a, in.clip);
}
@fragment fn fs_oit_line_transmittance(in: LineOut) -> OitCoefficients {
    let color = in.color;
    if color.a <= 0.0 { discard; }
    return oit_transmittance_fragment(color.a, in.clip);
}
@fragment fn fs_oit_line_accumulate(in: LineOut) -> @location(0) vec4<f32> {
    let color = in.color;
    if color.a <= 0.0 { discard; }
    return oit_accumulate_fragment(color, in.clip);
}
"#
}

/// A frame's OIT primitives across the three OIT stages (`DepthBounds`, `Transmittance`,
/// `Accumulate`).
#[derive(Clone, Copy, Debug, Default)]
pub struct OitBatch<'a> {
    pub quads: &'a [Vertex],
    pub quads_through: &'a [Vertex],
    pub lines: &'a [[f32; 8]],
    pub lines_through: &'a [[f32; 8]],
    pub stage_ends: [[u32; 4]; OIT_STAGES - 1],
}

impl OitBatch<'_> {
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
            && self.quads_through.is_empty()
            && self.lines.is_empty()
            && self.lines_through.is_empty()
    }
}

/// Batch ranges in the shared frame upload, relative to each stream. Plain primitive ranges
/// are in quads/lines, not bytes. `pipeline_runs` is a range of run indices in `ModPipelines`
/// (standard or OIT runs, matching the batch's stage).
#[derive(Clone, Debug, Default)]
pub struct DrawRanges {
    pub primitives: [std::ops::Range<u32>; 4],
    pub pipeline_runs: std::ops::Range<u32>,
    pub pipeline_draws: std::ops::Range<u32>,
}

/// Extra frame data produced by `submit_custom_geometry`, debug renderers, and
/// `prepare_render_buffers` (including resolved retained `InstanceGroup`s and world name tags),
/// consumed by `Renderer::draw_frame` before entity/instance/glyph uploads.
#[derive(Default)]
pub struct PreparedHookGeometry {
    pub entities: Vec<Vertex>,
    /// Span starts are relative to `entities`, before the renderer merges the streams.
    pub entity_spans: Vec<super::pack_draw::EntitySpan>,
    pub see_through: Vec<Vertex>,
    pub prepared_draws: Vec<super::PreparedModelDraw>,
    pub text_patches: Vec<crate::font::Patch>,
}

/// Renderer-driven lifecycle for mod world rendering hooks (`submit_custom_geometry`,
/// `prepare_render_buffers`, and `render_level_stage`).
pub trait FrameHooks {
    /// Whether any listener wants the `RenderLevelStage::AfterSky` boundary
    /// (which requires splitting the engine sky pass before buffer preparation).
    fn wants_after_sky(&self) -> bool {
        true
    }
    fn on_custom_geometry(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool);
    /// Fire `Hooks::frame_graph_setup` at the start of level rendering to record data-driven
    /// offscreen passes and transient per-frame targets.
    fn on_frame_graph_setup(&mut self, _sections: &[mod_api::RenderableSection], _oit_on: bool) {}
    /// Take the recorded frame graph for compilation and execution during this world frame.
    fn take_frame_graph(&mut self) -> Option<super::mod_frame_graph::RecordedFrameGraph> {
        None
    }
    fn on_after_sky(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool);
    fn on_prepare_render_buffers(
        &mut self,
        sections: &[mod_api::RenderableSection],
        oit_on: bool,
        out: &mut PreparedHookGeometry,
    );
    fn on_render_stage(
        &mut self,
        stage: mod_api::RenderLevelStage,
        sections: &[mod_api::RenderableSection],
        oit_on: bool,
    ) -> &[DrawRanges];
    fn prepared_primitives(&self) -> Batch<'_>;
    fn on_after_level(&mut self, sections: &[mod_api::RenderableSection], oit_on: bool);
}

/// A frame's primitives, camera-relative, as the game tessellated them: quads of 4 vertices
/// from the shared atlas (plain colours on its white texel, text on its glyphs, beams on the
/// beacon beam sprite) and lines as `gizmos::camera_relative` gives them. `*_through` ones are
/// seen through walls: no depth test.
#[derive(Clone, Copy, Debug, Default)]
pub struct Batch<'a> {
    pub quads: &'a [Vertex],
    pub quads_through: &'a [Vertex],
    pub lines: &'a [[f32; 8]],
    pub lines_through: &'a [[f32; 8]],
    /// Where each stage but the last ends, as counts of quads, quads through walls, lines and
    /// lines through walls: each slice holds its stages in frame order, and what follows the
    /// last end is the last stage's. All zero, everything draws at the last stage.
    pub stage_ends: [[u32; 4]; STAGES - 1],
    /// Primitives routed into the OIT translucent pass stages.
    pub oit: OitBatch<'a>,
    /// The quads mods draw with their own pipelines (`gfx::mod_pipelines`), with their stages.
    pub pipelines: super::mod_pipelines::Batch<'a>,
    /// Per-frame shader-pack uniform values supplied by mods via `WorldDrawContext::set_pack_uniform`.
    pub pack_uniforms: &'a [(&'static str, mod_api::PackUniformValue)],
}

impl Batch<'_> {
    pub fn is_empty(&self) -> bool {
        self.pipelines.runs.is_empty()
            && self.pipelines.oit_runs.is_empty()
            && self.pipelines.draws.is_empty()
            && self.pipelines.oit_draws.is_empty()
            && self.quads.is_empty()
            && self.quads_through.is_empty()
            && self.lines.is_empty()
            && self.lines_through.is_empty()
            && self.oit.is_empty()
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    viewport: [f32; 4],
}

struct PrimitivePipelines {
    /// Depth tested, then seen through walls.
    quad_pipelines: [wgpu::RenderPipeline; 2],
    line_pipelines: [wgpu::RenderPipeline; 2],
}

/// The pipelines and buffers that draw a [`Batch`].
pub struct WorldPrimitives {
    pipelines_standard: PrimitivePipelines,
    pack_pipelines: Option<((wgpu::TextureFormat, bool, bool), PrimitivePipelines)>,
    /// OIT pipelines for `DepthBounds`, `Transmittance`, and `Accumulate`, built when OIT is enabled.
    oit_pipelines: Option<[PrimitivePipelines; OIT_STAGES]>,
    layout: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    sampler: wgpu::Sampler,
    quads: VertexPages,
    lines: VertexPages,
    oit_quads: VertexPages,
    oit_lines: VertexPages,
    indices: wgpu::Buffer,
    index_quads: u32,
    group: Option<AtlasGroup>,
    quad_vertices: Vec<Vertex>,
    line_instances: Vec<[f32; 8]>,
    oit_quad_vertices: Vec<Vertex>,
    oit_line_instances: Vec<[f32; 8]>,
    index_data: Vec<u32>,
    /// Quads depth tested, quads through walls, lines depth tested, lines through walls.
    counts: [u32; 4],
    /// The batch's `stage_ends`, clamped to `counts`.
    stage_ends: [[u32; 4]; STAGES - 1],
    /// OIT quads depth tested, quads through walls, lines depth tested, lines through walls.
    oit_counts: [u32; 4],
    /// The batch's OIT `stage_ends`, clamped to `oit_counts`.
    oit_stage_ends: [[u32; 4]; OIT_STAGES - 1],
    /// Mods' own pipelines and their quads, drawn after each stage's primitives.
    pub(super) pipelines: super::mod_pipelines::ModPipelines,
    pack_uniforms: Vec<(&'static str, mod_api::PackUniformValue)>,
    /// Pipeline slots currently bound to an active shader pack's translated mod passes,
    /// refreshed per frame without allocating so fallback engine passes never double-draw them.
    bound_mod_slots: Vec<usize>,
}

struct AtlasGroup {
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// Copies bytes into a buffer as part of the frame (the renderer's staging uploads, or the
/// queue in a test). The callback consumes the bytes before returning; they may be reused.
pub type Upload<'u> = &'u mut dyn FnMut(&wgpu::Buffer, &[u8]);

/// Logical record ranges stay contiguous on the CPU; each GPU page starts at record zero.
/// Four vertices are a quad record. A line's instance is one record.
#[derive(Default)]
pub(super) struct VertexPages {
    pub(super) buffers: Vec<wgpu::Buffer>,
    records: u32,
}

impl VertexPages {
    pub(super) fn limit(bytes: u64, stride: u64) -> u32 {
        // Even a very large GPU allocation must fit one draw's u32 ranges (six indices per quad).
        (bytes / stride).min(u64::from(u32::MAX / 6)) as u32
    }

    fn reserve_page(
        &mut self,
        device: &wgpu::Device,
        label: &'static str,
        page: usize,
        bytes: u64,
    ) {
        let limit = super::upload::legal_buffer_limit(device);
        if self
            .buffers
            .get(page)
            .is_some_and(|buffer| buffer.size() >= bytes && buffer.size() <= limit)
        {
            return;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: super::upload::legal_buffer_size(bytes, limit),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if page == self.buffers.len() {
            self.buffers.push(buffer);
        } else {
            self.buffers[page] = buffer;
        }
    }

    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        label: &'static str,
        bytes: &[u8],
        stride: u64,
        first_page: usize,
    ) -> usize {
        self.records = Self::limit(super::upload::legal_buffer_limit(device), stride);
        if bytes.is_empty() || self.records == 0 {
            return first_page;
        }
        let page_bytes = self.records as usize * stride as usize;
        let mut page = first_page;
        for chunk in bytes.chunks(page_bytes) {
            self.reserve_page(device, label, page, chunk.len() as u64);
            upload(&self.buffers[page], chunk);
            page += 1;
        }
        page
    }

    pub(super) fn index_quads(&self, quads: u32) -> u32 {
        quads.min(self.records)
    }

    pub(super) fn ranges(
        &self,
        first_page: usize,
        range: std::ops::Range<u32>,
    ) -> impl Iterator<Item = (&wgpu::Buffer, std::ops::Range<u32>)> + '_ {
        let mut cursor = range.start;
        std::iter::from_fn(move || {
            if cursor >= range.end {
                return None;
            }
            let page = cursor / self.records;
            let local = cursor % self.records;
            let count = (range.end - cursor).min(self.records - local);
            cursor += count;
            Some((
                &self.buffers[first_page + page as usize],
                local..local + count,
            ))
        })
    }
}

const QUAD_STRIDE: u64 = 4 * std::mem::size_of::<Vertex>() as u64;
const LINE_STRIDE: u64 = std::mem::size_of::<[f32; 8]>() as u64;

impl WorldPrimitives {
    /// Pipelines drawing into `color_format` over a reversed Depth32Float depth, decoding their
    /// colours where the frame holds linear light (`linear`).
    pub fn new(device: &wgpu::Device, (color_format, linear): (wgpu::TextureFormat, bool)) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("World primitives"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipelines_standard = Self::standard_pipelines(
            device,
            &layout,
            color_format,
            linear,
            false,
            wgpu::ColorWrites::ALL,
        );
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            pipelines_standard,
            pack_pipelines: None,
            oit_pipelines: None,
            globals: buffer(
                "World primitive globals",
                std::mem::size_of::<Globals>() as u64,
                wgpu::BufferUsages::UNIFORM,
            ),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("World primitives"),
                ..Default::default()
            }),
            layout,
            quads: VertexPages::default(),
            lines: VertexPages::default(),
            oit_quads: VertexPages::default(),
            oit_lines: VertexPages::default(),
            indices: buffer("World primitive indices", 0, wgpu::BufferUsages::INDEX),
            index_quads: 0,
            group: None,
            quad_vertices: Vec::new(),
            line_instances: Vec::new(),
            oit_quad_vertices: Vec::new(),
            oit_line_instances: Vec::new(),
            index_data: Vec::new(),
            counts: [0; 4],
            stage_ends: [[0; 4]; STAGES - 1],
            oit_counts: [0; 4],
            oit_stage_ends: [[0; 4]; OIT_STAGES - 1],
            pipelines: super::mod_pipelines::ModPipelines::new(device, (color_format, linear)),
            pack_uniforms: Vec::new(),
            bound_mod_slots: Vec::new(),
        }
    }

    /// Pipelines for the classic shader pack's active colortex0 and depthtex0 attachments.
    pub(super) fn prepare_pack_pass(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        rgb: bool,
        linear: bool,
    ) {
        let key = (format, rgb, linear);
        if self
            .pack_pipelines
            .as_ref()
            .is_none_or(|(previous, _)| *previous != key)
        {
            let mask = if rgb {
                wgpu::ColorWrites::COLOR
            } else {
                wgpu::ColorWrites::ALL
            };
            self.pack_pipelines = Some((
                key,
                Self::standard_pipelines(device, &self.layout, format, linear, true, mask),
            ));
        }
        self.pipelines.ensure_pack_pipelines(device, format, rgb);
    }

    fn standard_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        linear: bool,
        pack: bool,
        write_mask: wgpu::ColorWrites,
    ) -> PrimitivePipelines {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("World primitives"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("World primitives"),
            bind_group_layouts: &[Some(layout)],
            ..Default::default()
        });
        let quad_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint32];
        let line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let quad_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &quad_attributes,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        })];
        let pipeline = |label: &str,
                        entry: [&str; 2],
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology,
                        through: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry[0]),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("pack_coordinates", if pack { 1.0 } else { 0.0 })],
                        ..Default::default()
                    },
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    // Seen from inside or behind as well: a box around the camera, a beam.
                    cull_mode: None,
                    ..Default::default()
                },
                // Blended over what is drawn, writing no depth: a translucent box doesn't hide
                // the primitives behind it. Reversed depth: nearer is greater.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if through {
                        wgpu::CompareFunction::Always
                    } else if pack {
                        wgpu::CompareFunction::LessEqual
                    } else {
                        wgpu::CompareFunction::GreaterEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry[1]),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &super::linear_blend::frame_constants(color_format, linear),
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let quads = ["vs_quad", "fs_quad"];
        let lines = ["vs_line", "fs_line"];
        let list = wgpu::PrimitiveTopology::TriangleList;
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let quad_pipelines = [
            pipeline("World primitives", quads, &quad_buffers, list, false),
            pipeline(
                "World primitives through walls",
                quads,
                &quad_buffers,
                list,
                true,
            ),
        ];
        let line_pipelines = [
            pipeline("World primitive lines", lines, &line_buffers, strip, false),
            pipeline(
                "World primitive lines through walls",
                lines,
                &line_buffers,
                strip,
                true,
            ),
        ];
        PrimitivePipelines {
            quad_pipelines,
            line_pipelines,
        }
    }

    /// Compiles the OIT primitive pipelines for all three OIT stages, binding `oit_layouts` at
    /// group 2 (leaving group 1 empty, matching `oit.rs` / `oit_features.rs`).
    pub fn enable_oit(&mut self, device: &wgpu::Device, oit_layouts: &[wgpu::BindGroupLayout; 4]) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("World primitives OIT"),
            source: wgpu::ShaderSource::Wgsl(oit_shader().into()),
        });
        let quad_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint32];
        let line_attributes =
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Unorm8x4,3=>Float32];
        let quad_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &quad_attributes,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        })];
        let stages = [
            (super::oit::Stage::DepthBounds, "bounds"),
            (super::oit::Stage::Transmittance, "transmittance"),
            (super::oit::Stage::Accumulate, "accumulate"),
        ];
        let list = wgpu::PrimitiveTopology::TriangleList;
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let pipelines = stages.map(|(stage, suffix)| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("World primitives OIT"),
                bind_group_layouts: &[Some(&self.layout), None, Some(&oit_layouts[stage as usize])],
                ..Default::default()
            });
            let targets = super::oit::stage_targets(stage);
            let make = |label: &str,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        topology,
                        through: bool| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vertex),
                        compilation_options: Default::default(),
                        buffers,
                    },
                    primitive: wgpu::PrimitiveState {
                        topology,
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(if through {
                            wgpu::CompareFunction::Always
                        } else {
                            wgpu::CompareFunction::GreaterEqual
                        }),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        compilation_options: Default::default(),
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let quad_frag = format!("fs_oit_quad_{suffix}");
            let line_frag = format!("fs_oit_line_{suffix}");
            PrimitivePipelines {
                quad_pipelines: [
                    make(
                        &format!("World primitives OIT quads {suffix}"),
                        "vs_quad",
                        &quad_frag,
                        &quad_buffers,
                        list,
                        false,
                    ),
                    make(
                        &format!("World primitives OIT quads through walls {suffix}"),
                        "vs_quad",
                        &quad_frag,
                        &quad_buffers,
                        list,
                        true,
                    ),
                ],
                line_pipelines: [
                    make(
                        &format!("World primitives OIT lines {suffix}"),
                        "vs_line",
                        &line_frag,
                        &line_buffers,
                        strip,
                        false,
                    ),
                    make(
                        &format!("World primitives OIT lines through walls {suffix}"),
                        "vs_line",
                        &line_frag,
                        &line_buffers,
                        strip,
                        true,
                    ),
                ],
            }
        });
        self.oit_pipelines = Some(pipelines);
    }

    /// Reserves CPU and GPU geometry storage for `quads` and `lines`, total across both depth
    /// modes. Call before preparing a frame, at setup to avoid growth during submission. Only
    /// index growth uploads the full index array; prepare geometry again after buffer growth.
    pub fn reserve(&mut self, device: &wgpu::Device, upload: Upload<'_>, quads: u32, lines: u32) {
        self.quad_vertices
            .reserve((quads as usize * 4).saturating_sub(self.quad_vertices.len()));
        self.line_instances
            .reserve((lines as usize).saturating_sub(self.line_instances.len()));
        let max = super::upload::legal_buffer_limit(device);
        let quads = quads.min(VertexPages::limit(max, QUAD_STRIDE));
        let lines = lines.min(VertexPages::limit(max, LINE_STRIDE));
        if quads > 0 {
            self.quads.reserve_page(
                device,
                "World primitive quads",
                0,
                u64::from(quads) * QUAD_STRIDE,
            );
        }
        if lines > 0 {
            self.lines.reserve_page(
                device,
                "World primitive lines",
                0,
                u64::from(lines) * LINE_STRIDE,
            );
        }
        self.reserve_indices(device, upload, quads);
    }

    /// Reserves the shared quad indices for `quads` quads; growth uploads the full array.
    fn reserve_indices(&mut self, device: &wgpu::Device, upload: Upload<'_>, quads: u32) {
        if quads > self.index_quads {
            let max = super::upload::legal_buffer_limit(device);
            let capacity = quads.next_power_of_two().min(VertexPages::limit(max, 24));
            self.index_data.extend(
                (self.index_quads..capacity).flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i)),
            );
            self.index_quads = capacity;
            let bytes: &[u8] = bytemuck::cast_slice(&self.index_data);
            if self.indices.size() < bytes.len() as u64 {
                self.indices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("World primitives"),
                    size: super::upload::legal_buffer_size(bytes.len() as u64, max),
                    usage: self.indices.usage(),
                    mapped_at_creation: false,
                });
            }
            upload(&self.indices, bytes);
        }
    }

    fn draw_pages(
        pass: &mut wgpu::RenderPass<'_>,
        indices: &wgpu::Buffer,
        (quad_pipeline, line_pipeline): (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        (quads, quad_page, quad_range): (&VertexPages, usize, std::ops::Range<u32>),
        (lines, line_page, line_range): (&VertexPages, usize, std::ops::Range<u32>),
    ) {
        if !quad_range.is_empty() {
            pass.set_pipeline(quad_pipeline);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            for (buffer, range) in quads.ranges(quad_page, quad_range) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(range.start * 6..range.end * 6, 0, 0..1);
            }
        }
        if !line_range.is_empty() {
            pass.set_pipeline(line_pipeline);
            for (buffer, range) in lines.ranges(line_page, line_range) {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..4, range);
            }
        }
    }

    /// Drop cached bind groups of released mod storage buffers (see
    /// [`mod_pipelines::ModPipelines::forget_storage`]).
    pub fn forget_storage(&mut self, released: &[u64]) {
        self.pipelines.forget_storage(released);
    }

    /// Forget the warnings about released mod render target textures (see
    /// [`mod_pipelines::ModPipelines::forget_render_targets`]).
    pub fn forget_render_targets(&mut self, released: &[u64]) {
        self.pipelines.forget_render_targets(released);
    }

    /// Uploads this frame's `batch`, drawn with `view_proj` (camera-relative world to clip,
    /// jittered as the world is) into a frame `viewport` pixels big, sampling `atlas`. When
    /// `vanilla_blocks` is `Some`, mod GLSL pipelines bind those `core_lines::uniforms` blocks
    /// (`Globals`, `Projection`, `DynamicTransforms`, `Fog`); when `None`, fallback blocks are
    /// synthesized from `(view_proj, viewport, batch.pipelines.time)`. Whether there is anything
    /// to draw. CPU and GPU storage grows at its high-water mark; subsequent frames within that
    /// capacity reuse it. Bindings are rebuilt only when the atlas view changes.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        upload: Upload<'_>,
        batch: &Batch<'_>,
        view_proj: Mat4,
        viewport: [u32; 2],
        atlas: &wgpu::TextureView,
        vanilla_blocks: Option<&[u8; 1024]>,
    ) -> bool {
        let quads = [batch.quads, batch.quads_through].map(|q| (q.len() / 4) as u32);
        let lines = [batch.lines, batch.lines_through].map(|l| l.len() as u32);
        self.counts = [quads[0], quads[1], lines[0], lines[1]];
        self.stage_ends = clamped_ends(batch.stage_ends, self.counts);
        let oit_quads = [batch.oit.quads, batch.oit.quads_through].map(|q| (q.len() / 4) as u32);
        let oit_lines = [batch.oit.lines, batch.oit.lines_through].map(|l| l.len() as u32);
        self.oit_counts = [oit_quads[0], oit_quads[1], oit_lines[0], oit_lines[1]];
        self.oit_stage_ends = clamped_ends(batch.oit.stage_ends, self.oit_counts);
        self.pack_uniforms.clear();
        self.pack_uniforms.extend_from_slice(batch.pack_uniforms);
        // Before the empty check, which counts the pipeline runs this queues; the pipelines
        // themselves were built with the renderer (`Renderer::build_mod_pipelines`).
        let pipeline_quads = self.pipelines.prepare(
            device,
            upload,
            &batch.pipelines,
            view_proj,
            viewport,
            vanilla_blocks,
        );
        if batch.is_empty() {
            return false;
        }
        self.quad_vertices.clear();
        for q in [batch.quads, batch.quads_through] {
            self.quad_vertices.extend_from_slice(&q[..q.len() / 4 * 4]);
        }
        self.line_instances.clear();
        self.line_instances.extend_from_slice(batch.lines);
        self.line_instances.extend_from_slice(batch.lines_through);
        self.oit_quad_vertices.clear();
        for q in [batch.oit.quads, batch.oit.quads_through] {
            self.oit_quad_vertices
                .extend_from_slice(&q[..q.len() / 4 * 4]);
        }
        self.oit_line_instances.clear();
        self.oit_line_instances.extend_from_slice(batch.oit.lines);
        self.oit_line_instances
            .extend_from_slice(batch.oit.lines_through);
        self.quads.upload(
            device,
            upload,
            "World primitive quads",
            bytemuck::cast_slice(&self.quad_vertices),
            QUAD_STRIDE,
            0,
        );
        self.lines.upload(
            device,
            upload,
            "World primitive lines",
            bytemuck::cast_slice(&self.line_instances),
            LINE_STRIDE,
            0,
        );
        self.oit_quads.upload(
            device,
            upload,
            "World OIT quads",
            bytemuck::cast_slice(&self.oit_quad_vertices),
            QUAD_STRIDE,
            0,
        );
        self.oit_lines.upload(
            device,
            upload,
            "World OIT lines",
            bytemuck::cast_slice(&self.oit_line_instances),
            LINE_STRIDE,
            0,
        );
        self.reserve_indices(
            device,
            upload,
            self.quads
                .index_quads(quads[0] + quads[1])
                .max(self.oit_quads.index_quads(oit_quads[0] + oit_quads[1]))
                .max(pipeline_quads),
        );
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            viewport: [viewport[0] as f32, viewport[1] as f32, 0.0, 0.0],
        };
        upload(&self.globals, bytemuck::bytes_of(&globals));
        if self.group.as_ref().is_none_or(|g| g.view != *atlas) {
            self.group = Some(AtlasGroup {
                view: atlas.clone(),
                group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("World primitives"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.globals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(atlas),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                }),
            });
        }
        true
    }

    /// Refresh OIT mod-pipeline bind groups (`DepthBoundsSampler`, `Coeff0`, `Coeff1`) when OIT
    /// targets change generation.
    pub(super) fn prepare_oit_mod_pipelines(
        &mut self,
        device: &wgpu::Device,
        samples: Option<super::oit::CloudSamples<'_>>,
    ) {
        self.pipelines.prepare_oit_groups(device, samples);
    }

    /// Encode a prepared command. No buffer allocation or upload is possible from this path.
    pub(super) fn draw_prepared(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        oit: Option<super::oit::Stage>,
        draws: &[DrawRanges],
    ) -> bool {
        let mut drew = match oit {
            Some(stage) => self.oit_pipelines.as_ref().is_some_and(|pipelines| {
                self.draw_prepared_ranges(pass, &pipelines[stage as usize], true, draws)
            }),
            None => self.draw_prepared_ranges(pass, &self.pipelines_standard, false, draws),
        };
        let has_pipeline_runs = draws.iter().any(|d| !d.pipeline_runs.is_empty());
        if has_pipeline_runs {
            let ranges = draws
                .iter()
                .filter(|d| !d.pipeline_runs.is_empty())
                .map(|d| d.pipeline_runs.clone());
            match oit {
                Some(stage) => self
                    .pipelines
                    .draw_prepared_oit(pass, stage, &self.indices, ranges),
                None => self
                    .pipelines
                    .draw_prepared_runs(pass, &self.indices, ranges),
            }
            drew = true;
        }
        let has_pipeline_draws = draws.iter().any(|d| !d.pipeline_draws.is_empty());
        if has_pipeline_draws {
            let ranges: Vec<std::ops::Range<u32>> = draws
                .iter()
                .filter(|d| !d.pipeline_draws.is_empty())
                .map(|d| d.pipeline_draws.clone())
                .collect();
            match oit {
                Some(stage) => {
                    self.pipelines
                        .draw_prepared_oit_draws(pass, stage, &self.indices, &ranges)
                }
                None => self
                    .pipelines
                    .draw_prepared_draws(pass, &self.indices, &ranges),
            }
            drew = true;
        }
        drew
    }

    pub(super) fn draw_pack_prepared(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        draws: &[DrawRanges],
    ) -> bool {
        let Some((_, pipelines)) = &self.pack_pipelines else {
            return false;
        };
        if draws.is_empty() || self.group.is_none() {
            return false;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Prepared classic pack world stage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        self.draw_prepared_ranges(&mut pass, pipelines, false, draws)
    }

    /// Per-frame shader-pack uniform values uploaded from the latest [`Batch`].
    pub(super) fn pack_uniforms(&self) -> &[(&'static str, mod_api::PackUniformValue)] {
        &self.pack_uniforms
    }

    /// Cache the active shader pack's bound mod pipeline slots for this frame (reusing capacity).
    pub(super) fn set_bound_mod_slots(&mut self, slots: &[usize]) {
        self.bound_mod_slots.clear();
        self.bound_mod_slots.extend_from_slice(slots);
    }

    /// Append uploaded vertex chunks for mod pipelines bound to translated shader-pack passes.
    pub(super) fn pack_bound_chunks_into(
        &self,
        bound_slots: &[usize],
        out: &mut Vec<super::pack_post::ModDrawChunk>,
    ) {
        self.pipelines
            .pack_bound_chunks_into(&self.indices, bound_slots, out);
    }

    /// Draws deferred custom geometry (`STAGE_CUSTOM_GEOMETRY`, including unbound `ModPipelines`
    /// runs) into a classic shader pack's active `colortex0` + `depthtex0` attachments.
    pub(super) fn draw_pack_custom(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> bool {
        self.draw_pack_stages(
            encoder,
            color,
            depth,
            STAGE_CUSTOM_GEOMETRY,
            STAGE_CUSTOM_GEOMETRY,
        )
    }

    /// Draws prepared primitive and unbound `ModPipelines` runs in `first..=last` into a
    /// classic shader pack's active `colortex0` + `depthtex0` attachments.
    pub(super) fn draw_pack_stages(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        first: usize,
        last: usize,
    ) -> bool {
        let Some((_, pipelines)) = &self.pack_pipelines else {
            return false;
        };
        let Some(AtlasGroup { group, .. }) = &self.group else {
            return false;
        };
        let [quads, quads_through, lines, lines_through] =
            spans(self.counts, &self.stage_ends, first, last);
        let has_primitives = !(quads.is_empty()
            && quads_through.is_empty()
            && lines.is_empty()
            && lines_through.is_empty());
        let has_pipelines = self
            .pipelines
            .has_unbound_runs(first, last, &self.bound_mod_slots);
        if !has_primitives && !has_pipelines {
            return false;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Classic pack custom geometry"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if has_primitives {
            pass.set_bind_group(0, group, &[]);
            for (through, (quad_range, line_range)) in
                [(quads, lines), (quads_through, lines_through)]
                    .into_iter()
                    .enumerate()
            {
                Self::draw_pages(
                    &mut pass,
                    &self.indices,
                    (
                        &pipelines.quad_pipelines[through],
                        &pipelines.line_pipelines[through],
                    ),
                    (&self.quads, 0, quad_range),
                    (&self.lines, 0, line_range),
                );
            }
        }
        if has_pipelines {
            self.pipelines
                .draw_pack(&mut pass, &self.indices, first, last, &self.bound_mod_slots);
        }
        true
    }

    fn draw_prepared_ranges(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &PrimitivePipelines,
        oit: bool,
        draws: &[DrawRanges],
    ) -> bool {
        let Some(AtlasGroup { group, .. }) = &self.group else {
            return false;
        };
        let (quads, lines, counts) = if oit {
            (&self.oit_quads, &self.oit_lines, self.oit_counts)
        } else {
            (&self.quads, &self.lines, self.counts)
        };
        let PrimitivePipelines {
            quad_pipelines,
            line_pipelines,
        } = pipelines;
        let mut drew = false;
        for draw in draws {
            let [q, qt, l, lt] = draw.primitives.clone();
            if !(q.is_empty() && qt.is_empty() && l.is_empty() && lt.is_empty()) {
                drew = true;
                pass.set_bind_group(0, group, &[]);
                for (i, (qr, lr)) in [
                    (q, l),
                    (
                        counts[0] + qt.start..counts[0] + qt.end,
                        counts[2] + lt.start..counts[2] + lt.end,
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    Self::draw_pages(
                        pass,
                        &self.indices,
                        (&quad_pipelines[i], &line_pipelines[i]),
                        (quads, 0, qr),
                        (lines, 0, lr),
                    );
                }
            }
        }
        drew
    }

    /// Draws the OIT primitives and OIT mod-pipeline quads for `stage` into the active OIT pass
    /// (group 2 is already bound by `oit::Oit::run`). Leaves the pass with its own pipeline, bind
    /// group 0 and buffers.
    pub(super) fn draw_oit(&self, pass: &mut wgpu::RenderPass<'_>, stage: super::oit::Stage) {
        if let (Some(oit_pipelines), Some(AtlasGroup { group, .. })) =
            (&self.oit_pipelines, &self.group)
        {
            let stage_index = stage as usize;
            let [quads, quads_through, lines, lines_through] = spans(
                self.oit_counts,
                &self.oit_stage_ends,
                stage_index,
                stage_index,
            );
            if !(quads.is_empty()
                && quads_through.is_empty()
                && lines.is_empty()
                && lines_through.is_empty())
            {
                let stage_pipelines = &oit_pipelines[stage_index];
                pass.set_bind_group(0, group, &[]);
                for (through, (quad_range, line_range)) in
                    [(quads, lines), (quads_through, lines_through)]
                        .into_iter()
                        .enumerate()
                {
                    Self::draw_pages(
                        pass,
                        &self.indices,
                        (
                            &stage_pipelines.quad_pipelines[through],
                            &stage_pipelines.line_pipelines[through],
                        ),
                        (&self.oit_quads, 0, quad_range),
                        (&self.oit_lines, 0, line_range),
                    );
                }
            }
        }
        self.pipelines.draw_oit(pass, stage, &self.indices);
    }

    /// Draws the stages from `*next` to `stage` that [`Self::prepare`] uploaded into a pass
    /// over the world's colour and depth, and moves `next` past them: a stage the frame did
    /// not reach in this pass draws here, after those before it. Within the stages, the
    /// depth-tested quads and lines, then those seen through walls over everything, then the
    /// mods' pipeline quads in the order queued. Leaves the pass with its own pipeline, bind
    /// group 0 and buffers.
    pub fn draw_through(&self, pass: &mut wgpu::RenderPass<'_>, next: &mut usize, stage: usize) {
        let (first, last) = (*next, stage.min(STAGES - 1));
        if first > last {
            return;
        }
        *next = last + 1;
        let Some(AtlasGroup { group, .. }) = &self.group else {
            return;
        };
        let [quads, quads_through, lines, lines_through] =
            spans(self.counts, &self.stage_ends, first, last);
        if !(quads.is_empty()
            && quads_through.is_empty()
            && lines.is_empty()
            && lines_through.is_empty())
        {
            pass.set_bind_group(0, group, &[]);
        }
        for (through, (quad_range, line_range)) in [(quads, lines), (quads_through, lines_through)]
            .into_iter()
            .enumerate()
        {
            Self::draw_pages(
                pass,
                &self.indices,
                (
                    &self.pipelines_standard.quad_pipelines[through],
                    &self.pipelines_standard.line_pipelines[through],
                ),
                (&self.quads, 0, quad_range),
                (&self.lines, 0, line_range),
            );
        }
        self.pipelines
            .draw_excluding(pass, &self.indices, first, last, &self.bound_mod_slots);
    }

    /// Execute one mod-owned offscreen render pass (`OffscreenPass`) into `data.target`.
    pub fn execute_offscreen_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        data: &super::mod_pipelines::OffscreenPassData<'_>,
    ) {
        let upload: Upload<'_> = &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes);
        let needed_quads = self.pipelines.offscreen_needed_quads(data.draws);
        if needed_quads > 0 {
            self.reserve_indices(device, upload, needed_quads);
        }
        self.pipelines
            .execute_offscreen_pass(device, queue, encoder, &self.indices, data);
    }
}

/// A batch's stage ends made safe to draw from: never decreasing, never past `counts`.
fn clamped_ends<const N: usize>(ends: [[u32; 4]; N], counts: [u32; 4]) -> [[u32; 4]; N] {
    let mut floor = [0; 4];
    ends.map(|end| {
        floor = std::array::from_fn(|i| end[i].clamp(floor[i], counts[i]));
        floor
    })
}

/// The uploaded quads and lines of stages `first..=last`: quads depth tested, quads through
/// walls, lines depth tested, lines through walls, each as a range of its buffer, where the
/// through-walls ones follow all the depth-tested ones.
fn spans<const N: usize>(
    counts: [u32; 4],
    ends: &[[u32; 4]; N],
    first: usize,
    last: usize,
) -> [std::ops::Range<u32>; 4] {
    let start = first.checked_sub(1).map_or([0; 4], |stage| ends[stage]);
    let end = ends.get(last).copied().unwrap_or(counts);
    let [quads, _, lines, _] = counts;
    [
        start[0]..end[0],
        quads + start[1]..quads + end[1],
        start[2]..end[2],
        lines + start[3]..lines + end[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const SIZE: u32 = 64;
    /// RGBA, as `Vertex::color` holds it.
    const RED: u32 = 0xFF00_00FF;
    const GREEN: u32 = 0xFF00_FF00;
    const BLUE: u32 = 0xFFFF_0000;
    /// ARGB, as a line carries it.
    const MAGENTA_ARGB: u32 = 0xFFFF_00FF;

    /// A camera-relative quad of plain colour facing the camera, `z` blocks ahead of it, over
    /// the pixels `x0..x1`, `y0..y1` of the frame (y down).
    fn quad(view_proj: Mat4, [x0, y0, x1, y1]: [f32; 4], z: f32, rgba: u32) -> [Vertex; 4] {
        let depth = view_proj.project_point3(Vec3::new(0.0, 0.0, -z)).z;
        let inverse = view_proj.inverse();
        let at = |x: f32, y: f32| {
            let ndc = Vec3::new(
                x / SIZE as f32 * 2.0 - 1.0,
                1.0 - y / SIZE as f32 * 2.0,
                depth,
            );
            inverse.project_point3(ndc).to_array()
        };
        [at(x0, y0), at(x0, y1), at(x1, y1), at(x1, y0)].map(|pos| Vertex {
            pos,
            uv: [0.5, 0.5],
            layer: 0,
            color: rgba,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0, 0.0],
        })
    }

    /// A horizontal line, 4 pixels wide, over the pixels `x0..x1` of row `y`, `z` blocks ahead.
    fn line(view_proj: Mat4, [x0, x1]: [f32; 2], y: f32, z: f32, argb: u32) -> [f32; 8] {
        let q = quad(view_proj, [x0, y, x1, y], z, 0);
        let [a, b] = [q[0].pos, q[3].pos];
        [
            a[0],
            a[1],
            a[2],
            b[0],
            b[1],
            b[2],
            f32::from_bits(argb),
            4.0,
        ]
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let (_, device, queue) = crate::gfx::test_gpu::optional_device()?;
        Some((device, queue))
    }

    #[test]
    fn each_stage_draws_its_own_part_of_every_slice() {
        // Quads, quads through walls, lines, lines through walls: 6, 2, 4, 3 in all.
        let counts = [6, 2, 4, 3];
        // Stage 0 has 1 quad; stages 1 and 2 nothing; stage 3 has 2 quads and a line;
        // stage 4 nothing; stage 5 has a quad through walls; stage 6 has the rest.
        let ends = [
            [1, 0, 0, 0],
            [1, 0, 0, 0],
            [1, 0, 0, 0],
            [3, 0, 1, 0],
            [3, 0, 1, 0],
            [3, 1, 1, 0],
        ];
        assert_eq!(spans(counts, &ends, 0, 0), [0..1, 6..6, 0..0, 4..4]);
        assert_eq!(spans(counts, &ends, 1, 1), [1..1, 6..6, 0..0, 4..4]);
        assert_eq!(spans(counts, &ends, 2, 2), [1..1, 6..6, 0..0, 4..4]);
        assert_eq!(spans(counts, &ends, 3, 3), [1..3, 6..6, 0..1, 4..4]);
        assert_eq!(spans(counts, &ends, 4, 4), [3..3, 6..6, 1..1, 4..4]);
        assert_eq!(spans(counts, &ends, 5, 5), [3..3, 6..7, 1..1, 4..4]);
        assert_eq!(spans(counts, &ends, 6, 6), [3..6, 7..8, 1..4, 4..7]);
        // A pass that skipped the middle stages draws them with the next one, in order.
        assert_eq!(spans(counts, &ends, 1, 5), [1..3, 6..7, 0..1, 4..4]);
        // Everything, as a batch without stages draws.
        assert_eq!(spans(counts, &ends, 0, 6), [0..6, 6..8, 0..4, 4..7]);
        assert_eq!(
            spans(counts, &[[0; 4]; STAGES - 1], 6, 6),
            [0..6, 6..8, 0..4, 4..7]
        );
    }

    #[test]
    fn stage_ends_past_the_counts_or_going_back_are_clamped() {
        let ends = clamped_ends(
            [
                [9, 0, 2, 0],
                [1, 0, 1, 0],
                [9, 9, 9, 9],
                [0; 4],
                [2, 1, 0, 0],
                [0; 4],
            ],
            [6, 2, 4, 3],
        );
        assert_eq!(
            ends,
            [
                [6, 0, 2, 0],
                [6, 0, 2, 0],
                [6, 2, 4, 3],
                [6, 2, 4, 3],
                [6, 2, 4, 3],
                [6, 2, 4, 3],
            ]
        );
    }

    #[test]
    fn oit_stages_slice_and_clamp_independently() {
        let counts = [5, 3, 4, 2];
        let ends = [[2, 1, 1, 0], [4, 1, 3, 2]];
        assert_eq!(spans(counts, &ends, 0, 0), [0..2, 5..6, 0..1, 4..4]);
        assert_eq!(spans(counts, &ends, 1, 1), [2..4, 6..6, 1..3, 4..6]);
        assert_eq!(spans(counts, &ends, 2, 2), [4..5, 6..8, 3..4, 6..6]);
        assert_eq!(
            spans(counts, &[[0; 4]; OIT_STAGES - 1], 2, 2),
            [0..5, 5..8, 0..4, 4..6]
        );
        assert_eq!(
            clamped_ends([[9, 1, 5, 0], [1, 0, 2, 9]], counts),
            [[5, 1, 4, 0], [5, 1, 4, 2]]
        );
        assert!(OitBatch::default().is_empty());
        assert!(Batch::default().is_empty());
        let line = [[0.0_f32; 8]];
        let oit_only = Batch {
            oit: OitBatch {
                lines: &line,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!oit_only.is_empty());
    }

    #[test]
    fn the_oit_shader_validates() {
        let source = oit_shader();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in [
            "vs_quad",
            "vs_line",
            "fs_oit_quad_bounds",
            "fs_oit_quad_transmittance",
            "fs_oit_quad_accumulate",
            "fs_oit_line_bounds",
            "fs_oit_line_transmittance",
            "fs_oit_line_accumulate",
        ] {
            assert!(
                module.entry_points.iter().any(|e| e.name == entry),
                "{entry}"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn device_limited_pages_keep_every_record_and_page_local_ranges() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        let (device, _) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_buffer_size: 1024,
                max_uniform_buffer_binding_size: 1024,
                max_storage_buffer_binding_size: 1024,
                ..Default::default()
            },
            ..Default::default()
        }))
        .expect("device with small vertex pages");
        let mut pages = VertexPages::default();
        let data = (0..40).flat_map(|record| [record; 40]).collect::<Vec<u8>>();
        let mut written = Vec::new();
        let next = pages.upload(
            &device,
            &mut |buffer, bytes| {
                assert!(buffer.size() <= 1024);
                written.extend_from_slice(bytes);
            },
            "page test",
            &data,
            40,
            0,
        );
        assert_eq!(next, 2);
        assert_eq!(written, data);
        assert_eq!(pages.index_quads(40), 25);
        let ranges = pages
            .ranges(0, 22..31)
            .map(|(buffer, range)| (buffer.size(), range))
            .collect::<Vec<_>>();
        assert_eq!(ranges, [(1024, 22..25), (1024, 0..6)]);
    }

    #[test]
    #[ignore = "daily"]
    fn uploads_preserve_ranges_and_reuse_high_water_storage() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let view_proj = Mat4::IDENTITY;
        let red = quad(view_proj, [0.0, 0.0, 32.0, 32.0], 2.0, RED);
        let blue = quad(view_proj, [32.0, 32.0, 64.0, 64.0], 2.0, BLUE);
        // Each slice drops its own trailing partial quad, not just the combined tail.
        let tested = [&red[..], &red[..1]].concat();
        let through = [&blue[..], &blue[..3]].concat();
        let lines = [line(view_proj, [0.0, 64.0], 16.0, 2.0, MAGENTA_ARGB)];
        let lines_through = [line(view_proj, [0.0, 64.0], 48.0, 2.0, MAGENTA_ARGB)];
        let batch = Batch {
            quads: &tested,
            quads_through: &through,
            lines: &lines,
            lines_through: &lines_through,
            ..Default::default()
        };
        assert!(!primitives.prepare(
            &device,
            &mut |_, _| panic!("empty frames must not upload"),
            &Batch::default(),
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        ));
        assert!(primitives.group.is_none());
        let mut writes = Vec::new();
        primitives.reserve(
            &device,
            &mut |buffer, bytes| writes.push((buffer.clone(), bytes.to_vec())),
            2,
            2,
        );
        let buffers = [
            primitives.quads.buffers[0].clone(),
            primitives.lines.buffers[0].clone(),
            primitives.indices.clone(),
        ];
        let storage = |p: &WorldPrimitives| {
            [
                (
                    p.quad_vertices.as_ptr() as usize,
                    p.quad_vertices.capacity(),
                ),
                (
                    p.line_instances.as_ptr() as usize,
                    p.line_instances.capacity(),
                ),
                (p.index_data.as_ptr() as usize, p.index_data.capacity()),
            ]
        };
        let original_storage = storage(&primitives);
        assert_eq!(writes.len(), 1, "setup uploads only indices");
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| writes.push((buffer.clone(), bytes.to_vec())),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        ));
        assert_eq!(primitives.counts, [1, 1, 1, 1]);
        assert_eq!(writes.len(), 4);
        assert_eq!(
            writes[1].1,
            bytemuck::cast_slice::<_, u8>(&[red, blue].concat())
        );
        assert_eq!(
            writes[0].1,
            bytemuck::cast_slice::<_, u8>(&[0u32, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7])
        );
        assert_eq!(
            writes[2].1,
            bytemuck::cast_slice::<_, u8>(&[lines[0], lines_through[0]])
        );
        assert_eq!(writes[3].0, primitives.globals);
        assert_eq!(
            [
                &primitives.quads.buffers[0],
                &primitives.lines.buffers[0],
                &primitives.indices
            ],
            [&buffers[0], &buffers[1], &buffers[2]],
            "first active frame uses setup buffers",
        );
        assert_eq!(storage(&primitives), original_storage);
        let group = primitives.group.as_ref().unwrap().group.clone();
        // Same view via a cloned handle, smaller frame, empty frame, then full frame again.
        // Empty frames neither upload nor discard the reusable resources.
        for batch in [
            batch,
            Batch {
                quads: &red,
                ..Default::default()
            },
            Batch::default(),
            batch,
        ] {
            let mut uploads = 0;
            let indices = primitives.indices.clone();
            let active = primitives.prepare(
                &device,
                &mut |buffer, bytes| {
                    assert_ne!(*buffer, indices, "indices must not be regenerated");
                    queue.write_buffer(buffer, 0, bytes);
                    uploads += 1;
                },
                &batch,
                view_proj,
                [SIZE, SIZE],
                &atlas_view.clone(),
                None,
            );
            assert_eq!(active, !batch.is_empty());
            let expected_uploads = if batch.is_empty() {
                0
            } else if batch.lines.is_empty() {
                2
            } else {
                3
            };
            assert_eq!(uploads, expected_uploads);
            assert_eq!(
                [
                    &primitives.quads.buffers[0],
                    &primitives.lines.buffers[0],
                    &primitives.indices
                ],
                [&buffers[0], &buffers[1], &buffers[2]],
            );
            assert_eq!(primitives.group.as_ref().unwrap().group, group);
            assert_eq!(storage(&primitives), original_storage);
            if batch.is_empty() {
                assert_eq!(primitives.counts, [0; 4]);
            }
        }
        // Growth appends valid indices beyond the previous capacity.
        let larger = [red, blue, red].concat();
        let mut index_upload = Vec::new();
        let old_indices = primitives.indices.clone();
        primitives.prepare(
            &device,
            &mut |buffer, bytes| {
                if buffer.usage().contains(wgpu::BufferUsages::INDEX) {
                    index_upload.extend_from_slice(bytemuck::cast_slice::<u8, u32>(bytes));
                }
            },
            &Batch {
                quads: &larger,
                ..Default::default()
            },
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        );
        assert_ne!(primitives.indices, old_indices);
        assert_eq!(
            index_upload,
            [
                0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15
            ]
        );
        assert_eq!(primitives.group.as_ref().unwrap().group, group);
    }

    /// A wall 5 blocks ahead fills the depth (reversed: nearer is greater). Behind it, a
    /// depth-tested quad and line stay hidden while the same seen through walls show; a
    /// depth-tested quad in front of the wall shows.
    #[test]
    #[ignore = "daily"]
    fn depth_tested_primitives_hide_behind_walls_and_see_through_ones_do_not() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let view_proj = super::super::world_projection(70.0, 1.0, 512.0);
        let wall = view_proj.project_point3(Vec3::new(0.0, 0.0, -5.0)).z;
        // Reversed depth: 10 blocks away is less than the wall, 2 blocks away more.
        assert!(view_proj.project_point3(Vec3::new(0.0, 0.0, -10.0)).z < wall);
        assert!(view_proj.project_point3(Vec3::new(0.0, 0.0, -2.0)).z > wall);

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture = |size: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            SIZE,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            SIZE,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        // The atlas: one white texel, what plain colours sample.
        let atlas = texture(
            1,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            atlas.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            atlas.size(),
        );

        let (half, full) = (SIZE as f32 / 2.0, SIZE as f32);
        // Top left: behind the wall, depth tested. Top right: behind it, through walls.
        // Bottom left: in front of it, depth tested. Bottom right: lines behind it, the
        // depth-tested one along row 40, the one through walls along row 56.
        let quads = [
            quad(view_proj, [0.0, 0.0, half, half], 10.0, RED),
            quad(view_proj, [0.0, half, half, full], 2.0, BLUE),
            // The stage must not draw this prepared but unselected batch.
            quad(view_proj, [half, half, full, full], 2.0, RED),
        ]
        .concat();
        let quads_through = quad(view_proj, [half, 0.0, full, half], 10.0, GREEN);
        let lines = [
            line(view_proj, [half, full], 40.0, 10.0, MAGENTA_ARGB),
            line(view_proj, [half, full], 16.0, 2.0, MAGENTA_ARGB),
        ];
        let lines_through = [
            line(view_proj, [half, full], 56.0, 10.0, MAGENTA_ARGB),
            line(view_proj, [half, full], 24.0, 10.0, MAGENTA_ARGB),
        ];
        let batch = Batch {
            quads: &quads,
            quads_through: &quads_through,
            lines: &lines,
            lines_through: &lines_through,
            ..Default::default()
        };

        let mut primitives = WorldPrimitives::new(&device, (format, false));
        let atlas_view = atlas.create_view(&Default::default());
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        ));
        let original_group = primitives.group.as_ref().unwrap().group.clone();
        // A resource reload replaces the atlas, even at the same dimensions. Blue is now
        // half intensity, making stale bindings visible in the pixel assertions below.
        let replacement = texture(
            1,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            replacement.as_image_copy(),
            &[255, 255, 128, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            replacement.size(),
        );
        let replacement_view = replacement.create_view(&Default::default());
        primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &replacement_view,
            None,
        );
        let replacement_group = primitives.group.as_ref().unwrap().group.clone();
        assert_ne!(replacement_group, original_group);
        primitives.prepare(
            &device,
            &mut |buffer, bytes| queue.write_buffer(buffer, 0, bytes),
            &batch,
            view_proj,
            [SIZE, SIZE],
            &replacement_view.clone(),
            None,
        );
        assert_eq!(primitives.group.as_ref().unwrap().group, replacement_group);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let color_view = color.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wall),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            assert!(primitives.draw_prepared(
                &mut pass,
                None,
                &[DrawRanges {
                    primitives: [0..2, 0..1, 0..2, 0..2],
                    ..Default::default()
                }]
            ));
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let texels = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [texels[i], texels[i + 1], texels[i + 2], texels[i + 3]]
        };
        let black = [0, 0, 0, 255];
        assert_eq!(
            at(16, 16),
            black,
            "a depth-tested quad behind the wall is hidden"
        );
        assert_eq!(
            at(48, 16),
            [0, 255, 0, 255],
            "a through-wall quad covers a depth-tested line in front of the wall"
        );
        assert_eq!(
            at(16, 48),
            [0, 0, 128, 255],
            "a depth-tested quad before it shows, sampling the replacement atlas"
        );
        assert_eq!(
            at(48, 40),
            black,
            "a depth-tested line behind the wall is hidden"
        );
        assert_eq!(
            at(48, 56),
            [255, 0, 255, 255],
            "a line through walls shows behind it"
        );
        assert_eq!(
            at(48, 24),
            [255, 0, 255, 255],
            "a through-wall line covers a through-wall quad",
        );
        // The line is its width (4 pixels) thick, not more. Unselected geometry stays undrawn.
        assert_eq!(at(48, 52), black);
    }

    #[test]
    #[ignore = "daily"]
    fn chunked_uploads_respect_small_max_buffer_size_and_preserve_stage_order() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };
        // A non-power-of-two limit that still fits the 64 × 64 RGBA readback.
        let limit = 20 * 1024;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_buffer_size: limit,
                max_uniform_buffer_binding_size: limit,
                max_storage_buffer_binding_size: limit,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .expect("GPU device with small max_buffer_size");
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            atlas.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout::default(),
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas.create_view(&Default::default());
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        let view_proj =
            Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        let quad_bytes = 4 * std::mem::size_of::<Vertex>() as u64;
        let max_quads = VertexPages::limit(limit, quad_bytes);
        assert!(max_quads > 0);
        // Exact-fit upload (whose power-of-two would exceed `limit`) stays within `limit`.
        let exact_fit: Vec<Vertex> = (0..max_quads)
            .flat_map(|_| quad(view_proj, [0.0, 0.0, 32.0, 32.0], 2.0, RED))
            .collect();
        assert!(
            (exact_fit.len() * std::mem::size_of::<Vertex>()).next_power_of_two() as u64 > limit
        );
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| {
                assert!(
                    buffer.size() <= limit,
                    "buffer {} > limit {limit}",
                    buffer.size()
                );
                assert!(bytes.len() as u64 <= buffer.size());
                queue.write_buffer(buffer, 0, bytes);
            },
            &Batch {
                quads: &exact_fit,
                ..Default::default()
            },
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        ));
        assert_eq!(primitives.quads.buffers.len(), 1);

        // Multi-page upload exceeding `limit`: first pages draw RED in stage 0, last page
        // draws GREEN over the right half in the final stage.
        let total = max_quads as usize * 2 + 5;
        let mut multi_quads = Vec::with_capacity(total * 4);
        for i in 0..total {
            let color = if i + 1 == total { GREEN } else { RED };
            let left = if i + 1 == total { 32.0 } else { 0.0 };
            multi_quads.extend_from_slice(&quad(view_proj, [left, 0.0, 64.0, 64.0], 2.0, color));
        }
        let mut stage_ends = [[0u32; 4]; STAGES - 1];
        for end in &mut stage_ends {
            end[0] = (total - 1) as u32;
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut uploads = crate::gfx::upload::Uploads::with_chunk(&device, 1024);
        assert!(primitives.prepare(
            &device,
            &mut |buffer, bytes| {
                assert!(
                    buffer.size() <= limit,
                    "buffer {} > limit {limit}",
                    buffer.size()
                );
                assert!(bytes.len() as u64 <= buffer.size());
                uploads.buffer(&mut encoder, buffer, 0, bytes);
            },
            &Batch {
                quads: &multi_quads,
                stage_ends,
                ..Default::default()
            },
            view_proj,
            [SIZE, SIZE],
            &atlas_view,
            None,
        ));
        assert_eq!(primitives.quads.buffers.len(), 3);
        assert!(primitives.indices.size() <= limit);

        let colour = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &colour.create_view(&Default::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.create_view(&Default::default()),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let mut next = 0;
            primitives.draw_through(&mut pass, &mut next, 0);
            primitives.draw_through(&mut pass, &mut next, STAGES - 1);
        }
        encoder.copy_texture_to_buffer(
            colour.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let texels = readback.slice(..).get_mapped_range().unwrap().to_vec();
        let pixel = |x| {
            let i = ((32 * SIZE + x) * 4) as usize;
            &texels[i..i + 4]
        };
        assert_eq!(pixel(16), [255, 0, 0, 255]);
        assert_eq!(pixel(48), [0, 255, 0, 255]);
    }
}
