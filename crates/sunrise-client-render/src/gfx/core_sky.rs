//! A resource pack's `core/sky` and `core/stars` shaders: the overworld's sky disc and stars drawn
//! with the pack's GLSL (`SkyRenderer.renderSkyDisc`, `renderStars`), as a pack that adds a haze
//! to the horizon or tints the night sky expects. Without one the sky draws natively (`fs_sky` in
//! terrain.wgsl, stars.rs).

use crate::polytone_post::ExpressionUniform;
use glam::Mat4;
use sunrise_glsl::{Glue, Translated};
use wgpu::util::DeviceExt;

/// `DefaultVertexFormat.POSITION`: the disc's and the stars' vertices.
pub(crate) const GLUE: Glue = Glue {
    inputs: &[("Position", "vec3")],
    prelude: "layout(location = 0) in vec3 hz_position;",
    setup: "Position = hz_position;",
    // 26.3's ProjMat already has wgpu's reversed 0 to 1 depth (Projection: zZeroToOne).
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// `SkyRenderer.buildSkyDisc(16)`: a fan around the point 16 above the camera, out to 512 every
/// 45 degrees, as a triangle list.
pub(crate) fn disc_triangles() -> Vec<[f32; 3]> {
    let rim: Vec<[f32; 3]> = (-180..=180)
        .step_by(45)
        .map(|i: i32| {
            let a = (i as f32).to_radians();
            [512.0 * a.cos(), 16.0, 512.0 * a.sin()]
        })
        .collect();
    rim.windows(2)
        .flat_map(|edge| [[0.0, 16.0, 0.0], edge[0], edge[1]])
        .collect()
}

/// One of the sky's pack pipelines with its vertices and uniforms.
struct Part {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// The packs' expression uniforms its stages read (`shader_modifiers`).
    extra: super::expression_blocks::ExpressionBlocks,
    vertices: wgpu::Buffer,
    count: u32,
}

impl Part {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        (program, expressions): (&Translated, &[ExpressionUniform]),
        (name, location): (&str, &str),
        blend: Option<wgpu::BlendState>,
        triangles: &[[f32; 3]],
    ) -> Result<Self, String> {
        let (pipeline, group, uniform, extra) =
            crate::core_shader::scoped(device, location, || {
                Self::pipeline(device, format, (program, expressions), name, blend)
            })?;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(name),
            contents: bytemuck::cast_slice(triangles),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Ok(Self {
            pipeline,
            group,
            uniform,
            extra,
            vertices,
            count: triangles.len() as u32,
        })
    }

    /// The pipeline of `program` with its group, its uniform buffer and its Polytone blocks.
    fn pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        (program, expressions): (&Translated, &[ExpressionUniform]),
        name: &str,
        blend: Option<wgpu::BlendState>,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::BindGroup,
        wgpu::Buffer,
        super::expression_blocks::ExpressionBlocks,
    ) {
        let module = |stage, module: &naga::Module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("Resource pack {name} ({stage})")),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module.clone())),
            })
        };
        let vertex = module("vertex", &program.vertex);
        let fragment = module("fragment", &program.fragment);
        // Vanilla's SKY and STARS pipelines have no samplers: the device refuses a pack's that
        // reads one.
        // Each block a stage reads is bound to the stages reading it: vanilla's four, then the
        // expression uniforms.
        let extra = super::expression_blocks::ExpressionBlocks::new(program, expressions, 1024);
        let blocks = super::expression_blocks::vanilla_blocks(program);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(name),
            size: extra.size(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (entries, resources) = super::expression_blocks::bindings(&blocks, &extra, &uniform);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(name),
            entries: &entries,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &layout,
            entries: &resources,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            // The sky is drawn first, behind everything: no depth test, no depth write.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
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
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        (pipeline, group, uniform, extra)
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

/// The packs' sky disc and stars pipelines, each present when a pack replaces that shader.
pub(crate) struct CoreSky {
    disc: Option<Part>,
    stars: Option<Part>,
}

/// The packs' sky and stars on the renderer.
impl super::Renderer {
    /// The resource packs' `core/sky` and `core/stars` (`HudAtlas::sky_shader`,
    /// `stars_shader`, each with the expression uniforms resolved for it), or the built-in sky
    /// and stars; or the device's refusal of a pipeline vanilla requires, which fails the pack
    /// reload.
    pub fn set_sky_shaders(
        &mut self,
        disc: Option<(&Translated, &[ExpressionUniform])>,
        stars: Option<(&Translated, &[ExpressionUniform])>,
    ) -> Result<(), String> {
        self.core_sky = sky_for_frame(&self.device, self.output.frame, disc, stars)?;
        Ok(())
    }
}

/// As `core_lines::lines_for_frame`: the pipelines are made whatever the frame draws, then kept
/// only off a frame of linear light.
pub(super) fn sky_for_frame(
    device: &wgpu::Device,
    frame: super::FrameConfiguration,
    disc: Option<(&Translated, &[ExpressionUniform])>,
    stars: Option<(&Translated, &[ExpressionUniform])>,
) -> Result<Option<CoreSky>, String> {
    let sky = CoreSky::new(device, frame.format, disc, stars)?;
    Ok(sky.filter(|_| !frame.linear_light))
}

impl CoreSky {
    /// `None` when neither shader is replaced; the device's refusal of either pipeline, named
    /// by its location.
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        disc: Option<(&Translated, &[ExpressionUniform])>,
        stars: Option<(&Translated, &[ExpressionUniform])>,
    ) -> Result<Option<Self>, String> {
        use crate::hud::pack_text::{SKY, STARS};
        // A program past the device's limits draws natively (`sunrise_glsl::PastDeviceLimit`).
        let fits = |pipeline| {
            move |(program, _): &(&Translated, &[ExpressionUniform])| {
                crate::core_shader::within_device_limits(device, program, pipeline, &[format])
            }
        };
        let disc = disc.filter(fits(SKY[0].pipeline));
        let stars = stars.filter(fits(STARS[0].pipeline));
        let disc = disc.map(|program| {
            // RenderPipelines.SKY: ColorTargetState.DEFAULT, no blending.
            let name = ("sky", SKY[0].pipeline);
            Part::new(device, format, program, name, None, &disc_triangles())
        });
        let disc = disc.transpose()?;
        let stars = stars.map(|program| {
            // RenderPipelines.STARS: BlendFunction.OVERLAY (src alpha and one; alpha one, zero).
            let overlay = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            let triangles = super::stars::star_triangles();
            let name = ("stars", STARS[0].pipeline);
            Part::new(device, format, program, name, Some(overlay), &triangles)
        });
        let stars = stars.transpose()?;
        Ok((disc.is_some() || stars.is_some()).then_some(Self { disc, stars }))
    }

    pub(super) fn draws_disc(&self) -> bool {
        self.disc.is_some()
    }

    pub(super) fn draws_stars(&self) -> bool {
        self.stars.is_some()
    }

    /// Uploads the disc's and the stars' blocks: `sky` and `stars` from [`uniforms`], then each
    /// one's Polytone blocks (the frame's `PolyGlobals` and its expression uniforms).
    pub(super) fn prepare(
        &self,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        sky: &[u8],
        stars: Option<&[u8]>,
    ) {
        if let Some(disc) = &self.disc {
            uploads.buffer(encoder, &disc.uniform, 0, sky);
            disc.extra.upload(uploads, encoder, &disc.uniform);
        }
        if let (Some(part), Some(data)) = (&self.stars, stars) {
            uploads.buffer(encoder, &part.uniform, 0, data);
            part.extra.upload(uploads, encoder, &part.uniform);
        }
    }

    /// The disc's and the stars' expression uniforms.
    pub(super) fn expression_blocks(
        &mut self,
    ) -> impl Iterator<Item = &mut super::expression_blocks::ExpressionBlocks> {
        [self.disc.as_mut(), self.stars.as_mut()]
            .into_iter()
            .flatten()
            .map(|part| &mut part.extra)
    }

    /// The disc, in place of the native sky colour. The caller sets its own group 0 again after.
    pub(super) fn draw_disc(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(disc) = &self.disc {
            disc.draw(pass);
        }
    }

    /// The stars, in place of the native ones. The caller sets its own group 0 again after.
    pub(super) fn draw_stars(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(stars) = &self.stars {
            stars.draw(pass);
        }
    }
}

/// The std140 blocks for one sky draw: `Globals`, `Projection` (`projection`, far enough for the
/// disc's 512-block rim), `DynamicTransforms` (`model_view`: the camera's rotation, times the
/// stars' rotation for them; `color`: the sky colour, or the star brightness in every channel)
/// and `Fog` (`fog_color`, `fog`: environmental start and end, render distance start and end,
/// sky end, clouds end).
pub(super) fn uniforms(
    globals: [u8; 48],
    projection: Mat4,
    model_view: Mat4,
    color: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 6],
) -> Vec<u8> {
    let mut data = vec![0f32; 256];
    data[64..80].copy_from_slice(&projection.to_cols_array());
    let transforms = 128;
    data[transforms..transforms + 16].copy_from_slice(&model_view.to_cols_array());
    data[transforms + 16..transforms + 32].copy_from_slice(&Mat4::IDENTITY.to_cols_array());
    data[transforms + 32..transforms + 36].copy_from_slice(&color);
    data[192..196].copy_from_slice(&fog_color);
    data[196..202].copy_from_slice(&fog);
    let mut bytes: Vec<u8> = bytemuck::cast_slice(&data).to_vec();
    bytes[..48].copy_from_slice(&globals);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `buildSkyDisc(16)`: eight triangles from the point above the camera to the rim, which
    /// starts and ends at x -512 (cos 180°) and passes through z -512, x 512 and z 512.
    #[test]
    fn the_disc_is_vanillas_fan_as_triangles() {
        let triangles = disc_triangles();
        assert_eq!(triangles.len(), 24);
        assert!(triangles.chunks(3).all(|t| t[0] == [0.0, 16.0, 0.0]));
        let rim: Vec<[f32; 3]> = triangles.chunks(3).map(|t| t[1]).collect();
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(near(rim[0], [-512.0, 16.0, 0.0]));
        assert!(near(rim[2], [0.0, 16.0, -512.0]));
        assert!(near(rim[4], [512.0, 16.0, 0.0]));
        assert!(near(rim[6], [0.0, 16.0, 512.0]));
        assert!(near(triangles[23], [-512.0, 16.0, 0.0]));
    }

    /// The sky's and the stars' pipelines are made on a frame of linear light too, which keeps
    /// the built-in sky: the device refusing either (a fragment stage sampling `Sampler0`, which
    /// these pipelines don't bind) fails the load in both modes, named by that pipeline, and pipelines it takes
    /// are drawn only off linear light. A varying read `noperspective` or `centroid` is taken.
    #[test]
    #[ignore = "daily"]
    fn linear_light_makes_the_sky_pipelines_before_keeping_the_built_in_sky() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, _)) = crate::gfx::test_gpu::optional_device() else {
            eprintln!("skipped: no adapter");
            return;
        };
        let vertex = "#version 330\nlayout(location = 0) in vec3 Position;\n\
                      void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n";
        let translate = |fragment: &str| {
            crate::core_shader::translate_core(vertex, fragment, &[], &|_| None, &GLUE)
                .expect("the shaders translate")
        };
        let taken = translate(
            "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = vec4(1.0);\n}\n",
        );
        // A refusal vanilla's builder makes too (`PipelineBuilder`: a descriptor missing from the
        // pipeline's layout): `Sampler0`, which `SKY` and `STARS` don't bind.
        let refused = translate(
            "#version 330\nuniform sampler2D Sampler0;\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {\n    fragColor = texture(Sampler0, vec2(0.5));\n}\n",
        );
        let frame = |linear_light| super::super::FrameConfiguration {
            format: wgpu::TextureFormat::Rgba16Float,
            linear_light,
            ..super::super::EffectiveOutput::NONE.frame
        };
        let made = |linear, disc: &Translated, stars: &Translated| {
            sky_for_frame(
                &device,
                frame(linear),
                Some((disc, &[])),
                Some((stars, &[])),
            )
        };
        assert!(
            made(true, &taken, &taken).unwrap().is_none(),
            "the built-in sky"
        );
        // Vanilla's check compares only flat-ness: a varying the fragment stage reads
        // `noperspective` or `centroid` makes both pipelines.
        let varying = |qualifier: &str| {
            crate::core_shader::translate_core(
                "#version 330\nlayout(location = 0) in vec3 Position;\nlayout(location = 0) out vec2 tag;\n\
                 void main() {\n    gl_Position = vec4(Position, 1.0);\n    tag = vec2(1.0);\n}\n",
                &format!(
                    "#version 330\nlayout(location = 0) {qualifier} in vec2 tag;\n\
                     layout(location = 0) out vec4 fragColor;\n\
                     void main() {{\n    fragColor = vec4(tag, 0.0, 1.0);\n}}\n"
                ),
                &[],
                &|_| None,
                &GLUE,
            )
            .expect("the shaders translate")
        };
        for qualifier in ["noperspective", "centroid"] {
            let program = varying(qualifier);
            let sky = made(false, &program, &program).unwrap_or_else(|e| panic!("{e}"));
            assert!(
                sky.is_some_and(|s| s.draws_disc() && s.draws_stars()),
                "{qualifier}"
            );
        }
        // A colour output the device doesn't take into the 4-channel float target (`vec3`,
        // `uvec4`), which vanilla's builder doesn't check: that part draws natively, in both modes,
        // and the other is the pack's.
        for output in [
            "out vec3 fragColor;\nvoid main() {\n    fragColor = vec3(1.0);\n}\n",
            "out uvec4 fragColor;\nvoid main() {\n    fragColor = uvec4(1u);\n}\n",
        ] {
            let other = translate(&format!("#version 330\nlayout(location = 0) {output}"));
            for linear in [true, false] {
                let sky = made(linear, &other, &taken).unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(
                    sky.map(|s| (s.draws_disc(), s.draws_stars())),
                    (!linear).then_some((false, true)),
                    "{output}"
                );
            }
        }
        let drawn = made(false, &taken, &taken)
            .unwrap()
            .expect("the pack's sky");
        assert!(drawn.draws_disc() && drawn.draws_stars());
        for linear in [true, false] {
            for (disc, stars, pipeline) in [
                (&refused, &taken, "minecraft:pipeline/sky"),
                (&taken, &refused, "minecraft:pipeline/stars"),
            ] {
                let error = made(linear, disc, stars)
                    .err()
                    .expect("the device refuses it");
                assert_eq!(
                    error.split_once(": ").map(|(p, _)| p),
                    Some(pipeline),
                    "linear light {linear}"
                );
            }
        }
    }

    /// The prepared resources (`SUNRISE_RESOURCES`, or `.local/sunrise-client/<version>`), where
    /// vanilla's sky shader and its includes are.
    fn resources() -> std::path::PathBuf {
        std::env::var_os("SUNRISE_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            })
    }

    /// Vanilla's `sky.vsh` with a pack's fragment shader that paints `ColorModulator` (the sky
    /// colour) where the disc is: looking at the horizon, the top half of the target is the
    /// disc, the bottom half stays clear. The same holds with the pack's own older
    /// `dynamictransforms.glsl`, whose block has a member past 26.3's: vanilla's builder takes it
    /// (it checks a descriptor's name, type and dimension, not a block's size), and the block is
    /// bound at the shader's size.
    #[test]
    #[ignore = "daily"]
    fn a_pack_sky_shader_paints_the_disc_above_the_sunrise() {
        let _gpu = crate::gfx::test_gpu::lock();
        let root = resources();
        let vertex = std::fs::read_to_string(root.join("assets/minecraft/shaders/core/sky.vsh"))
            .expect("prepared vanilla sky.vsh");
        let include = |id: &str| {
            let (namespace, path) = id.split_once(':')?;
            std::fs::read_to_string(root.join(format!("assets/{namespace}/{path}"))).ok()
        };
        for older_block in [false, true] {
            let fragment = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
            layout(location = 0) out vec4 fragColor;\n\
            void main() {\n    fragColor = ColorModulator;\n}\n";
            let older = |id: &str| {
                include(id).map(|text| {
                    text.replace("vec3 ModelOffset;", "vec3 ModelOffset;\n    vec4 More;")
                })
            };
            let program = if older_block {
                crate::core_shader::translate_core(&vertex, fragment, &[], &older, &GLUE)
            } else {
                crate::core_shader::translate_core(&vertex, fragment, &[], &include, &GLUE)
            }
            .expect("the sky shaders translate");
            assert_eq!(
                program.declared_size(2),
                if older_block { 176 } else { 160 }
            );
            let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
                eprintln!("no GPU: skipping");
                return;
            };
            const SIZE: u32 = 64;
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let sky = CoreSky::new(&device, format, Some((&program, &[])), None)
                .unwrap_or_else(|e| panic!("{e}"))
                .expect("the sky pipeline");
            let projection = crate::gfx::world_projection(90.0, 1.0, 2048.0);
            let data = uniforms(
                [0; 48],
                projection,
                Mat4::IDENTITY,
                [0.25, 0.5, 1.0, 1.0],
                [0.0; 4],
                [1e6; 6],
            );
            queue.write_buffer(&sky.disc.as_ref().unwrap().uniform, 0, &data);
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
            let color = target(format, wgpu::TextureUsages::COPY_SRC);
            let depth = target(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::empty(),
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
                sky.draw_disc(&mut pass);
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
            let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
            let at = |x: usize, y: usize| pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].to_vec();
            // Row 0 is the top: the disc, in the sky colour; the bottom row is below the horizon.
            let top = at(32, 4);
            assert!(
                top.iter()
                    .zip([64u8, 128, 255, 255])
                    .all(|(&p, w)| p.abs_diff(w) <= 1),
                "{top:?} older block {older_block}"
            );
            assert_eq!(at(32, 60), vec![0, 0, 0, 0]);
        }
    }
}

#[cfg(test)]
#[path = "core_sky_expression_tests.rs"]
mod expression_tests;
