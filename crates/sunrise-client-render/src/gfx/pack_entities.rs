//! Entities, block entities, translucent entity parts and particles drawn by a shader pack's
//! `gbuffers_entities`, `gbuffers_block`, `gbuffers_entities_translucent` and
//! `gbuffers_particles` (spec 2.3, 5.3, 5.4). Re-exported from sunrise_render_shaderpack.

pub use sunrise_render_shaderpack::pack_entities::*;

<<<<<<< sync-sunrise-1
use sunrise_shaders::render_stage;

use super::Vertex;
use super::pack_draw::EntitySpan;

/// `HzDraw`'s size (pack_draw.rs).
const SLOT: u64 = 32;

const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x2, 2 => Uint32, 3 => Unorm8x4, 4 => Uint8x4, 5 => Snorm8x4
];

/// The entity vertex: Sunrise's own (`Vertex`).
pub fn buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 1] {
    [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    })]
}

/// What `gbuffers_entities` and `gbuffers_block` read from an entity vertex: camera-relative
/// position, the shared atlas's UV, colour, light (block, sky in 16ths) and the world-oriented
/// normal. The quad's corners, read from the same vertices at set 1, give the face normal where
/// the vertex has none (custom geometry) and `at_tangent` along the texture's u, as for terrain.
pub fn inputs() -> sunrise_shaders::GbuffersInputs {
    let words = std::mem::size_of::<Vertex>() / 4;
    let mut declarations = "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in vec2 hz_v_uv;\n\
         layout(location = 2) in uint hz_v_overlay;\nlayout(location = 3) in vec4 hz_v_color;\n\
         layout(location = 4) in uvec4 hz_v_light;\nlayout(location = 5) in vec4 hz_v_normal;\n\
         layout(set = 1, binding = 0) readonly buffer HzEntityVertices { uint hz_entity_vertices[]; };\n\
         vec4 hz_tangent; vec2 hz_mid;\n"
        .to_owned();
    // `at_velocity`: the vertex's offset from last frame in view space, this frame's view
    // position less last frame's. Last frame's vertices stay at binding 2, relative to last
    // frame's camera, and binding 3 gives each vertex its index there (none: no velocity).
    declarations.push_str(&format!(
        "layout(set = 1, binding = 2) readonly buffer HzEntityPrevious {{ uint hz_entity_previous[]; }};\n\
         layout(set = 1, binding = 3) readonly buffer HzEntityPreviousIndex {{ uint hz_previous_index[]; }};\n\
         uint hz_vi;\n\
         vec3 hz_velocity(mat4 view, mat4 previous_view) {{\n    \
             uint p = hz_previous_index[hz_vi];\n    \
             if (p == 0xffffffffu) return vec3(0.0);\n    \
             uint w = p * {words}u;\n    \
             vec3 then = uintBitsToFloat(uvec3(hz_entity_previous[w], hz_entity_previous[w + 1u], hz_entity_previous[w + 2u]));\n    \
             return (view * vec4(hz_v_pos, 1.0)).xyz - (previous_view * vec4(then, 1.0)).xyz;\n\
         }}\n"
    ));
    let corner = |k: usize, field: &str| {
        let at = |o: usize| format!("hz_entity_vertices[hz_q + {}u]", k * words + o);
        match field {
            "pos" => format!("uintBitsToFloat(uvec3({}, {}, {}))", at(0), at(1), at(2)),
            _ => format!("uintBitsToFloat(uvec2({}, {}))", at(3), at(4)),
        }
    };
    let body = format!(
        "    hz_vi = uint(gl_VertexIndex);\n    \
         uint hz_q = (uint(gl_VertexIndex) & ~3u) * {words}u;\n    \
         vec3 hz_e1 = {p1} - {p0};\n    \
         vec3 hz_e2 = {p2} - {p0};\n    \
         vec2 hz_t0 = {t0};\n    \
         vec2 hz_d1 = {t1} - hz_t0;\n    \
         vec2 hz_d2 = {t2} - hz_t0;\n    \
         hz_mid = (hz_t0 + {t2}) * 0.5;\n    \
         vec3 hz_n = cross(hz_e1, hz_e2);\n    \
         hz_Normal = dot(hz_v_normal.xyz, hz_v_normal.xyz) > 0.01 ? normalize(hz_v_normal.xyz)\n        \
             : dot(hz_n, hz_n) > 1e-12 ? normalize(hz_n) : vec3(0.0, 1.0, 0.0);\n    \
         hz_tangent = vec4(1.0, 0.0, 0.0, 1.0);\n    \
         float hz_r = hz_d1.x * hz_d2.y - hz_d2.x * hz_d1.y;\n    \
         vec3 hz_t = (hz_e1 * hz_d2.y - hz_e2 * hz_d1.y) * sign(hz_r);\n    \
         vec3 hz_b = (hz_e2 * hz_d1.x - hz_e1 * hz_d2.x) * sign(hz_r);\n    \
         hz_t -= hz_Normal * dot(hz_Normal, hz_t);\n    \
         if (dot(hz_t, hz_t) > 1e-12) {{\n        \
             hz_t = normalize(hz_t);\n        \
             hz_tangent = vec4(hz_t, dot(cross(hz_t, hz_Normal), hz_b) < 0.0 ? -1.0 : 1.0);\n    \
         }}\n    \
         hz_Vertex = vec4(hz_v_pos, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(hz_v_uv, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(float(hz_v_light.y), float(hz_v_light.x), 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Color = hz_v_color;\n",
        p0 = corner(0, "pos"),
        p1 = corner(1, "pos"),
        p2 = corner(2, "pos"),
        t0 = corner(0, "uv"),
        t1 = corner(1, "uv"),
        t2 = corner(2, "uv"),
    );
    let attributes = [
        ("at_tangent", "hz_tangent"),
        ("mc_midTexCoord", "vec4(hz_mid, 0.0, 1.0)"),
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaUV0", "hz_v_uv"),
        ("vaUV1", "ivec2(0, 10)"),
        ("vaUV2", "ivec2(hz_MultiTexCoord1.xy)"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
        (
            "at_velocity",
            "hz_velocity(gbufferModelView, gbufferPreviousModelView)",
        ),
        // Sunrise's own, as for terrain: the HDR tier of a light-giving item, particle or glowing
        // text (`mesher::hdr_tier`, the overlay's high bits), 0 for none, 1 to 4 for low to very
        // bright.
        ("sunriseTier", "float((hz_v_overlay >> 16u) & 255u)"),
    ];
    sunrise_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after: String::new(),
        prepass_uv: false,
        // Cutout, as vanilla's entity types mostly are (OQ-23): the loader's test at 0.1.
        alpha_test: Some(sunrise_shaders::AlphaTest {
            op: ">",
            reference: 0.1,
        }),
        parts: None,
        item_id_declared: false,
        far_depth: false,
        // Entities, items and block entities read the shared atlas: its own maps (paged_maps.rs).
        paged_maps: true,
        water_mask: false,
        water_patches: false,
    }
}

/// What a shadow program with a geometry stage reads of entities and block entities: the
/// inputs of [`inputs`], each vertex read from set 1 (`hz_entity_vertices`), as `Vertex` lays
/// it out. The draw's corners walk its quads two triangles each, as `quad_indices` does: corner
/// `c` of a draw from vertex `first` (a whole quad) is drawn as vertex `first / 4 * 6 + c`.
pub fn geometry_inputs() -> sunrise_shaders::GeometryInputs {
    let words = std::mem::size_of::<Vertex>() / 4;
    let pull = format!(
        "        uint hz_pw = hz_vertex_index * {words}u;\n        \
         hz_v_pos = uintBitsToFloat(uvec3(hz_entity_vertices[hz_pw], hz_entity_vertices[hz_pw + 1u], hz_entity_vertices[hz_pw + 2u]));\n        \
         hz_v_uv = uintBitsToFloat(uvec2(hz_entity_vertices[hz_pw + 3u], hz_entity_vertices[hz_pw + 4u]));\n        \
         hz_v_overlay = hz_entity_vertices[hz_pw + 5u];\n        \
         hz_v_color = unpackUnorm4x8(hz_entity_vertices[hz_pw + 6u]);\n        \
         uint hz_pl = hz_entity_vertices[hz_pw + 7u];\n        \
         hz_v_light = uvec4(hz_pl & 255u, (hz_pl >> 8u) & 255u, (hz_pl >> 16u) & 255u, hz_pl >> 24u);\n        \
         hz_v_normal = unpackSnorm4x8(hz_entity_vertices[hz_pw + 8u]);\n"
    );
    sunrise_shaders::GeometryInputs {
        inputs: inputs(),
        pull,
        // Quad `c / 6`, corner 0, 1, 2, 0, 2, 3 of it.
        index:
            "((@) / 6u * 4u + ((@) % 6u < 3u ? (@) % 6u : ((@) % 6u == 3u ? 0u : (@) % 6u - 2u)))"
                .to_owned(),
    }
}

/// What `gbuffers_clouds` reads from the clouds' mesh (entity vertices, `vs_cloud`): the
/// position moved by the clouds' offset, the colour of the face's shade from their palette,
/// full light, the face's normal. Set 1 holds the clouds' uniform (`clouds::Uniform`).
pub fn cloud_inputs() -> sunrise_shaders::GbuffersInputs {
    let declarations = "layout(location = 0) in vec3 hz_v_pos;\nlayout(location = 1) in vec2 hz_v_uv;\n\
         layout(location = 2) in uint hz_v_overlay;\nlayout(location = 3) in vec4 hz_v_color;\n\
         layout(location = 4) in uvec4 hz_v_light;\nlayout(location = 5) in vec4 hz_v_normal;\n\
         layout(set = 1, binding = 0) uniform HzCloud { vec4 hz_cloud_offset; vec4 hz_cloud_colors[4]; };\n"
        .to_owned();
    let body = "    hz_Vertex = vec4(hz_v_pos + hz_cloud_offset.xyz, 1.0);\n    \
         hz_MultiTexCoord0 = vec4(hz_v_uv, 0.0, 1.0);\n    \
         hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    \
         hz_MultiTexCoord2 = hz_MultiTexCoord1;\n    \
         hz_Color = hz_cloud_colors[min(hz_v_light.x, 3u)];\n    \
         hz_Normal = dot(hz_v_normal.xyz, hz_v_normal.xyz) > 0.01 ? normalize(hz_v_normal.xyz) : vec3(0.0, 1.0, 0.0);\n"
        .to_owned();
    let attributes = [
        ("vaPosition", "hz_Vertex.xyz"),
        ("vaUV0", "hz_v_uv"),
        ("vaUV2", "ivec2(240, 240)"),
        ("vaColor", "hz_Color"),
        ("vaNormal", "hz_Normal"),
    ];
    sunrise_shaders::GbuffersInputs {
        declarations,
        body,
        attributes: attributes
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect(),
        after: String::new(),
        prepass_uv: false,
        alpha_test: None,
        parts: None,
        item_id_declared: false,
        far_depth: false,
        paged_maps: false,
        water_mask: false,
        water_patches: false,
    }
}

/// `entityColor` for a run whose first quad carries overlay code `overlay`: the hurt red or a
/// white flash, its alpha how much of that colour shows over the entity (vanilla mixes the
/// overlay texel's colour in by one minus the texel's alpha).
pub fn entity_color(overlay: u32) -> [f32; 4] {
    let [r, g, b, a] = crate::overlay::HURT_TEXEL.map(|v| f32::from(v) / 255.0);
    let white = |u: u32| {
        let a = f32::from(crate::overlay::white_alpha(u)) / 255.0;
        [1.0, 1.0, 1.0, 1.0 - a]
    };
    match overlay {
        1 | 8 => [r, g, b, 1.0 - a],
        5 => white(15),
        16..32 => white(overlay - crate::overlay::WHITE),
        _ => [0.0; 4],
    }
}

/// One draw: its quads in `PackEntities`' buffer, its program and its slot of `HzDraw`.
pub(super) struct Draw {
    /// The first vertex.
    pub base: u32,
    pub quads: u32,
    /// The kind of program that draws it (`ENTITY`, `BLOCK`, `TRANSLUCENT`, `PARTICLE`).
    pub program: usize,
    /// The pack's gbuffers program draws it (else Sunrise does).
    pub pack: bool,
    /// The pack's shadow program draws it into the shadow map too.
    pub shadow: bool,
    pub slot: u32,
    /// End portal or gateway faces: drawn with `end_portal.png` bound as the atlas.
    pub portal: bool,
    /// A boat's water patch (`WATER_MASK_OVERLAY`): depth only, with the program's pipeline that
    /// writes no colour.
    pub water_mask: bool,
}

/// Which entity runs a pack's shadow pass draws (2.4, 3.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Casters {
    /// `shadow_entities` (or `shadow`) draws entities...
    pub entities: bool,
    /// ...only the local player (`shadowPlayer` without `shadowEntities`).
    pub only_player: bool,
    /// `shadow_block` (or `shadow`) draws block entities...
    pub blocks: bool,
    /// ...only those whose block emits light (`shadowLightBlockEntities`).
    pub only_lit: bool,
}

impl Casters {
    /// Whether the shadow pass draws `span`'s run, of kind `program`.
    pub fn cast(&self, span: &EntitySpan, program: usize) -> bool {
        match program {
            ENTITY => self.entities && (!self.only_player || span.player),
            BLOCK | BLOCK_TRANSLUCENT => self.blocks && (!self.only_lit || span.emissive),
            _ => false,
        }
    }
}

/// The vertices and per-draw uniforms of the runs a pack draws, rebuilt each frame.
pub(super) struct PackEntities {
    vertices: wgpu::Buffer,
    /// Last frame's `vertices` (the two swap each frame), for `at_velocity`.
    previous: wgpu::Buffer,
    /// For each of `vertices`, its index in `previous` (`NO_PREVIOUS`: none).
    previous_index: wgpu::Buffer,
    /// This frame's entities (`Frame::entity_motion`) and last frame's.
    motion: Motion,
    previous_motion: Motion,
    index_scratch: Vec<u32>,
    /// Set 1: the vertices again, for the quads' corners; last frame's and the indices into them.
    pub table: wgpu::BindGroupLayout,
    pub table_group: wgpu::BindGroup,
    slots: wgpu::Buffer,
    pub group: wgpu::BindGroup,
    stride: u64,
    /// Two triangles per quad, for `quads` quads.
    pub indices: wgpu::Buffer,
    quads: u32,
    scratch: Vec<Vertex>,
    uniforms: Vec<u8>,
    pub draws: Vec<Draw>,
}

impl PackEntities {
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let stride = SLOT.max(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        let entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let table = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack entity vertices"),
            entries: &[entry(0), entry(2), entry(3)],
        });
        let vertices = Self::vertex_buffer(device, 4096);
        let previous = Self::vertex_buffer(device, 4096);
        let previous_index = Self::index_buffer(device, 4096);
        let table_group = Self::table_group(device, &table, &vertices, &previous, &previous_index);
        let (slots, group) = Self::slot_buffer(device, layout, 64, stride);
        Self {
            vertices,
            previous,
            previous_index,
            motion: Motion::default(),
            previous_motion: Motion::default(),
            index_scratch: Vec::new(),
            table,
            table_group,
            slots,
            group,
            stride,
            indices: quad_indices(device, 1024),
            quads: 1024,
            scratch: Vec::new(),
            uniforms: Vec::new(),
            draws: Vec::new(),
        }
    }

    fn vertex_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack entity vertices"),
            size: vertices * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn index_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack entity previous indices"),
            size: vertices * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn table_group(
        device: &wgpu::Device,
        table: &wgpu::BindGroupLayout,
        vertices: &wgpu::Buffer,
        previous: &wgpu::Buffer,
        index: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack entity vertices"),
            layout: table,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: previous.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: index.as_entire_binding(),
                },
            ],
        })
    }

    fn slot_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        slots: u64,
        stride: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shader pack entity draws"),
            size: slots * stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shader pack entity draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(SLOT),
                }),
            }],
        });
        (buffer, group)
    }

    pub fn skin_target(&self) -> &wgpu::Buffer {
        &self.vertices
    }

    pub fn vertices(&self) -> wgpu::BufferSlice<'_> {
        self.vertices.slice(..)
    }

    /// The dynamic offset of draw slot `slot`.
    pub fn offset(&self, slot: u32) -> u32 {
        (u64::from(slot) * self.stride) as u32
    }

    /// Splits `entities` between the pack, which draws the runs of `spans` its programs cover
    /// (`drawn`: whether it has `gbuffers_entities`, `gbuffers_block` or their fallbacks), and
    /// Sunrise, which draws the rest (`rest`). Uploads the pack's part, and the runs its shadow
    /// programs draw (`shadow`). Runs hidden from the camera only ever cast shadows.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
=======
pub trait PackEntitiesExt {
    fn prepare_client(
>>>>>>> sunrise/main
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        entities: &[crate::gfx::Vertex],
        spans: &[sunrise_render_core::EntitySpan],
        drawn: [bool; PROGRAMS],
        crack: ([f32; 4], [u32; 2]),
        shadow: Casters,
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&crate::gfx::entity_skin::SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    );
}

impl PackEntitiesExt for PackEntities {
    fn prepare_client(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        entities: &[crate::gfx::Vertex],
        spans: &[sunrise_render_core::EntitySpan],
        drawn: [bool; PROGRAMS],
        crack: ([f32; 4], [u32; 2]),
        shadow: Casters,
        rest: &mut Vec<crate::gfx::Vertex>,
        skin: Option<&crate::gfx::entity_skin::SkinFrame>,
        motion: (&[(u32, u64)], &[u32]),
    ) {
        let (jobs, spans_slice, vertices) = match skin {
            Some(s) => {
                let j: Vec<sunrise_render_core::SkinRunJob> = s
                    .jobs
                    .iter()
                    .map(|job| sunrise_render_core::SkinRunJob {
                        dst: job.dst,
                        count: job.count,
                        overlay: job.overlay,
                    })
                    .collect();
                (Some(j), Some(s.spans.as_slice()), s.vertices)
            }
            None => (None, None, 0),
        };
        let posed = jobs.as_ref().map(|j| sunrise_render_core::PosedModelRuns {
            vertices,
            jobs: j,
            spans: spans_slice.unwrap_or_default(),
        });
        self.prepare(
            device,
            queue,
            layout,
            entities,
            spans,
            drawn,
            crack,
            shadow,
            rest,
            posed.as_ref(),
            motion,
        );
    }
}
