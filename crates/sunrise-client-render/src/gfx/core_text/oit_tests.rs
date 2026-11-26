//! The pack's `OIT_TEXT` stages: what each binds, when its groups are made again, and which
//! programs leave the stages native or fail the load.
use super::*;
use crate::gfx::oit::{CloudSamples, Stage};

const SIZE: u32 = 8;

/// A vertex stage that places the quad where the vertices say (over the whole target here).
const VSH: &str = "#version 330\nlayout(location = 0) in vec3 Position;\n\
    void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n";

/// Each stage writes what it reads, so a wrong binding shows in the targets: the glyphs'
/// (`Sampler0`) red, the lightmap's (`Sampler2`) green, its own expression uniform (`Bounds`,
/// `Trans`, `Acc`: each stage's block in its own place of the buffer) and the stage samplers it
/// is given (`DepthBoundsSampler`; `Coeff0` and `Coeff1` in the accumulate stage).
const FSH: &str = "#version 330\nuniform sampler2D Sampler0;\nuniform sampler2D Sampler2;\n\
    #if defined(OIT_TRANSMITTANCE) || defined(OIT_ACCUMULATE)\nuniform sampler2D DepthBoundsSampler;\n#endif\n\
    #ifdef OIT_ACCUMULATE\nuniform sampler2D Coeff0;\nuniform sampler2D Coeff1;\n#endif\n\
    #ifdef OIT_DEPTH_BOUNDS\nlayout(std140) uniform Bounds {\n    float value;\n};\n\
    #elif defined(OIT_TRANSMITTANCE)\nlayout(std140) uniform Trans {\n    float value;\n};\n\
    #elif defined(OIT_ACCUMULATE)\nlayout(std140) uniform Acc {\n    float value;\n};\n#endif\n\
    layout(location = 0) out vec4 fragColor;\n\
    #ifdef OIT_TRANSMITTANCE\nlayout(location = 1) out vec4 second;\n#endif\n\
    void main() {\n#ifndef OIT\n    fragColor = vec4(1.0);\n#else\n\
        float glyph = texture(Sampler0, vec2(0.5)).r;\n\
        float light = texture(Sampler2, vec2(0.5)).g;\n\
    #if defined(OIT_DEPTH_BOUNDS)\n    fragColor = vec4(glyph, light, value, 0.0);\n\
    #elif defined(OIT_TRANSMITTANCE)\n    vec4 bounds = texture(DepthBoundsSampler, vec2(0.5));\n\
        fragColor = vec4(glyph, light, value, bounds.x);\n    second = vec4(bounds.y, 0.0, 0.0, 0.0);\n\
    #else\n    vec4 bounds = texture(DepthBoundsSampler, vec2(0.5));\n\
        fragColor = vec4(value + glyph, bounds.z + light, texture(Coeff0, vec2(0.5)).x, \
    texture(Coeff1, vec2(0.5)).y);\n#endif\n#endif\n}\n";

/// The text shader's file: each stage's value, exact in a half float.
const MODIFIERS: &str = r#"{"Bounds": "0.125 * 2", "Trans": "0.25 * 2", "Acc": "0.25 * 3"}"#;

/// The pack's `OIT_TEXT` programs and their expression uniforms, loaded as the atlas loads them.
fn pack_programs(fsh: &str) -> Box<super::super::OitPrograms> {
    use sunrise_assets::resource_source::ResourceSource;
    let pack = crate::hud::pack_text::text_tests::source(&[
        ("assets/minecraft/shaders/core/text.vsh", VSH.to_owned()),
        ("assets/minecraft/shaders/core/text.fsh", fsh.to_owned()),
        (
            "assets/minecraft/polytone/shader_modifiers/core/text.json",
            MODIFIERS.to_owned(),
        ),
    ]);
    let mut atlas = crate::hud::HudAtlas::boot();
    atlas.load_pack_shaders(&pack, &ResourceSource::default());
    atlas.text_shaders.oit.expect("the three stages")
}

/// `fsh` translated for each stage as `hud::pack_text` translates it, without expression
/// uniforms.
fn stage_programs(fsh: &[&str; 3]) -> [Translated; 3] {
    std::array::from_fn(|i| {
        let stage = &crate::gfx::core_clouds::OIT_STAGES[i];
        crate::core_shader::translate_core_with(
            VSH,
            fsh[i],
            stage.defines,
            &|_| None,
            &super::super::WORLD_GLUE,
            &|name| stage.samplers.contains(&name),
        )
        .unwrap_or_else(|e| panic!("{}: {e:?}", stage.pipeline))
    })
}

/// A `SIZE` square texture of `format` every stage draws into or samples.
fn target(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// A stage sample of `texel` everywhere, as `Oit`'s targets are (`Rgba32Float` for the bounds; the
/// coefficients' `Rgba16Float` reads the same through a float sampler).
fn filled(device: &wgpu::Device, queue: &wgpu::Queue, texel: [f32; 4]) -> wgpu::TextureView {
    let texture = target(device, wgpu::TextureFormat::Rgba32Float);
    let texels: Vec<f32> = texel.repeat((SIZE * SIZE) as usize);
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 16),
            rows_per_image: Some(SIZE),
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

/// The middle texel of `texture`, as floats.
fn middle(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<f32> {
    let size = texture.format().block_copy_size(None).unwrap() as usize;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * u64::from(SIZE),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = readback.slice(..).get_mapped_range().unwrap();
    let at = 256 * (SIZE as usize / 2) + size * (SIZE as usize / 2);
    let bytes = &data[at..at + size];
    if size == 16 {
        bytemuck::pod_collect_to_vec(bytes)
    } else {
        (bytes.chunks(2))
            .map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect()
    }
}

/// `oit`'s `stage` over the whole target after [`OitPart::prepare`] uploaded zeros for vanilla's
/// blocks and its expression uniforms' values, into the stage's targets (`Oit`'s formats) cleared
/// as `Oit` clears them, over a depth cleared to the far plane: each target's middle texel.
fn draw(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    oit: &OitPart,
    stage: Stage,
) -> Vec<Vec<f32>> {
    let corner = |x: f32, y: f32| crate::mesher::Vertex {
        pos: [x, y, 0.5],
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    };
    let quad = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let vertices = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&quad),
            usage: wgpu::BufferUsages::VERTEX,
        },
    );
    let (formats, clears): (Vec<_>, Vec<_>) = match stage {
        Stage::DepthBounds => (
            vec![wgpu::TextureFormat::Rgba32Float],
            vec![wgpu::Color {
                r: -(f32::MAX as f64),
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }],
        ),
        Stage::Transmittance => (
            vec![wgpu::TextureFormat::Rgba16Float; 2],
            vec![wgpu::Color::TRANSPARENT; 2],
        ),
        Stage::Accumulate => (
            vec![wgpu::TextureFormat::Rgba16Float],
            vec![wgpu::Color::TRANSPARENT],
        ),
    };
    let targets: Vec<wgpu::Texture> = formats.iter().map(|f| target(device, *f)).collect();
    let views: Vec<wgpu::TextureView> = (targets.iter())
        .map(|t| t.create_view(&Default::default()))
        .collect();
    let depth = target(device, wgpu::TextureFormat::Depth32Float);
    let depth_view = depth.create_view(&Default::default());
    let mut uploads = crate::gfx::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    oit.prepare(&mut uploads, &mut encoder, &[0u8; VANILLA_BYTES as usize]);
    {
        let attachments: Vec<_> = (views.iter().zip(&clears))
            .map(|(view, clear)| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(*clear),
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &attachments,
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
        let (_, groups) = oit.groups.as_ref().expect("bound");
        pass.set_pipeline(&oit.stages[stage as usize].pipeline);
        pass.set_bind_group(0, &groups[stage as usize], &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    targets.iter().map(|t| middle(device, queue, t)).collect()
}

/// The pack's three stages, over a glyph texel of red 255 and a lightmap texel of green 255.
fn part(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    programs: [(&Translated, &[crate::polytone_post::ExpressionUniform]); 3],
) -> Result<Option<Box<OitPart>>, String> {
    let atlas = super::super::tests::texture(device, queue, [255, 0, 0, 255]);
    let lightmap = super::super::tests::texture(device, queue, [0, 255, 0, 255]);
    let sampler = device.create_sampler(&Default::default());
    OitPart::new(device, programs, &atlas, (&lightmap, &sampler))
}

/// Each stage reads the glyphs, the lightmap, its own expression uniform and the stage samples it
/// is given:
/// - bounds: glyph 1, light 1, `Bounds` 0.25, over the bounds' clear by MAX;
/// - transmittance: glyph 1, light 1, `Trans` 0.5, the depth bounds' 0.125, and its 0.375 into the
///   second target;
/// - accumulate: `Acc` 0.75 + glyph 1, the bounds' 0.625 + light 1, `Coeff0`'s 0.875 and
///   `Coeff1`'s 0.0625.
#[test]
#[ignore = "daily"]
fn each_improved_transparency_stage_reads_its_own_inputs() {
    let _gpu = crate::gfx::test_gpu::lock();
    let programs = pack_programs(FSH);
    let Some((device, queue)) = crate::gfx::test_gpu::oit_device() else {
        return;
    };
    let mut oit = part(
        &device,
        &queue,
        std::array::from_fn(|i| (&*programs.programs[i], &programs.expressions[i][..])),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("the stages");
    for stage in &mut oit.stages {
        stage.extra.evaluate(&Default::default());
    }
    let bounds = filled(&device, &queue, [0.125, 0.375, 0.625, 0.0]);
    let coeff0 = filled(&device, &queue, [0.875, 0.0, 0.0, 0.0]);
    let coeff1 = filled(&device, &queue, [0.0, 0.0625, 0.0, 0.0]);
    oit.bind(
        &device,
        &CloudSamples {
            generation: 1,
            views: [&bounds, &coeff0, &coeff1],
        },
    );
    let gpu = (&device, &queue);
    assert_eq!(
        draw(gpu, &oit, Stage::DepthBounds),
        [vec![1.0, 1.0, 0.25, 0.0]]
    );
    assert_eq!(
        draw(gpu, &oit, Stage::Transmittance),
        [vec![1.0, 1.0, 0.5, 0.125], vec![0.375, 0.0, 0.0, 0.0]]
    );
    assert_eq!(
        draw(gpu, &oit, Stage::Accumulate),
        [vec![1.75, 1.625, 0.875, 0.0625]]
    );
}

/// The groups follow the targets they bind: new targets (generation 2) are read, and the same
/// generation again keeps the groups made for it (views passed with it are not bound again).
#[test]
#[ignore = "daily"]
fn the_groups_follow_new_improved_transparency_targets() {
    let _gpu = crate::gfx::test_gpu::lock();
    let programs = pack_programs(FSH);
    let Some((device, queue)) = crate::gfx::test_gpu::oit_device() else {
        return;
    };
    let mut oit = part(
        &device,
        &queue,
        std::array::from_fn(|i| (&*programs.programs[i], &programs.expressions[i][..])),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("the stages");
    let zero = filled(&device, &queue, [0.0; 4]);
    let gpu = (&device, &queue);
    // The transmittance stage's bounds texel (alpha of the first target, red of the second).
    let read = |oit: &OitPart| {
        let texels = draw(gpu, oit, Stage::Transmittance);
        (texels[0][3], texels[1][0])
    };
    let first = filled(&device, &queue, [0.125, 0.25, 0.0, 0.0]);
    let bind = |oit: &mut OitPart, generation, view| {
        oit.bind(
            &device,
            &CloudSamples {
                generation,
                views: [view, &zero, &zero],
            },
        )
    };
    bind(&mut oit, 1, &first);
    assert_eq!(read(&oit), (0.125, 0.25));
    let second = filled(&device, &queue, [0.5, 0.75, 0.0, 0.0]);
    bind(&mut oit, 2, &second);
    assert_eq!(read(&oit), (0.5, 0.75), "new targets");
    let third = filled(&device, &queue, [1.0, 1.0, 0.0, 0.0]);
    bind(&mut oit, 2, &third);
    assert_eq!(
        read(&oit),
        (0.5, 0.75),
        "the same targets keep their groups"
    );
}

/// A program past what the device takes in its stage's targets (an unsigned output into the
/// accumulate stage's float target, `PastDeviceLimit::ColourOutput`) leaves all three stages
/// native: no part, and no refusal.
#[test]
#[ignore = "daily"]
fn a_stage_past_the_device_limits_leaves_every_stage_native() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = crate::gfx::test_gpu::oit_device() else {
        return;
    };
    let fits = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = vec4(1.0); }\n";
    let unsigned = "#version 330\nlayout(location = 0) out uvec4 fragColor;\n\
        void main() { fragColor = uvec4(1u); }\n";
    let programs = stage_programs(&[fits, fits, unsigned]);
    let made = part(
        &device,
        &queue,
        std::array::from_fn(|i| (&programs[i], &[][..])),
    );
    assert!(matches!(made, Ok(None)), "{:?}", made.err());
}

/// The device refusing one stage's pipeline fails the load, named by that stage's location. The
/// refusal is one vanilla's builder makes too: `Sampler1`, which no text pipeline binds.
#[test]
#[ignore = "daily"]
fn a_stage_the_device_refuses_fails_the_load_named_by_its_pipeline() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = crate::gfx::test_gpu::oit_device() else {
        return;
    };
    let fits = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = vec4(1.0); }\n";
    let refused = "#version 330\nuniform sampler2D Sampler1;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = texture(Sampler1, vec2(0.5)); }\n";
    for (i, pipeline) in [
        "minecraft:pipeline/oit_depth_bounds_text",
        "minecraft:pipeline/oit_transmittance_text",
        "minecraft:pipeline/oit_accumulate_text",
    ]
    .into_iter()
    .enumerate()
    {
        let mut fsh = [fits; 3];
        fsh[i] = refused;
        let programs = stage_programs(&fsh);
        let error = part(
            &device,
            &queue,
            std::array::from_fn(|i| (&programs[i], &[][..])),
        )
        .err()
        .expect("the device refuses it");
        assert_eq!(error.split_once(": ").map(|(p, _)| p), Some(pipeline));
    }
}
