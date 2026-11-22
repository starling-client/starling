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
    text.prepare(&mut uploads, &mut encoder, &[0u8; 1024]);
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
        let part = if see_through {
            &text.see_through
        } else {
            &text.polygon_offset
        };
        let part = part.as_ref().expect("the pipeline");
        pass.set_pipeline(&part.pipeline);
        pass.set_bind_group(0, &part.group, &[]);
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
fn vanillas_text_shader_makes_both_world_text_pipelines() {
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
    let (device, queue) = device().expect("a GPU");
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
    .expect("the pack's world text pipelines");
    assert!(text.polygon_offset.is_some() && text.see_through.is_some());
}
