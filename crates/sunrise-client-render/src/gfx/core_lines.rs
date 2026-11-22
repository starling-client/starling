//! A resource pack's `core/rendertype_lines` shader on the block outline (`RenderTypes.LINES`,
//! and `secondaryBlockOutline` under a high contrast one): an outline its fragment shader colours
//! or animates. Without one the outline draws natively (`vs_line` in terrain.wgsl).

use crate::core_shader::{CoreProgram, Glue};
use crate::polytone_post::ExpressionUniform;
use glam::Mat4;

/// The outline's edges (`pack_lines::layout`: start, end, ARGB colour and width per instance) as
/// vanilla's lines: four vertices an edge, the start twice and the end twice, the side
/// `gl_VertexIndex % 2`, as `vs_line` draws them.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("Normal", "vec3"),
        ("LineWidth", "float"),
    ],
    prelude: "layout(location = 0) in vec3 hz_start;\n\
              layout(location = 1) in vec3 hz_end;\n\
              layout(location = 2) in vec4 hz_argb;\n\
              layout(location = 3) in float hz_width;",
    // ARGB in a little-endian u32 reads as B, G, R, A. ShapeRenderer's normal: the direction.
    setup: "Position = hz_start; if (int(gl_VertexIndex) >= 2) Position = hz_end; \
            Color = hz_argb.zyxw; Normal = normalize(hz_end - hz_start); LineWidth = hz_width;",
    // 26.3's ProjMat already has wgpu's reversed 0 to 1 depth (Projection: zZeroToOne).
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

pub(crate) struct CoreLines {
    lines: wgpu::RenderPipeline,
    secondary: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// The packs' expression uniforms its stages read (`shader_modifiers`).
    pub(super) extra: super::expression_blocks::ExpressionBlocks,
}

/// The packs' block outline on the renderer.
impl super::Renderer {
    /// The resource packs' `core/rendertype_lines` (`HudAtlas::lines_shader`, with the
    /// expression uniforms resolved for it) for the block outline, or the built-in one.
    pub fn set_lines_shader(&mut self, program: Option<(&CoreProgram, &[ExpressionUniform])>) {
        let format = self.output.frame.format;
        // A pack's GLSL writes vanilla's gamma: a frame of linear light keeps the built-in one.
        let program = program.filter(|_| !self.output.frame.linear_light);
        self.core_lines = program.and_then(|(program, expressions)| {
            CoreLines::new(&self.device, format, program, expressions)
        });
    }
}

impl CoreLines {
    /// The pipelines for `program` with `expressions` (resolved when it loaded: the expression
    /// blocks its stages read).
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        program: &CoreProgram,
        expressions: &[ExpressionUniform],
    ) -> Option<Self> {
        // A pack's shader is untrusted: the backend failing to compile it is an internal error.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = |label, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("Resource pack lines (vertex)", &program.vertex);
        let fragment = module("Resource pack lines (fragment)", &program.fragment);
        // Vanilla's LINES pipeline has no samplers: a pack that reads one is not used. The
        // blocks' sizes are checked here too, not at the draw: a pack's own larger block fails
        // in this scope.
        // Each block a stage reads is bound to the stages reading it: vanilla's four, then the
        // expression uniforms.
        let extra = super::expression_blocks::ExpressionBlocks::new(program, expressions, 1024);
        let blocks = super::expression_blocks::vanilla_blocks(program);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Resource pack lines uniforms"),
            size: extra.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (entries, resources) = super::expression_blocks::bindings(&blocks, &extra, &uniform);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Resource pack lines"),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resource pack lines"),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Resource pack lines"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        // The built-in outline's state: a strip per edge, reversed-Z depth test, translucent.
        let pipeline = |label, depth_write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(super::pack_lines::layout())],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
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
                    module: &fragment,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines = pipeline("Resource pack block outline", true);
        let secondary = pipeline("Resource pack block outline secondary", false);
        let internal = pollster::block_on(internal.pop());
        if let Some(e) = internal.or(pollster::block_on(validation.pop())) {
            log::warn!("the resource pack's rendertype_lines shader is not used: {e}");
            return None;
        }
        Some(Self {
            lines,
            secondary,
            group,
            uniform,
            extra,
        })
    }

    /// Uploads the frame's blocks: `data` (the pipeline's own, [`uniforms`]), then its Polytone
    /// blocks (the frame's `PolyGlobals` and each expression uniform's value).
    pub(super) fn upload(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        data: &[u8],
    ) {
        uploads.buffer(encoder, &self.uniform, 0, data);
        self.extra.upload(uploads, encoder, &self.uniform);
    }

    /// Draw `edges` (instances of the outline buffer bound at slot 0) with the pack's shader:
    /// the secondary stroke without depth writes. The caller sets its own group 0 again after.
    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        secondary: bool,
        edges: std::ops::Range<u32>,
    ) {
        pass.set_pipeline(if secondary {
            &self.secondary
        } else {
            &self.lines
        });
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..4, edges);
    }
}

/// The std140 blocks: `Globals` (`post_chain::Globals::bytes`), `Projection` (`jitter`, the lens
/// and the view bob `pose`, as vanilla's projection matrix carries it), `DynamicTransforms` (the
/// camera's rotation with the PERSPECTIVE `layering` scale, colour modulator white) and `Fog`
/// (`fog`: environmental start and end, render distance start and end, sky end, clouds end).
/// `ProjMat · ModelViewMat` is `vs_line`'s `view_proj` with its layering scale.
pub(super) fn uniforms(
    globals: [u8; 48],
    view: &super::View,
    pose: Mat4,
    jitter: Mat4,
    layering: f32,
    fog_color: [f32; 4],
    fog: [f32; 6],
) -> [u8; 1024] {
    let projection = jitter * view.projection * pose;
    let model_view = view.view * Mat4::from_scale(glam::Vec3::splat(layering));
    let mut data = [0f32; 256];
    data[64..80].copy_from_slice(&projection.to_cols_array());
    let transforms = 128;
    data[transforms..transforms + 16].copy_from_slice(&model_view.to_cols_array());
    data[transforms + 16..transforms + 32].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[transforms + 32..transforms + 36].fill(1.0);
    data[192..196].copy_from_slice(&fog_color);
    data[196..202].copy_from_slice(&fog);
    let mut bytes = [0u8; 1024];
    bytes.copy_from_slice(bytemuck::cast_slice(&data));
    bytes[..48].copy_from_slice(&globals);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    const INCLUDES: [(&str, &str); 3] = [
        (
            "minecraft:shaders/include/globals.glsl",
            "layout(std140) uniform Globals {\n    ivec3 CameraBlockPos;\n    float GlintAlpha;\n    vec3 CameraOffset;\n    float GameTime;\n    vec2 ScreenSize;\n    int MenuBlurRadius;\n    int UseRgss;\n};\n",
        ),
        (
            "minecraft:shaders/include/dynamictransforms.glsl",
            "layout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n",
        ),
        (
            "minecraft:shaders/include/projection.glsl",
            "layout(std140) uniform Projection {\n    mat4 ProjMat;\n};\n",
        ),
    ];

    fn include(id: &str) -> Option<String> {
        INCLUDES
            .iter()
            .find(|(path, _)| *path == id)
            .map(|(_, text)| (*text).to_owned())
    }

    /// 26.3's `rendertype_lines.vsh` in shape: a stroke `LineWidth` pixels wide across the
    /// screen-space direction of `Normal`, its side from `gl_VertexIndex % 2`.
    const LINES_VSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        #include <minecraft:globals.glsl>\n#include <minecraft:dynamictransforms.glsl>\n#include <minecraft:projection.glsl>\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec3 Normal;\nlayout(location = 3) in float LineWidth;\n\
        layout(location = 2) out vec4 vertexColor;\n\
        const mat4 SHRINK = mat4(0.99, 0.0, 0.0, 0.0, 0.0, 0.99, 0.0, 0.0, 0.0, 0.0, 0.99, 0.0, 0.0, 0.0, 0.0, 1.0);\n\
        void main() {\n    vec4 from = ProjMat * SHRINK * ModelViewMat * vec4(Position, 1.0);\n\
            vec4 to = ProjMat * SHRINK * ModelViewMat * vec4(Position + Normal, 1.0);\n\
            vec2 along = normalize((to.xy / to.w - from.xy / from.w) * ScreenSize);\n\
            vec2 side = vec2(-along.y, along.x) * LineWidth / ScreenSize;\n\
            if (gl_VertexIndex % 2 == 0) {\n        side = -side;\n    }\n\
            gl_Position = vec4(from.xy / from.w + side, from.z / from.w, 1.0) * from.w;\n\
            vertexColor = Color;\n}\n";

    /// An RGB outline pack's fragment shader in shape: the colour from `GameTime`.
    const RGB_FSH: &str = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        #include <minecraft:globals.glsl>\n\
        layout(location = 2) in vec4 vertexColor;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vec4(GameTime, 1.0 - GameTime, vertexColor.b, 1.0);\n}\n";

    /// ProjMat · ModelViewMat (with the vertex shader's shrink between them) is `vs_line`'s
    /// view_proj on the layered, shrunk position: the pack's outline sits where the built-in one
    /// does, view bob and TAA jitter included.
    #[test]
    fn the_pack_matrices_put_the_outline_where_vs_line_does() {
        let view = super::super::View::screen(37.0, -21.0, 70.0, 16.0 / 9.0, 256.0);
        let pose =
            Mat4::from_rotation_z(0.02) * Mat4::from_translation(Vec3::new(0.01, -0.03, 0.0));
        let jitter = Mat4::from_translation(Vec3::new(0.0004, -0.0002, 0.0));
        let layering = 1.0 - 1.0 / 4096.0;
        let bytes = uniforms([0; 48], &view, pose, jitter, layering, [0.0; 4], [0.0; 6]);
        let data: Vec<f32> = bytes
            .as_chunks()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let projection = Mat4::from_cols_slice(&data[64..80]);
        let model_view = Mat4::from_cols_slice(&data[128..144]);
        assert_eq!(&data[160..164], &[1.0; 4], "ColorModulator");
        let shrink = 1.0 - 1.0 / 256.0;
        for p in [Vec3::new(1.5, -0.2, 3.0), Vec3::new(-4.0, 2.0, -0.5)] {
            let pack =
                projection * Mat4::from_scale(Vec3::splat(shrink)) * model_view * p.extend(1.0);
            let native = jitter * view.view_proj(pose) * (p * layering * shrink).extend(1.0);
            let error: Vec4 = (pack - native).abs();
            assert!(
                error.max_element() < 1e-5 * native.w.abs(),
                "{pack} {native}"
            );
        }
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let (_, device, queue) = crate::gfx::test_gpu::optional_device()?;
        Some((device, queue))
    }

    /// One edge across the middle of a 64 by 64 target, 4 pixels wide, blue (ARGB `ff0000ff`),
    /// at `GameTime` 0.25, drawn with `program`: the RGBA rows read back.
    fn draw_edge(device: &wgpu::Device, queue: &wgpu::Queue, program: &CoreProgram) -> Vec<u8> {
        draw_edge_with(device, queue, program, &[], None)
    }

    /// [`draw_edge`] with `expressions` evaluated and uploaded as in a frame (`CoreLines::upload`),
    /// beside `poly_globals`.
    fn draw_edge_with(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        program: &CoreProgram,
        expressions: &[ExpressionUniform],
        poly_globals: Option<&super::super::expression_blocks::PolyGlobalsBytes>,
    ) -> Vec<u8> {
        const SIZE: u32 = 64;
        let mut lines = CoreLines::new(
            device,
            wgpu::TextureFormat::Rgba8Unorm,
            program,
            expressions,
        )
        .expect("the pack's pipelines");
        lines.extra.evaluate(&Default::default());
        let target = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                view_formats: &[],
            })
        };
        let color = target(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::COPY_SRC,
        );
        let depth = target(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::empty(),
        );
        let view = super::super::View {
            view: Mat4::IDENTITY,
            projection: super::super::world_projection(70.0, 1.0, 64.0),
            aspect: 1.0,
            tan_half_fov: 35f32.to_radians().tan(),
        };
        let globals = crate::post_chain::Globals {
            game_time: 0.25,
            ..Default::default()
        };
        let data = uniforms(
            globals.bytes([SIZE, SIZE], 0),
            &view,
            Mat4::IDENTITY,
            Mat4::IDENTITY,
            1.0,
            [0.0; 4],
            [0.0; 6],
        );
        let mut uploads = super::super::upload::Uploads::new(device);
        let mut upload = device.create_command_encoder(&Default::default());
        lines.extra.set_globals(poly_globals);
        lines.upload(&mut uploads, &mut upload, &data);
        uploads.finish(&upload);
        queue.submit([upload.finish()]);
        let mut edge = [-1.0, 0.0, -2.0, 1.0, 0.0, -2.0, 0.0, 4.0f32];
        edge[6] = f32::from_bits(0xff00_00ff);
        let edges = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&edge),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_vertex_buffer(0, edges.slice(..));
            lines.draw(&mut pass, false, 0..1);
        }
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            color.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        read.slice(..).get_mapped_range().unwrap().to_vec()
    }

    /// A pack's outline reading an expression uniform (`shader_modifiers`) in its fragment stage
    /// gets the value the frame gives it, in a block of its own after the pipeline's, uploaded
    /// with the pipeline's blocks (`CoreLines::upload`): 0.2, which a unorm byte holds (51).
    #[test]
    #[ignore = "daily"]
    fn a_pack_outline_reads_its_expression_uniform() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping");
            return;
        };
        let fsh = "#version 330\nlayout(std140) uniform Tint {\n    float value;\n};\n\
                   layout(location = 2) in vec4 vertexColor;\n\
                   layout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(value, 0.0, vertexColor.b, 1.0);\n}\n";
        let program = crate::core_shader::translate_core_with(
            LINES_VSH,
            fsh,
            &[],
            &include,
            &GLUE,
            &|block| block == "Tint",
        )
        .expect("the shaders translate");
        let id = super::super::expression_blocks::LINES[0];
        let modifiers = modifiers(r#"{"Tint": "0.2"}"#);
        let expressions = modifiers.resolve((id, id), &program, &[]).unwrap();
        let fragment = wgpu::ShaderStages::FRAGMENT;
        assert_eq!(
            expressions
                .iter()
                .map(|e| (&*e.name, e.stages))
                .collect::<Vec<_>>(),
            [("Tint", fragment)]
        );
        let pixels = draw_edge_with(&device, &queue, &program, &expressions, None);
        let middle = &pixels[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4];
        assert_eq!(middle, [51, 0, 255, 255]);
    }

    /// A pack's outline importing Polytone's built-in `polytone_globals.glsl` reads the
    /// `PolyGlobals` the frame gives, beside its expression uniform: the player's block height
    /// (64) in red, `Tint` (its file's `0.1 * 2`, evaluated: 51) in green; zeros when none is
    /// given.
    #[test]
    #[ignore = "daily"]
    fn a_pack_outline_reads_poly_globals() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping");
            return;
        };
        let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
                   layout(std140) uniform Tint {\n    float value;\n};\n\
                   layout(location = 2) in vec4 vertexColor;\n\
                   layout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, value, \
                   vertexColor.b, 1.0);\n}\n";
        let with_globals = |id: &str| {
            include(id).or_else(|| {
                sunrise_polytone::globals::is_include(id)
                    .then(|| sunrise_polytone::globals::GLSL.to_owned())
            })
        };
        let globals = sunrise_polytone::globals::BLOCK;
        let program = crate::core_shader::translate_core_with(
            LINES_VSH,
            fsh,
            &[],
            &with_globals,
            &GLUE,
            &|block| block == "Tint" || block == globals,
        )
        .expect("the shaders translate");
        let id = super::super::expression_blocks::LINES[0];
        let modifiers = modifiers(r#"{"Tint": "0.1 * 2"}"#);
        let expressions = modifiers.resolve((id, id), &program, &[globals]).unwrap();
        let bytes = sunrise_polytone::globals::Globals {
            player: [0.5, 64.9, 0.5],
            ..Default::default()
        }
        .std140();
        let middle = |given| {
            let pixels = draw_edge_with(&device, &queue, &program, &expressions, given);
            pixels[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4].to_vec()
        };
        assert_eq!(middle(Some(&bytes)), [64, 51, 255, 255]);
        assert_eq!(middle(None), [0, 51, 255, 255]);
    }

    /// The packs' `shader_modifiers/core/rendertype_lines.json`, `file`.
    fn modifiers(file: &str) -> crate::polytone_post::ShaderModifiers {
        let pack = sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/polytone/shader_modifiers/core/rendertype_lines.json".to_owned(),
                sunrise_assets::resource_source::Resource::bytes(file.as_bytes().to_vec()),
            )]
            .into_iter()
            .collect(),
        );
        crate::polytone_post::ShaderModifiers::read(&pack, Default::default())
    }

    /// The stroke's middle takes the pack's colour (`GameTime` red, its inverse green, the
    /// vertex colour's blue); a pixel far from it stays clear.
    fn assert_pack_coloured(pixels: &[u8]) {
        let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        for y in [31, 32] {
            let pixel = at(32, y);
            let want = [64, 191, 255, 255];
            assert!(
                pixel.iter().zip(want).all(|(&p, w)| p.abs_diff(w) <= 1),
                "row {y}: {pixel:?}"
            );
        }
        assert_eq!(at(32, 4), [0, 0, 0, 0]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_pack_fragment_shader_colours_the_outline() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping");
            return;
        };
        let program = crate::core_shader::translate_core(LINES_VSH, RGB_FSH, &[], &include, &GLUE)
            .expect("the shaders translate");
        assert_pack_coloured(&draw_edge(&device, &queue, &program));
    }

    /// A pack's own include with a bigger block than vanilla's (an older `DynamicTransforms`)
    /// is not used, and the outline draws natively, rather than failing at the draw.
    #[test]
    #[ignore = "daily"]
    fn a_pack_block_bigger_than_vanillas_falls_back() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((device, _)) = device() else {
            return;
        };
        let bigger = |id: &str| {
            include(id)
                .map(|text| text.replace("vec3 ModelOffset;", "vec3 ModelOffset;\n    vec4 More;"))
        };
        let program = crate::core_shader::translate_core(LINES_VSH, RGB_FSH, &[], &bigger, &GLUE)
            .expect("the shaders translate");
        assert!(CoreLines::new(&device, wgpu::TextureFormat::Rgba8Unorm, &program, &[]).is_none());
    }

    /// Vanilla's own `rendertype_lines.vsh` (from the prepared resources) under a pack's
    /// fragment shader.
    #[test]
    #[ignore = "daily"]
    fn vanillas_lines_vertex_shader_takes_a_packs_fragment_shader() {
        let _gpu = crate::gfx::test_gpu::lock();
        let root = std::env::var_os("SUNRISE_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let vertex = std::fs::read_to_string(
            root.join("assets/minecraft/shaders/core/rendertype_lines.vsh"),
        )
        .expect("prepared vanilla rendertype_lines.vsh");
        let include = |id: &str| {
            let (namespace, path) = id.split_once(':')?;
            std::fs::read_to_string(root.join(format!("assets/{namespace}/{path}"))).ok()
        };
        let program = crate::core_shader::translate_core(&vertex, RGB_FSH, &[], &include, &GLUE)
            .expect("vanilla's vertex shader translates");
        // A pack that replaces only the vertex shader runs vanilla's fragment shader.
        let fragment = include("minecraft:shaders/core/rendertype_lines.fsh").unwrap();
        let vanilla = crate::core_shader::translate_core(&vertex, &fragment, &[], &include, &GLUE)
            .expect("vanilla's fragment shader translates");
        let Some((device, queue)) = device() else {
            eprintln!("no GPU: skipping the draw");
            return;
        };
        assert_pack_coloured(&draw_edge(&device, &queue, &program));
        assert!(CoreLines::new(&device, wgpu::TextureFormat::Rgba8Unorm, &vanilla, &[]).is_some());
    }
}
