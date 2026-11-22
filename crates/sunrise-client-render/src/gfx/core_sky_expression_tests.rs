//! A pack's sky and stars reading expression uniforms (`shader_modifiers`): each pipeline's
//! values reach its own blocks through [`CoreSky::prepare`], after the pipeline's own, each
//! resolved from its own shader's file.
use super::*;

/// The first triangle covers the target, every other one is degenerate, whatever the disc's or
/// the stars' positions.
const VSH: &str = "#version 330\nlayout(location = 0) in vec3 Position;\n\
    const vec2 CORNERS[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));\n\
    void main() {\n    gl_Position = int(gl_VertexIndex) < 3 \
    ? vec4(CORNERS[int(gl_VertexIndex)] + 0.0 * Position.xy, 0.5, 1.0) : vec4(0.0);\n}\n";
/// The `Tint` expression uniform in red.
const FSH: &str = "#version 330\nlayout(std140) uniform Tint {\n    float value;\n};\n\
    layout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = vec4(value, 0.0, 0.0, 1.0);\n}\n";

/// One pass of the disc or the stars (`stars`) after `CoreSky::prepare` uploaded the blocks
/// (and `poly_globals`): the red of the target's middle.
fn red(device: &wgpu::Device, queue: &wgpu::Queue, sky: &mut CoreSky, stars: bool) -> u8 {
    pixel(device, queue, sky, stars, None)[0]
}

/// [`red`]'s whole pixel, with `poly_globals` uploaded.
fn pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sky: &mut CoreSky,
    stars: bool,
    poly_globals: Option<&super::super::expression_blocks::PolyGlobalsBytes>,
) -> [u8; 4] {
    const SIZE: u32 = 16;
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
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(256 * SIZE),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut uploads = super::super::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let blocks = uniforms(
        [0; 48],
        Mat4::IDENTITY,
        Mat4::IDENTITY,
        [1.0; 4],
        [0.0; 4],
        [1e6; 6],
    );
    for extra in sky.expression_blocks() {
        extra.set_globals(poly_globals);
    }
    sky.prepare(&mut uploads, &mut encoder, &blocks, Some(&blocks));
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
        if stars {
            sky.draw_stars(&mut pass);
        } else {
            sky.draw_disc(&mut pass);
        }
    }
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE),
            },
        },
        color.size(),
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
    let at = 8 * 256 + 8 * 4;
    pixels[at..at + 4].try_into().unwrap()
}

/// The disc and the stars each read their own `Tint`, from their own shader's file (the same
/// name in both): each its own value, in a block after vanilla's four. Values a unorm byte
/// holds exactly (0.2 is 51, 0.6 is 153).
#[test]
#[ignore = "daily"]
fn a_pack_sky_and_stars_read_their_expression_uniforms() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let program =
        crate::core_shader::translate_core_with(VSH, FSH, &[], &|_| None, &GLUE, &|block| {
            block == "Tint"
        })
        .expect("the shaders translate");
    let file = |value: &str| {
        sunrise_assets::resource_source::Resource::bytes(
            format!("{{\"Tint\": \"{value}\"}}").into_bytes(),
        )
    };
    let pack = sunrise_assets::resource_source::ResourceSource::indexed(
        [
            (
                "assets/minecraft/polytone/shader_modifiers/core/sky.json".to_owned(),
                file("0.2"),
            ),
            (
                "assets/minecraft/polytone/shader_modifiers/core/stars.json".to_owned(),
                file("0.6"),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let modifiers = crate::polytone_post::ShaderModifiers::read(&pack, Default::default());
    let resolve = |id: &str| modifiers.resolve((id, id), &program, &[]).unwrap();
    let (disc, stars) = (
        resolve(super::super::expression_blocks::SKY[0]),
        resolve(super::super::expression_blocks::STARS[0]),
    );
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut sky = CoreSky::new(
        &device,
        format,
        Some((&program, &disc)),
        Some((&program, &stars)),
    )
    .expect("the pipelines");
    for blocks in sky.expression_blocks() {
        blocks.evaluate(&Default::default());
    }
    assert_eq!(red(&device, &queue, &mut sky, false), 51);
    assert_eq!(red(&device, &queue, &mut sky, true), 153);
}

/// Polytone's `PolyGlobals`, imported from the built-in `polytone_globals.glsl` beside an
/// expression uniform: the disc and the stars each read the block's bytes as the game gives
/// them (the player's block at x 153, y 64), and their own `Tint` in its block after it.
#[test]
#[ignore = "daily"]
fn a_pack_sky_reads_poly_globals() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let globals_fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
        layout(std140) uniform Tint {\n    float value;\n};\n\
        layout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, value, \
        float(PolyPlayerBlockPos.x) / 255.0, 1.0);\n}\n";
    let include = |id: &str| {
        sunrise_polytone::globals::is_include(id)
            .then(|| sunrise_polytone::globals::GLSL.to_owned())
    };
    let provided = |block: &str| block == "Tint" || block == sunrise_polytone::globals::BLOCK;
    let disc_program =
        crate::core_shader::translate_core_with(VSH, globals_fsh, &[], &include, &GLUE, &provided)
            .expect("the shaders translate");
    let pack = sunrise_assets::resource_source::ResourceSource::indexed(
        [
            (
                "assets/minecraft/polytone/shader_modifiers/core/sky.json".to_owned(),
                sunrise_assets::resource_source::Resource::bytes(b"{\"Tint\": \"0.2\"}".to_vec()),
            ),
            (
                "assets/minecraft/polytone/shader_modifiers/core/stars.json".to_owned(),
                sunrise_assets::resource_source::Resource::bytes(b"{\"Tint\": \"0.6\"}".to_vec()),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let modifiers = crate::polytone_post::ShaderModifiers::read(&pack, Default::default());
    let own = [sunrise_polytone::globals::BLOCK];
    let resolve = |id: &str, program| modifiers.resolve((id, id), program, &own).unwrap();
    let disc = resolve(super::super::expression_blocks::SKY[0], &disc_program);
    let stars = resolve(super::super::expression_blocks::STARS[0], &disc_program);
    let mut sky = CoreSky::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        Some((&disc_program, &disc)),
        Some((&disc_program, &stars)),
    )
    .expect("the pipelines");
    let reads: Vec<bool> = sky.expression_blocks().map(|b| b.reads_globals()).collect();
    assert_eq!(reads, [true, true]);
    for blocks in sky.expression_blocks() {
        blocks.evaluate(&Default::default());
    }
    let globals = sunrise_polytone::globals::Globals {
        player: [153.5, 64.0, -2.5],
        ..Default::default()
    }
    .std140();
    let mut drawn = |stars, given| pixel(&device, &queue, &mut sky, stars, given);
    assert_eq!(drawn(false, Some(&globals)), [64, 51, 153, 255]);
    assert_eq!(drawn(true, Some(&globals)), [64, 153, 153, 255]);
    // No globals given: the block reads zeros.
    assert_eq!(drawn(false, None), [0, 51, 0, 255]);
    assert_eq!(drawn(true, None), [0, 153, 0, 255]);
}
