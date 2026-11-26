use super::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let (_, device, queue) = crate::gfx::test_gpu::optional_device()?;
    Some((device, queue))
}

/// A 1 by 1 texture of `rgba`, as the glyph atlas or the lightmap.
pub(super) fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: [u8; 4],
) -> wgpu::TextureView {
    wgpu::util::DeviceExt::create_texture_with_data(
        device,
        queue,
        &wgpu::TextureDescriptor {
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
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &rgba,
    )
    .create_view(&Default::default())
}

/// One pass of `text`'s world text (`see_through` false) or see-through text over the whole
/// target, its vertices' light `light`, after [`CoreText::prepare`] uploaded zeros for vanilla's
/// blocks and the expression uniforms' values: the target's middle pixel.
pub(super) fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    text: &CoreText,
    see_through: bool,
    light: u32,
) -> [u8; 4] {
    const SIZE: u32 = 64;
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
    let corner = |x: f32, y: f32| crate::mesher::Vertex {
        pos: [x, y, 0.5],
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light,
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
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut uploads = super::super::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    text.prepare(
        &mut uploads,
        &mut encoder,
        &[0u8; 1024],
        ([[0.0; 4]; 2], false),
    );
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
        let (pipeline, group) = if see_through {
            let part = text.see_through.as_ref().expect("the pipeline");
            (&part.pipeline, &part.group)
        } else {
            let world = text.world.as_ref().expect("the pipeline");
            (&world.polygon_offset, &world.group)
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..6, 0..1);
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
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
    let middle = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    pixels[middle..middle + 4].try_into().unwrap()
}

#[test]
#[ignore = "daily"]
fn a_pack_text_shader_reads_the_light_as_vanilla_does() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        eprintln!("no GPU: skipping");
        return;
    };
    let vsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
        layout(location = 0) flat out ivec2 light;\n\
        void main() { gl_Position = vec4(Position, 1.0); light = UV2; }\n";
    let fsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) flat in ivec2 light;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = vec4(vec2(light) / 255.0, 0.0, 1.0); }\n";
    let program =
        crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, &WORLD_GLUE).unwrap();
    let atlas = texture(&device, &queue, [0; 4]);
    let sampler = device.create_sampler(&Default::default());
    let text = CoreText::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        Some((&program, &[])),
        None,
        &atlas,
        (&atlas, &sampler),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("the pack's pipeline");
    let light = 0x30 | (0xC0 << 8);
    assert_eq!(
        draw(&device, &queue, &text, false, light),
        [0xC0, 0x30, 0, 0xFF],
        "UV2 is (block, sky)"
    );
}

#[test]
#[ignore = "daily"]
fn vanillas_text_shader_makes_every_world_text_pipeline() {
    let _gpu = crate::gfx::test_gpu::lock();
    let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        });
    let read = |path: &str| std::fs::read_to_string(root.join(path)).ok();
    let vertex = read("assets/minecraft/shaders/core/text.vsh").expect("vanilla's text.vsh");
    let fragment = read("assets/minecraft/shaders/core/text.fsh").expect("vanilla's text.fsh");
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        read(&format!("assets/{namespace}/{path}"))
    };
    let translate = |defines: &[&str], glue: &Glue| {
        crate::core_shader::translate_core(&vertex, &fragment, defines, &include, glue)
            .expect("vanilla's text shader translates")
    };
    let world = translate(&[], &WORLD_GLUE);
    let see_through = translate(&["IS_SEE_THROUGH"], &SEE_THROUGH_GLUE);
    // Improved Transparency's stages need its device features, where the adapter has them.
    let (device, queue) = (crate::gfx::test_gpu::oit_device())
        .or_else(device)
        .expect("a GPU");
    let (atlas, lightmap) = (
        texture(&device, &queue, [0; 4]),
        texture(&device, &queue, [0; 4]),
    );
    let sampler = device.create_sampler(&Default::default());
    let text = CoreText::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        Some((&world, &[])),
        Some((&see_through, &[])),
        &atlas,
        (&lightmap, &sampler),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("the pack's world text pipelines");
    let world = text.world.as_ref().expect("world text pipelines");
    assert!(world.mark_normal.is_some() && world.mark_polygon_offset.is_some());
    assert!(text.see_through.is_some());
    // `OIT_TEXT`'s three stages, where the device has Improved Transparency.
    if !crate::gfx::oit::supported(&device) {
        eprintln!("skipped: no Improved Transparency on this adapter");
        return;
    }
    // Each stage gives its own samplers (`OitRenderPassProvider`).
    let stages: [_; 3] = std::array::from_fn(|i| {
        let stage = &crate::gfx::core_clouds::OIT_STAGES[i];
        crate::core_shader::translate_core_with(
            &vertex,
            &fragment,
            stage.defines,
            &include,
            &WORLD_GLUE,
            &|name| stage.samplers.contains(&name),
        )
        .expect("vanilla's text shader translates for each stage")
    });
    let oit = super::oit::OitPart::new(
        &device,
        std::array::from_fn(|i| (&stages[i], &[][..])),
        &atlas,
        (&lightmap, &sampler),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(oit.is_some(), "the Improved Transparency text pipelines");
}

#[test]
fn pack_text_hdr_mark_wrapper_validates_naga_modules() {
    let vsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
        layout(location = 0) out vec4 vertexColor;\nlayout(location = 1) out vec2 texCoord0;\n\
        void main() { gl_Position = vec4(Position, 1.0); vertexColor = Color; texCoord0 = UV0; }\n";
    let fsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec4 vertexColor;\nlayout(location = 1) in vec2 texCoord0;\n\
        layout(location = 0) out vec4 fragColor;\n\
        void main() { if (texCoord0.x < 0.5) discard; fragColor = vertexColor; }\n";
    let program =
        crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, &WORLD_GLUE).unwrap();
    let (vs, fs) = wrap_program_for_hdr_mark(&program).expect("wrapped HDR mark modules");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&vs)
        .expect("wrapped vertex module validates");
    validator
        .validate(&fs)
        .expect("wrapped fragment module validates");
}

#[test]
#[ignore = "daily"]
fn pack_world_text_shades_both_normal_and_polygon_offset_depth_modes() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        eprintln!("no GPU: skipping");
        return;
    };
    // Swaps R and B so both depth modes visibly prove the pack shader ran rather than native text.
    let vsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
        layout(location = 0) out vec4 vertexColor;\n\
        void main() { gl_Position = vec4(Position, 1.0); vertexColor = Color; }\n";
    let fsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec4 vertexColor;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = vec4(vertexColor.b, vertexColor.g, vertexColor.r, vertexColor.a); }\n";
    let program =
        crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, &WORLD_GLUE).unwrap();
    let atlas = texture(&device, &queue, [255; 4]);
    let sampler = device.create_sampler(&Default::default());
    let text = CoreText::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        Some((&program, &[])),
        None,
        &atlas,
        (&atlas, &sampler),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("pack world text");
    let world = text.world.as_ref().expect("world part");

    const SIZE: u32 = 64;
    let mk_tex = |format, usage| {
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
    let color = mk_tex(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::COPY_SRC,
    );
    let depth = mk_tex(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::empty(),
    );
    let quad = |x0: f32, x1: f32, z: f32, rgba: [u8; 4]| {
        let c = u32::from_le_bytes(rgba);
        let v = |x, y| crate::mesher::Vertex {
            pos: [x, y, z],
            uv: [0.0; 2],
            layer: 0,
            color: c,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        };
        [
            v(x0, -1.0),
            v(x1, -1.0),
            v(x1, 1.0),
            v(x0, -1.0),
            v(x1, 1.0),
            v(x0, 1.0),
        ]
    };
    // Normal-depth quad covers full target (vertex red -> pack blue); coplanar polygon-offset
    // quad covers right half (vertex green -> pack green) and must win the coplanar depth tie.
    let mut verts = Vec::new();
    verts.extend_from_slice(&quad(-1.0, 1.0, 0.5, [255, 0, 0, 255]));
    verts.extend_from_slice(&quad(0.0, 1.0, 0.5, [0, 255, 0, 255]));
    let vbuf = wgpu::util::DeviceExt::create_buffer_init(
        &device,
        &wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        },
    );
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut uploads = super::super::upload::Uploads::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    text.prepare(
        &mut uploads,
        &mut encoder,
        &[0u8; 1024],
        ([[0.0; 4]; 2], false),
    );
    {
        let cv = color.create_view(&Default::default());
        let dv = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &cv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_vertex_buffer(0, vbuf.slice(..));
        pass.set_bind_group(0, &world.group, &[]);
        pass.set_pipeline(&world.normal);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&world.polygon_offset);
        pass.draw(6..12, 0..1);
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
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        pixels[i..i + 4].try_into().unwrap()
    };
    assert_eq!(
        at(SIZE / 4, SIZE / 2),
        [0, 0, 255, 255],
        "normal-depth text must be shaded by the pack shader"
    );
    assert_eq!(
        at(SIZE * 3 / 4, SIZE / 2),
        [0, 255, 0, 255],
        "coplanar polygon-offset text must also use the pack shader and win depth"
    );
}

#[test]
#[ignore = "daily"]
fn pack_world_text_hdr_mark_marks_only_drawn_fragments() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        eprintln!("no GPU: skipping");
        return;
    };
    // Pack shader discards left third (u < 0.33), outputs zero alpha on middle third
    // (0.33 <= u < 0.66), and draws bright white on right third (u >= 0.66).
    let vsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
        layout(location = 2) in vec2 UV0;\nlayout(location = 3) in ivec2 UV2;\n\
        layout(location = 0) out vec4 vertexColor;\nlayout(location = 1) out vec2 texCoord0;\n\
        void main() { gl_Position = vec4(Position, 1.0); vertexColor = Color; texCoord0 = UV0; }\n";
    let fsh = "#version 330\n#extension GL_ARB_separate_shader_objects : require\n\
        layout(location = 0) in vec4 vertexColor;\nlayout(location = 1) in vec2 texCoord0;\n\
        layout(location = 0) out vec4 fragColor;\n\
        void main() {\n\
            if (texCoord0.x < 0.33) discard;\n\
            if (texCoord0.x < 0.66) { fragColor = vec4(1.0, 1.0, 1.0, 0.0); return; }\n\
            fragColor = vertexColor;\n\
        }\n";
    let program =
        crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, &WORLD_GLUE).unwrap();
    let atlas = texture(&device, &queue, [255; 4]);
    let sampler = device.create_sampler(&Default::default());
    let text = CoreText::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        Some((&program, &[])),
        None,
        &atlas,
        (&atlas, &sampler),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .expect("pack world text");
    let world = text.world.as_ref().expect("world part");

    const SIZE: u32 = 64;
    let run = |mode_offset: bool, tier: u32| -> ([u8; 4], [u8; 4], [u8; 4]) {
        let mk_tex = |format, usage| {
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
        let color = mk_tex(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::COPY_SRC,
        );
        let depth = mk_tex(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::empty(),
        );
        let v = |x: f32, y: f32, u: f32| crate::mesher::Vertex {
            pos: [x, y, 0.5],
            uv: [u, 0.5],
            layer: tier << 16,
            color: u32::MAX,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        };
        let verts = [
            v(-1.0, -1.0, 0.0),
            v(1.0, -1.0, 1.0),
            v(1.0, 1.0, 1.0),
            v(-1.0, -1.0, 0.0),
            v(1.0, 1.0, 1.0),
            v(-1.0, 1.0, 0.0),
        ];
        let vbuf = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut uploads = super::super::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        let windows = [[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
        text.prepare(&mut uploads, &mut encoder, &[0u8; 1024], (windows, false));
        {
            let cv = color.create_view(&Default::default());
            let dv = depth.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &cv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &dv,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.25),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_bind_group(0, &world.group, &[]);
            if mode_offset {
                pass.set_pipeline(&world.polygon_offset);
            } else {
                pass.set_pipeline(&world.normal);
            }
            pass.draw(0..6, 0..1);
            let mark = if mode_offset {
                world.mark_polygon_offset.as_ref().expect("mark offset")
            } else {
                world.mark_normal.as_ref().expect("mark normal")
            };
            pass.set_pipeline(mark);
            pass.set_bind_group(0, &world.group, &[]);
            pass.set_bind_group(1, &text.hdr_mark_group, &[]);
            pass.draw(0..6, 0..1);
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
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = read.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: u32| -> [u8; 4] {
            let i = ((SIZE / 2 * SIZE + x) * 4) as usize;
            pixels[i..i + 4].try_into().unwrap()
        };
        (at(SIZE / 6), at(SIZE / 2), at(SIZE * 5 / 6))
    };

    for mode_offset in [false, true] {
        // Tier 2 (glowing): discarded and zero-alpha pixels must stay unmarked (alpha 255);
        // drawn white pixel (y = 1.0 -> lift = 1.0 -> mark = 0.175 * (3 - 1) = 0.35 -> ~89).
        let (discarded, zero_alpha, drawn) = run(mode_offset, 2);
        assert_eq!(
            discarded,
            [0, 0, 0, 255],
            "discarded pack fragment must stay unmarked (offset={mode_offset})"
        );
        assert_eq!(
            zero_alpha,
            [0, 0, 0, 255],
            "zero-alpha pack fragment must stay unmarked (offset={mode_offset})"
        );
        assert_eq!(
            drawn,
            [255, 255, 255, 89],
            "drawn glowing pack fragment must receive exact tier-2 HDR mark (0.35 -> 89) (offset={mode_offset})"
        );
        // Tier 0 (non-glowing): even drawn fragments remain unmarked (alpha 255).
        let (_, _, non_glow) = run(mode_offset, 0);
        assert_eq!(
            non_glow,
            [255, 255, 255, 255],
            "non-glowing text must stay unmarked (offset={mode_offset})"
        );
    }
}

/// The device refusing the world text's or the see-through text's pipeline fails the load, named
/// by vanilla's pipeline, with the other pipeline fine. The refusal is one vanilla's builder
/// makes too (`PipelineBuilder`: a descriptor missing from the pipeline's layout): `Sampler1`,
/// which neither binds.
#[test]
#[ignore = "daily"]
fn a_text_pipeline_the_device_refuses_fails_the_load() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let vsh = "#version 330\nlayout(location = 0) in vec3 Position;\n\
        void main() { gl_Position = vec4(Position, 1.0); }\n";
    let translate = |fsh: &str, glue: &Glue| {
        crate::core_shader::translate_core(vsh, fsh, &[], &|_| None, glue).unwrap()
    };
    let taken = "#version 330\nuniform sampler2D Sampler0;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = texture(Sampler0, vec2(0.5)); }\n";
    let refused = "#version 330\nuniform sampler2D Sampler1;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() { fragColor = texture(Sampler1, vec2(0.5)); }\n";
    let atlas = texture(&device, &queue, [0; 4]);
    let sampler = device.create_sampler(&Default::default());
    for (world, see_through, pipeline) in [
        (refused, taken, "minecraft:pipeline/text"),
        (taken, refused, "minecraft:pipeline/text_see_through"),
    ] {
        let world = translate(world, &WORLD_GLUE);
        let see_through = translate(see_through, &SEE_THROUGH_GLUE);
        let error = CoreText::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            Some((&world, &[])),
            Some((&see_through, &[])),
            &atlas,
            (&atlas, &sampler),
        )
        .err()
        .expect("the device refuses it");
        assert_eq!(error.split_once(": ").map(|(p, _)| p), Some(pipeline));
    }
}
