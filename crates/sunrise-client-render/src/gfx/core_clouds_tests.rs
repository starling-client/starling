use super::*;

const ALL: CloudsFrame = CloudsFrame {
    pack_shader: true,
    pack_oit_shader: true,
    clouds: true,
    native_clouds: true,
    improved_transparency: false,
    shader_pack: false,
    title: false,
    cloud_marks: false,
};

/// The pack's clouds draw only with a pack's shader and the clouds on, and only while nothing
/// else draws them its own way: each condition alone leaves the native mesh. Under Improved
/// Transparency they draw from the shader's `OIT_CLOUDS` programs, outside it from its
/// `CLOUDS` one; each without the other.
#[test]
fn a_packs_clouds_draw_only_where_nothing_else_draws_them() {
    assert!(ALL.pack_draws());
    let improved = CloudsFrame {
        improved_transparency: true,
        pack_shader: false,
        ..ALL
    };
    assert!(improved.pack_draws());
    let each = [
        CloudsFrame {
            pack_shader: false,
            ..ALL
        },
        CloudsFrame {
            pack_oit_shader: false,
            ..improved
        },
        CloudsFrame {
            clouds: false,
            ..ALL
        },
        CloudsFrame {
            native_clouds: false,
            ..ALL
        },
        CloudsFrame {
            shader_pack: true,
            ..ALL
        },
        CloudsFrame { title: true, ..ALL },
        CloudsFrame {
            cloud_marks: true,
            ..ALL
        },
    ];
    for (i, frame) in each.into_iter().enumerate() {
        assert!(!frame.pack_draws(), "condition {i}");
    }
}

/// The adapter's device, or none (said on stderr) where the machine has no adapter.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let device = crate::gfx::test_gpu::optional_device();
    if device.is_none() {
        eprintln!("skipped: no adapter");
    }
    device.map(|(_, device, queue)| (device, queue))
}

const SIZE: u32 = 8;

/// `program` as the classic clouds' only.
fn classic(
    program: &crate::core_shader::CoreProgram,
) -> (
    Option<&crate::core_shader::CoreProgram>,
    Option<&[crate::core_shader::CoreProgram; 3]>,
) {
    (Some(program), None)
}

/// Draws `core`'s prepared faces into an 8 by 8 target cleared to transparent black, with a
/// depth buffer cleared to the far plane (0, reversed Z), and reads it back.
fn draw(device: &wgpu::Device, queue: &wgpu::Queue, core: &CoreClouds, fancy: bool) -> Vec<u8> {
    let texture = |format, usage| {
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
            usage,
            view_formats: &[],
        })
    };
    let target = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let indices = crate::gfx::Renderer::quad_indices(device, core.quads.max(1));
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * u64::from(SIZE),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        core.draw(&mut pass, fancy, &indices);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = readback.slice(..).get_mapped_range().unwrap();
    (0..SIZE as usize)
        .flat_map(|row| data[row * 256..row * 256 + SIZE as usize * 4].to_vec())
        .collect()
}

/// `core`'s faces and blocks for `input`, uploaded through the renderer's belt.
fn prepare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    core: &mut CoreClouds,
    vanilla: &[u8; 1024],
    input: &crate::clouds::Input<'_>,
) -> Result<u32, TooManyFaces> {
    let mut uploads = super::super::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let quads = core.prepare(device, &mut uploads, &mut encoder, vanilla, input);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    quads
}

/// A one-cell texture at x -1 of the camera's cell, fast clouds, the colour opaque: one face,
/// `[-1, 0, -96]` (`clouds_pack_tests`).
fn flat_input(texture: &(u32, u32, Vec<bool>)) -> crate::clouds::Input<'_> {
    crate::clouds::Input {
        texture,
        uv: [0.0; 2],
        camera: glam::DVec3::new(0.0, f64::from(crate::clouds::CLOUD_HEIGHT) + 2.0, -3.96),
        time: 0,
        partial_tick: 0.0,
        range: 1,
        fast: true,
        color: [0.8, 1.0, 1.0, 1.0],
        height: crate::clouds::CLOUD_HEIGHT,
    }
}

/// A probe shader: the whole target covered by face 0's quad (wound clockwise, so a back face):
/// `CloudColor` in its left half, the face's three texels (plus 128) in red, green and blue in
/// its right half.
const PROBE_VSH: &str = "#version 330\n\
    layout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    vec3 CloudOffset;\n    vec3 CellSize;\n};\n\
    uniform isamplerBuffer CloudFaces;\n\
    layout(location = 0) out vec4 vertexColor;\nlayout(location = 1) out vec4 infoColor;\n\
    void main() {\n    int index = (gl_VertexIndex / 4) * 3;\n    int corner = gl_VertexIndex % 4;\n\
        ivec3 face = ivec3(texelFetch(CloudFaces, index).r, texelFetch(CloudFaces, index + 1).r,\n\
            texelFetch(CloudFaces, index + 2).r);\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? -1.0 : 1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n\
        vertexColor = vec4(vec3(face + 128) / 255.0, 1.0);\n    infoColor = CloudColor;\n}\n";
const PROBE_FSH: &str = "#version 330\nlayout(location = 0) in vec4 vertexColor;\n\
    layout(location = 1) in vec4 infoColor;\nlayout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = gl_FragCoord.x < 4.0 ? infoColor : vertexColor;\n}\n";

fn probe(device: &wgpu::Device) -> CoreClouds {
    let program = crate::core_shader::translate_core_with(
        PROBE_VSH,
        PROBE_FSH,
        &[],
        &|_| None,
        &GLUE,
        &|b| b == INFO_BLOCK || b == FACES,
    )
    .expect("the probe translates");
    CoreClouds::new(device, wgpu::TextureFormat::Rgba8Unorm, classic(&program))
        .expect("the pipelines")
}

/// The faces `clouds::pack` builds reach the pack's shader through `CloudFaces`, texel by
/// texel, and `CloudInfo` through its block: `[-1, 0, -96]` plus 128 is 127, 128, 32, and
/// `CloudColor` (0.8, 1, 1, 1) is 204, 255, 255, 255. Flat clouds don't cull, so the quad's
/// winding doesn't matter; fancy ones cull its back.
#[test]
#[ignore = "daily"]
fn a_pack_clouds_shader_reads_the_faces_and_cloud_info() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = probe(&device);
    let mut filled = vec![false; 16];
    filled[3] = true;
    let texture = (4, 4, filled);
    let input = flat_input(&texture);
    assert_eq!(
        prepare(&device, &queue, &mut core, &[0; 1024], &input),
        Ok(1)
    );
    let pixels = draw(&device, &queue, &core, false);
    for (i, texel) in pixels.chunks(4).enumerate() {
        let want = if i % (SIZE as usize) < 4 {
            [204, 255, 255, 255]
        } else {
            [127, 128, 32, 255]
        };
        assert_eq!(texel, want, "texel {i}");
    }
    // The same quad as fancy clouds: back-facing here, culled.
    let culled = draw(&device, &queue, &core, true);
    assert!(culled.iter().all(|&b| b == 0));
}

/// A frame's faces past what the device binds to one storage buffer are refused, named by
/// their own size (not the buffer's rounded one); ones that fit, to the byte, grow the buffer.
#[test]
#[ignore = "daily"]
fn faces_past_the_devices_storage_limit_are_refused() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = probe(&device);
    // Every cell filled, range 128: about 92 thousand cells of up to 12 faces.
    let texture = (256, 256, vec![true; 256 * 256]);
    let mut input = flat_input(&texture);
    input.fast = false;
    input.range = 128;
    let faces = crate::clouds::pack::faces(&input).len() as u64;
    let bytes = faces * 4;
    let limit = device.limits().max_storage_buffer_binding_size;
    assert_eq!(limit, 128 << 20, "wgpu's default limit");
    assert!(bytes <= limit, "{bytes} bytes fit");
    assert!(prepare(&device, &queue, &mut core, &[0; 1024], &input).is_ok());
    // Devices binding exactly the faces' bytes, and four bytes less: the first takes them, the
    // second refuses them, naming both sizes.
    let Some(adapter) = crate::gfx::test_gpu::adapter() else {
        return;
    };
    let binding = |limit: u64| {
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: limit,
                ..Default::default()
            },
            ..Default::default()
        }))
        .expect("a device binding less")
    };
    let (exact, exact_queue) = binding(bytes);
    let mut core = probe(&exact);
    assert!(prepare(&exact, &exact_queue, &mut core, &[0; 1024], &input).is_ok());
    let (small, small_queue) = binding(bytes - 4);
    let mut core = probe(&small);
    assert_eq!(
        prepare(&small, &small_queue, &mut core, &[0; 1024], &input),
        Err(TooManyFaces {
            bytes,
            limit: bytes - 4
        })
    );
}

fn resources() -> std::path::PathBuf {
    std::env::var_os("SUNRISE_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        })
}

/// Vanilla's own `clouds.vsh` and `clouds.fsh` through the pack path, one fancy cell above the
/// camera, which looks straight up at its bottom from its middle: the face `clouds.vsh` colours
/// with `faceColors[0]` (0.7) times `CloudColor` (0.8, 204 of 255), 0.56 or byte 142.8, so 143,
/// opaque (fog that far out is nothing). The cell's inward faces, wound the other way, don't
/// cover it: fancy clouds cull back faces.
#[test]
#[ignore = "daily"]
fn vanillas_clouds_shaders_draw_a_cell_above_the_camera() {
    let _gpu = crate::gfx::test_gpu::lock();
    let root = crate::resource_source::ResourceSource::directory(resources());
    let shader = |name: &str| root.read_to_string(&format!("assets/minecraft/shaders/core/{name}"));
    let (Ok(vertex), Ok(fragment)) = (shader("clouds.vsh"), shader("clouds.fsh")) else {
        eprintln!(
            "skipped: no prepared resources at {}",
            resources().display()
        );
        return;
    };
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        root.read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    };
    let program = crate::core_shader::translate_core_with(
        &vertex,
        &fragment,
        &CLASSIC_DEFINES,
        &include,
        &GLUE,
        &|b| b == INFO_BLOCK || b == FACES,
    )
    .expect("vanilla's clouds shaders translate");
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = CoreClouds::new(&device, wgpu::TextureFormat::Rgba8Unorm, classic(&program))
        .expect("pipelines");
    let mut filled = vec![false; 16];
    filled[0] = true;
    let texture = (4, 4, filled);
    let input = crate::clouds::Input {
        texture: &texture,
        uv: [0.0; 2],
        // The middle of cell 0, 0 (z is offset by 3.96), 10 below the layer.
        camera: glam::DVec3::new(
            6.0,
            f64::from(crate::clouds::CLOUD_HEIGHT) - 10.0,
            6.0 - 3.96,
        ),
        time: 0,
        partial_tick: 0.0,
        range: 1,
        fast: false,
        color: [0.8, 0.8, 0.8, 1.0],
        height: crate::clouds::CLOUD_HEIGHT,
    };
    let view = crate::gfx::View::screen(0.0, -90.0, 70.0, 1.0, 1024.0);
    let far = 1.0e9;
    let vanilla = crate::gfx::core_lines::uniforms(
        [0; 48],
        &view,
        glam::Mat4::IDENTITY,
        glam::Mat4::IDENTITY,
        1.0,
        [0.0; 4],
        [far; 6],
    );
    assert!(prepare(&device, &queue, &mut core, &vanilla, &input).is_ok());
    let pixels = draw(&device, &queue, &core, true);
    let middle = ((SIZE / 2) * SIZE + SIZE / 2) as usize * 4;
    assert_eq!(pixels[middle..middle + 4], [143, 143, 143, 255]);
}

/// A replacement shader that doesn't read `CloudFaces`: drawn all the same, one quad a face,
/// placed by `gl_VertexIndex` alone (here every quad covers the target in one colour, 51, 102,
/// 153 of 255).
#[test]
#[ignore = "daily"]
fn a_clouds_shader_that_reads_no_faces_still_draws_a_quad_a_face() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let vsh = "#version 330\nvoid main() {\n    int corner = gl_VertexIndex % 4;\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n}\n";
    let fsh = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vec4(0.2, 0.4, 0.6, 1.0);\n}\n";
    let program = crate::core_shader::translate_core_with(vsh, fsh, &[], &|_| None, &GLUE, &|b| {
        b == INFO_BLOCK || b == FACES
    })
    .expect("the shaders translate");
    assert!(program.other.is_empty(), "reads neither block");
    let mut core = CoreClouds::new(&device, wgpu::TextureFormat::Rgba8Unorm, classic(&program))
        .expect("pipelines");
    let mut filled = vec![false; 16];
    filled[3] = true;
    let texture = (4, 4, filled);
    assert_eq!(
        prepare(
            &device,
            &queue,
            &mut core,
            &[0; 1024],
            &flat_input(&texture)
        ),
        Ok(1)
    );
    for texel in draw(&device, &queue, &core, false).chunks(4) {
        assert_eq!(texel, [51, 102, 153, 255]);
    }
}

/// A device with Improved Transparency (`oit::features`), or none (said on stderr).
fn oit_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = crate::gfx::test_gpu::adapter()?;
    let features = super::super::oit::features(&adapter);
    if !features.contains(wgpu::Features::FLOAT32_BLENDABLE) {
        eprintln!(
            "skipped: no Improved Transparency on {:?}",
            adapter.get_info()
        );
        return None;
    }
    eprintln!("adapter {:?}", adapter.get_info());
    crate::gfx::test_gpu::optional_device_with(
        &adapter,
        &wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        },
    )
}

/// A vertex stage covering the target with each face's quad at device depth 0.5, wound so fancy
/// clouds keep it, handing `clouds.fsh` a fog distance of 0 and the colour 0.25, 0.5, 0.75, 1.
const OIT_VSH: &str = "#version 330\n\
    layout(location = 0) out float vertexDistance;\nlayout(location = 1) out vec4 vertexColor;\n\
    void main() {\n    int corner = gl_VertexIndex % 4;\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n    vertexDistance = 0.0;\n\
        vertexColor = vec4(0.25, 0.5, 0.75, 1.0);\n}\n";

/// `fragment` (with `OIT_VSH`) translated for each stage as the packs' loader does.
fn oit_programs(
    fragment: &str,
    include: &dyn Fn(&str) -> Option<String>,
) -> [crate::core_shader::CoreProgram; 3] {
    oit_programs_of(OIT_VSH, fragment, include)
}

/// `vertex` and `fragment` translated for each stage as the packs' loader does.
fn oit_programs_of(
    vertex: &str,
    fragment: &str,
    include: &dyn Fn(&str) -> Option<String>,
) -> [crate::core_shader::CoreProgram; 3] {
    let stage = |s: &OitStageSource| {
        crate::core_shader::translate_core_with(vertex, fragment, s.defines, include, &GLUE, &|b| {
            b == INFO_BLOCK || b == FACES || s.samplers.contains(&b)
        })
        .unwrap_or_else(|e| panic!("{}: {e}", s.pipeline))
    };
    [
        stage(&OIT_STAGES[0]),
        stage(&OIT_STAGES[1]),
        stage(&OIT_STAGES[2]),
    ]
}

/// An 8 by 8 texture of `format` usable as a target, a sampled input and a copy source.
fn oit_texture(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
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

/// The texel at the middle of `texture`, its bytes (the whole texture is copied: a depth one
/// can't be copied in part).
fn middle_texel(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
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
    data[at..at + size].to_vec()
}

/// `core`'s `stage` drawn as `Oit` draws the clouds' stage: into `targets` cleared to `clears`
/// over a depth cleared to 0 (the far plane, reversed Z), sampling `samples`. The middle texel of
/// each target and of the depth.
fn draw_stage(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    core: &CoreClouds,
    (stage, fancy): (super::super::oit::Stage, bool),
    targets: &[(&wgpu::Texture, wgpu::Color)],
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let depth = oit_texture(device, wgpu::TextureFormat::Depth32Float);
    let indices = crate::gfx::Renderer::quad_indices(device, core.quads.max(1));
    let views: Vec<wgpu::TextureView> = targets
        .iter()
        .map(|(t, _)| t.create_view(&Default::default()))
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let attachments: Vec<_> = views
            .iter()
            .zip(targets)
            .map(|(view, (_, clear))| {
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
        let depth_view = depth.create_view(&Default::default());
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
        core.draw_oit(&mut pass, stage, fancy, &indices);
    }
    queue.submit([encoder.finish()]);
    let texels = targets
        .iter()
        .map(|(t, _)| middle_texel(device, queue, t))
        .collect();
    (texels, middle_texel(device, queue, &depth))
}

fn f32s(bytes: &[u8]) -> Vec<f32> {
    bytemuck::pod_collect_to_vec(bytes)
}

fn f16s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks(2)
        .map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32())
        .collect()
}

/// The blocks with `ProjMat[2][2]` 0.5 and `ProjMat[3][2]` 3 (the rest 0): `deviceToLinearDepth`
/// of device depth 0.5 is 3 / (0.5 + 0.5), 3 blocks.
fn oit_blocks() -> [u8; 1024] {
    let mut blocks = [0u8; 1024];
    let projection = 256;
    blocks[projection + 10 * 4..][..4].copy_from_slice(&0.5f32.to_le_bytes());
    blocks[projection + 14 * 4..][..4].copy_from_slice(&3.0f32.to_le_bytes());
    blocks
}

const BOUNDS_CLEAR: wgpu::Color = wgpu::Color {
    r: -(f32::MAX as f64),
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// Vanilla's own `clouds.fsh` (its `oit.glsl` includes and all) as each of `OIT_CLOUDS`'s
/// stages, for an opaque fragment at device depth 0.5, 3 blocks away:
/// - depth bounds (`calculateDepthBounds`): -3, 3, 0.5 and, opaque, 0.5, the depth written by
///   the fancy pipeline only (`OIT_CLOUDS`' DEFAULT depth modifier);
/// - transmittance (`addTransmittance`), the bounds 2 to 4 blocks: depth 0.5 of the range, so
///   bin 3 and 0.4375 of the 7/8 scale, absorbance 4 (`toAbsorbance(0)`): coefficient 0 is
///   4 x 0.5625 = 2.25; the tree from index 11: coefficient 5 = (0.25 - 0.1875) x 2 x 4 = 0.5,
///   coefficient 2 = (0.5 - 0.4375) x sqrt 2 x 4 = 0.3536 (0.353515625 in half floats),
///   coefficient 1 = 0.4375 x 4 = 1.75;
/// - accumulate (`sampleColorForAccumulation`), distinct coefficient inputs and bounds 2 to 6:
///   transmittance rounds to 0.5 in RGBA16Float, scaling the premultiplied colour.
#[test]
#[ignore = "daily"]
fn vanillas_clouds_fragment_shader_draws_each_improved_transparency_stage() {
    use super::super::oit::Stage;
    let _gpu = crate::gfx::test_gpu::lock();
    let root = crate::resource_source::ResourceSource::directory(resources());
    let Ok(fragment) = root.read_to_string("assets/minecraft/shaders/core/clouds.fsh") else {
        eprintln!(
            "skipped: no prepared resources at {}",
            resources().display()
        );
        return;
    };
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        root.read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    };
    let programs = oit_programs(&fragment, &include);
    let Some((device, queue)) = oit_device() else {
        return;
    };
    let mut core = CoreClouds::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        (None, Some(&programs)),
    )
    .expect("pipelines");
    assert!(core.improved_transparency() && !core.classic());
    let mut cells = vec![false; 16];
    cells[3] = true;
    let texture = (4, 4, cells);
    assert_eq!(
        prepare(
            &device,
            &queue,
            &mut core,
            &oit_blocks(),
            &flat_input(&texture)
        ),
        Ok(1)
    );
    let (bounds_format, coefficient_format) = (
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureFormat::Rgba16Float,
    );
    // The transmittance stage samples bounds 2 to 4 blocks.
    let bounds_in = oit_texture(&device, bounds_format);
    let texels: Vec<f32> = [-2.0f32, 4.0, 0.0, 0.0].repeat((SIZE * SIZE) as usize);
    queue.write_texture(
        bounds_in.as_image_copy(),
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 16),
            rows_per_image: Some(SIZE),
        },
        bounds_in.size(),
    );
    let coefficients_in = [0, 1].map(|_| oit_texture(&device, coefficient_format));
    let views = [&bounds_in, &coefficients_in[0], &coefficients_in[1]]
        .map(|t| t.create_view(&Default::default()));
    core.bind_oit(
        &device,
        &super::super::oit::CloudSamples {
            generation: 1,
            views: [&views[0], &views[1], &views[2]],
        },
    );
    let gpu = (&device, &queue);
    let bounds = oit_texture(&device, bounds_format);
    for (fancy, depth) in [(true, 0.5f32), (false, 0.0)] {
        let (texels, written) = draw_stage(
            gpu,
            &core,
            (Stage::DepthBounds, fancy),
            &[(&bounds, BOUNDS_CLEAR)],
        );
        assert_eq!(f32s(&texels[0]), [-3.0, 3.0, 0.5, 0.5], "fancy {fancy}");
        assert_eq!(f32s(&written), [depth], "fancy {fancy}");
    }
    let coefficients = [0, 1].map(|_| oit_texture(&device, coefficient_format));
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::Transmittance, true),
        &[
            (&coefficients[0], wgpu::Color::TRANSPARENT),
            (&coefficients[1], wgpu::Color::TRANSPARENT),
        ],
    );
    assert_eq!(f16s(&texels[0]), [2.25, 1.75, 362.0 / 1024.0, 0.0]);
    assert_eq!(f16s(&texels[1]), [0.0, 0.5, 0.0, 0.0]);
    // Accumulate at depth 1/4 in the new bounds. With coefficients 2 and 3 zero,
    // the shader's wavelet interpolation gives absorbance c0 - c1 + c4 - 0.75.
    // Here it is just below ln(2), so the scaled colour rounds to the exact half
    // values below. Swapping the inputs instead gives absorbance 0.25.
    for (texture, coefficient) in coefficients_in
        .iter()
        .zip([[1.25, 233.0 / 4096.0, 0.0, 0.0], [0.25, 0.5, 0.0, 0.0]])
    {
        let texel = coefficient.map(|value| half::f16::from_f32(value).to_bits());
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&texel.repeat((SIZE * SIZE) as usize)),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 8),
                rows_per_image: Some(SIZE),
            },
            texture.size(),
        );
    }
    let bounds = filled(&device, &queue, [-2.0, 6.0, 0.0, 0.0]);
    let bounds_view = bounds.create_view(&Default::default());
    core.bind_oit(
        &device,
        &super::super::oit::CloudSamples {
            generation: 2,
            views: [&bounds_view, &views[1], &views[2]],
        },
    );
    let accumulated = oit_texture(&device, coefficient_format);
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::Accumulate, true),
        &[(&accumulated, wgpu::Color::TRANSPARENT)],
    );
    assert_eq!(f16s(&texels[0]), [0.125, 0.25, 0.375, 0.5]);
}

/// An 8 by 8 `Rgba32Float` texture of `texel` everywhere.
fn filled(device: &wgpu::Device, queue: &wgpu::Queue, texel: [f32; 4]) -> wgpu::Texture {
    let texture = oit_texture(device, wgpu::TextureFormat::Rgba32Float);
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
    texture
}

/// A fragment shader that ignores the stages' defines, one `out vec4 fragColor` as clouds.fsh
/// had before Improved Transparency: vanilla compiles it for each stage all the same, and each
/// stage draws its colour (0.25, 0.5, 0.75, 1) at location 0 with the stage's blend: MAX over
/// the bounds' clear, ADD over zero in the transmittance and accumulate targets. (A target it
/// doesn't write gets undefined values, as in vanilla, so only location 0 is compared.)
#[test]
#[ignore = "daily"]
fn a_clouds_shader_ignoring_the_stage_defines_draws_location_0_in_each_stage() {
    use super::super::oit::Stage;
    let _gpu = crate::gfx::test_gpu::lock();
    let fragment = "#version 330\nlayout(location = 0) in float vertexDistance;\n\
        layout(location = 1) in vec4 vertexColor;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vertexColor;\n}\n";
    let programs = oit_programs(fragment, &|_| None);
    let Some((device, queue)) = oit_device() else {
        return;
    };
    let mut core = CoreClouds::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        (None, Some(&programs)),
    )
    .expect("pipelines");
    assert!(core.improved_transparency());
    let mut filled_cells = vec![false; 16];
    filled_cells[3] = true;
    let texture = (4, 4, filled_cells);
    assert_eq!(
        prepare(
            &device,
            &queue,
            &mut core,
            &oit_blocks(),
            &flat_input(&texture)
        ),
        Ok(1)
    );
    let unused = filled(&device, &queue, [0.0; 4]);
    let view = unused.create_view(&Default::default());
    core.bind_oit(
        &device,
        &super::super::oit::CloudSamples {
            generation: 1,
            views: [&view, &view, &view],
        },
    );
    let gpu = (&device, &queue);
    let bounds = oit_texture(&device, wgpu::TextureFormat::Rgba32Float);
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::DepthBounds, false),
        &[(&bounds, BOUNDS_CLEAR)],
    );
    assert_eq!(f32s(&texels[0]), [0.25, 0.5, 0.75, 1.0]);
    let format = wgpu::TextureFormat::Rgba16Float;
    let coefficients = [0, 1].map(|_| oit_texture(&device, format));
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::Transmittance, false),
        &[
            (&coefficients[0], wgpu::Color::TRANSPARENT),
            (&coefficients[1], wgpu::Color::TRANSPARENT),
        ],
    );
    assert_eq!(f16s(&texels[0]), [0.25, 0.5, 0.75, 1.0]);
    let accumulated = oit_texture(&device, format);
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::Accumulate, false),
        &[(&accumulated, wgpu::Color::TRANSPARENT)],
    );
    assert_eq!(f16s(&texels[0]), [0.25, 0.5, 0.75, 1.0]);
}

/// A vertex stage covering the target with each face's quad, handing on the face's first byte
/// (`encodeFace`'s cell x, halved) as the colour's red; `clouds.fsh` ignoring the defines draws it.
const FACES_VSH: &str = "#version 330\nuniform isamplerBuffer CloudFaces;\n\
    layout(location = 0) out float vertexDistance;\nlayout(location = 1) out vec4 vertexColor;\n\
    void main() {\n    int corner = gl_VertexIndex % 4;\n\
        vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n\
        gl_Position = vec4(xy, 0.5, 1.0);\n    vertexDistance = 0.0;\n\
        vertexColor = vec4(float(texelFetch(CloudFaces, gl_VertexIndex / 4 * 3).r), 0.0, 0.0, 1.0);\n}\n";

/// The stages' groups follow what they bind:
/// - new Improved Transparency targets (generation 2): the transmittance stage reads the new
///   bounds, 3 to 7 blocks, where the fragment (3 blocks) is at depth 0 of the range: bin 0,
///   coefficient 0 = 4 x 1 = 4 and every wavelet 0 (generation 1's bounds, 2 to 4, gave 2.25,
///   1.75, 0.3535 and 0.5);
/// - a faces buffer made again for more faces than it held (the same targets): every cell of a
///   texture filled, range 14 (19 cells), more than the first buffer's 1024 faces; the bounds
///   stage's MAX over the faces' cell x bytes is 19 >> 1 = 9, read from the new buffer.
#[test]
#[ignore = "daily"]
fn the_improved_transparency_groups_follow_new_targets_and_a_grown_faces_buffer() {
    use super::super::oit::Stage;
    let _gpu = crate::gfx::test_gpu::lock();
    // `oit.glsl` declares the alpha-only stages' outputs (`fragColor` in the bounds stage).
    let fragment = "#version 330\n#include <minecraft:oit.glsl>\n\
        layout(location = 0) in float vertexDistance;\nlayout(location = 1) in vec4 vertexColor;\n\
        #ifndef OIT_ALPHA_ONLY\nlayout(location = 0) out vec4 fragColor;\n#endif\n\
        void main() {\n#ifdef OIT_TRANSMITTANCE\n    executeAlphaOnlyPhase(gl_FragCoord.z, 1.0);\n\
        #else\n    fragColor = vertexColor;\n#endif\n}\n";
    let root = crate::resource_source::ResourceSource::directory(resources());
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        root.read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    };
    if include("minecraft:shaders/include/oit.glsl").is_none() {
        eprintln!(
            "skipped: no prepared resources at {}",
            resources().display()
        );
        return;
    }
    let programs = oit_programs_of(FACES_VSH, fragment, &include);
    let Some((device, queue)) = oit_device() else {
        return;
    };
    let mut core = CoreClouds::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        (None, Some(&programs)),
    )
    .expect("pipelines");
    let mut one = vec![false; 16];
    one[3] = true;
    let one = (4, 4, one);
    assert_eq!(
        prepare(&device, &queue, &mut core, &oit_blocks(), &flat_input(&one)),
        Ok(1)
    );
    let zero = filled(&device, &queue, [0.0; 4]);
    let zero = zero.create_view(&Default::default());
    let gpu = (&device, &queue);
    let format = wgpu::TextureFormat::Rgba16Float;
    let transmittance = |core: &CoreClouds| {
        let coefficients = [0, 1].map(|_| oit_texture(&device, format));
        let (texels, _) = draw_stage(
            gpu,
            core,
            (Stage::Transmittance, false),
            &[
                (&coefficients[0], wgpu::Color::TRANSPARENT),
                (&coefficients[1], wgpu::Color::TRANSPARENT),
            ],
        );
        [f16s(&texels[0]), f16s(&texels[1])]
    };
    let first = filled(&device, &queue, [-2.0, 4.0, 0.0, 0.0]);
    let first = first.create_view(&Default::default());
    core.bind_oit(
        &device,
        &super::super::oit::CloudSamples {
            generation: 1,
            views: [&first, &zero, &zero],
        },
    );
    assert_eq!(
        transmittance(&core),
        [
            vec![2.25, 1.75, 362.0 / 1024.0, 0.0],
            vec![0.0, 0.5, 0.0, 0.0]
        ]
    );
    let second = filled(&device, &queue, [-3.0, 7.0, 0.0, 0.0]);
    let second = second.create_view(&Default::default());
    let samples = super::super::oit::CloudSamples {
        generation: 2,
        views: [&second, &zero, &zero],
    };
    core.bind_oit(&device, &samples);
    assert_eq!(
        transmittance(&core),
        [vec![4.0, 0.0, 0.0, 0.0], vec![0.0; 4]]
    );
    let all = (4, 4, vec![true; 16]);
    let many = crate::clouds::Input {
        range: 14,
        ..flat_input(&all)
    };
    let quads = prepare(&device, &queue, &mut core, &oit_blocks(), &many).unwrap();
    assert!(u64::from(quads) * 3 > FIRST_CAPACITY, "{quads} faces");
    core.bind_oit(&device, &samples);
    let bounds = oit_texture(&device, wgpu::TextureFormat::Rgba32Float);
    let (texels, _) = draw_stage(
        gpu,
        &core,
        (Stage::DepthBounds, false),
        &[(&bounds, BOUNDS_CLEAR)],
    );
    assert_eq!(f32s(&texels[0]), [9.0, 0.0, 0.0, 1.0]);
}

/// Who draws a frame's clouds, as `draw_frame` chains the picks: the pack takes the frame when
/// `pack_draws` (its faces, 4 quads here); else the native mesh is prepared (6 quads); then the
/// classic pass draws whichever has quads outside Improved Transparency, and its stages under it
/// (`oit_clouds`). `(classic pack, classic native, OIT stages)`.
fn picked(frame: CloudsFrame) -> (bool, bool, Option<&'static str>) {
    let pack_quads = frame.pack_draws().then_some(4);
    let native_quads = if native_mesh(frame.native_clouds, pack_quads) {
        6
    } else {
        0
    };
    let oit = frame.improved_transparency;
    let pack = pack_quads.is_some_and(|q| classic_pass_draws(q, frame.shader_pack, oit));
    let native = classic_pass_draws(native_quads, frame.shader_pack, oit);
    let stages = oit
        .then(|| oit_clouds::<str>(pack_quads.map(|_| "pack"), "native", native_quads))
        .flatten();
    (pack, native, stages)
}

/// The native mesh draws wherever no pack's shader does, in the pass of the frame's path: with no
/// pack (or once a reload removed it) under Improved Transparency its stages draw the native
/// mesh, outside it the classic pass does; with a pack its programs take the frame on each path.
#[test]
fn the_native_clouds_draw_in_the_frames_path_without_a_pack() {
    let none = CloudsFrame {
        pack_shader: false,
        pack_oit_shader: false,
        ..ALL
    };
    let improved = CloudsFrame {
        improved_transparency: true,
        ..ALL
    };
    let none_improved = CloudsFrame {
        improved_transparency: true,
        ..none
    };
    assert_eq!(picked(none_improved), (false, false, Some("native")));
    assert_eq!(picked(improved), (false, false, Some("pack")));
    // The pack removed: the frame is the one without a pack again.
    assert_eq!(picked(none_improved), (false, false, Some("native")));
    assert_eq!(picked(none), (false, true, None));
    assert_eq!(picked(ALL), (true, false, None));
    // A shader pack draws its own way: neither in the classic pass.
    let shader_pack = CloudsFrame {
        shader_pack: true,
        ..none
    };
    assert_eq!(picked(shader_pack), (false, false, None));
}

#[test]
#[ignore = "daily"]
fn linear_light_links_the_required_programs_before_suppressing_the_draw() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, _queue)) = oit_device() else {
        return;
    };
    let frame = super::super::FrameConfiguration {
        format: wgpu::TextureFormat::Rgba16Float,
        linear_light: true,
        ..super::super::EffectiveOutput::NONE.frame
    };
    let vertex = "layout(location = 0) flat out float tag;\nvoid main() { gl_Position = vec4(1.0); tag = 1.0; }";
    let fragment = "layout(location = 0) flat in float tag;\nlayout(location = 0) out vec4 pixel;\nvoid main() { pixel = vec4(float(tag)); }";
    let programs = |fragment: &str| crate::clouds::pack::Programs {
        classic: crate::core_shader::translate_core(
            vertex,
            fragment,
            &CLASSIC_DEFINES,
            &|_| None,
            &GLUE,
        )
        .unwrap(),
        oit: Box::new(oit_programs_of(vertex, fragment, &|_| None)),
    };
    assert!(
        clouds_for_frame(&device, frame, &programs(fragment))
            .unwrap()
            .is_none()
    );
    for (guard, pipeline) in [
        ("1", CLASSIC_PIPELINE),
        ("defined(OIT_TRANSMITTANCE)", OIT_STAGES[1].pipeline),
    ] {
        let varying = format!(
            "#if {guard}\nlayout(location = 0) flat in int tag;\n#else\nlayout(location = 0) flat in float tag;\n#endif"
        );
        let mismatch = fragment.replace("layout(location = 0) flat in float tag;", &varying);
        let error = clouds_for_frame(&device, frame, &programs(&mismatch))
            .err()
            .expect("device refused the varying");
        assert!(error.starts_with(pipeline), "{error}");
    }
}
