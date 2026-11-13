//! The late sky against the usual order, on the test machine's GPU: terrain.wgsl's, entity.wgsl's
//! and stars.wgsl's own shaders, over terrain with cutout holes, and the pixels.
use super::*;
use crate::gfx::{
    Globals, Renderer, entity_shader, reuse::Reuse, stars::Stars, sunrise_fan, terrain_source,
    test_gpu, upload::Uploads,
};
use crate::mesher::Vertex;
use bytemuck::Zeroable;
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SIZE: u32 = 64;
const SUNRISE_ALPHA: f32 = 0.8;

/// The sky's parts, in the usual order, each with its own switch: a part out of the picture
/// would pass the comparison without testing anything.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Sky,
    Sunrise,
    Celestial,
    Stars,
    DarkDisc,
}
const PARTS: [Part; 5] = [
    Part::Sky,
    Part::Sunrise,
    Part::Celestial,
    Part::Stars,
    Part::DarkDisc,
];

#[test]
fn the_late_sky_draws_the_usual_pixels() {
    let instance = test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    eprintln!("Late sky test adapter: {:?}", adapter.get_info());
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
    else {
        eprintln!("no GPU device: skipped");
        return;
    };
    let reuse = Reuse::default();
    reuse.begin();
    let kept = reuse.on(&device);
    let terrain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Late sky terrain"),
        // As the renderer builds it: the HDR prelude (`HDR_WRITE_LIGHT`) in front.
        source: wgpu::ShaderSource::Wgsl(
            (include_str!("../../hdr_light.wgsl").to_owned()
                + &terrain_source::terrain_shader(false))
                .into(),
        ),
    });
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bindings)],
        ..Default::default()
    });
    let sunrise_attributes = wgpu::vertex_attr_array![0=>Float32x4, 1=>Float32];
    let sunrise_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &sunrise_attributes,
    })];
    // As `Renderer::new`'s.
    let sunrise_blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    let celestial_blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    };
    let depth_state = |compare, write| wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    };
    let usual_sky = |vertex, fragment, buffers, blend| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &terrain_shader,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: Default::default(),
            depth_stencil: Some(depth_state(wgpu::CompareFunction::Always, false)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let usual_base = usual_sky("vs_sky", "fs_sky_base", &[], None);
    let usual_sunrise = usual_sky(
        "vs_sunrise",
        "fs_sunrise",
        &sunrise_buffers,
        Some(sunrise_blend),
    );
    let usual_dark = usual_sky(
        "vs_sky",
        "fs_dark_disc",
        &[],
        Some(wgpu::BlendState::ALPHA_BLENDING),
    );
    let entity = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Late sky entities"),
        source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
    });
    let entity_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
    let entity_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &entity_attributes,
    })];
    let celestial_targets = [Some(wgpu::ColorTargetState {
        format: FORMAT,
        blend: Some(celestial_blend),
        write_mask: wgpu::ColorWrites::ALL,
    })];
    let celestial = wgpu::RenderPipelineDescriptor {
        label: Some("Celestial bodies"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &entity,
            entry_point: Some("vs_entity"),
            compilation_options: Default::default(),
            buffers: &entity_buffers,
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(depth_state(wgpu::CompareFunction::Always, false)),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &entity,
            entry_point: Some("fs_sky_celestial"),
            compilation_options: Default::default(),
            targets: &celestial_targets,
        }),
        multiview_mask: None,
        cache: None,
    };
    let usual_celestial = device.create_render_pipeline(&celestial);
    let stars = Stars::new(&device, FORMAT);
    let late = LateSky::new(
        &TerrainModule {
            kept: &kept,
            shader: &terrain_shader,
            format: FORMAT,
            // As `Renderer::new`'s.
            constants: &[("SPRITE_ARRAYS", 1.0), ("SPRITE_SPLIT", 2048.0)],
        },
        &layout,
        (&sunrise_buffers, sunrise_blend),
        &celestial,
        &stars,
    )
    .expect("the test GPU takes these pipelines");
    // Solid terrain over the lower half, with the holes a cutout leaves.
    let occluder_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Late sky occluder"),
        source: wgpu::ShaderSource::Wgsl(
            "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let p = vec2(f32(i32(i & 1u) * 4 - 1), f32(i32(i >> 1u) * 4 - 1));
                return vec4(p, 0.5, 1.0);
            }
            @fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
                if at.y < 32.0 || (u32(at.x) / 4u + u32(at.y) / 4u) % 2u == 0u { discard; }
                return vec4(0.2, 0.7, 0.1, 0.9);
            }"
            .into(),
        ),
    });
    let occluder = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &occluder_shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(depth_state(wgpu::CompareFunction::GreaterEqual, true)),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &occluder_shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(FORMAT.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: size_of::<Globals>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let terrain_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bindings,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let texture = |pixels: &[u8], edge| {
        device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: edge,
                    height: edge,
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
    };
    // Translucent texels show the blend's order.
    let texels: Vec<u8> = (0..16 * 16)
        .flat_map(|i| {
            [
                (i * 17) as u8,
                (i * 31) as u8,
                (i * 7) as u8,
                (80 + i % 176) as u8,
            ]
        })
        .collect();
    let atlas = texture(&texels, 16).create_view(&Default::default());
    let lightmap = texture(&[255; 4], 1).create_view(&Default::default());
    let sampler = device.create_sampler(&Default::default());
    let entity_group = |pipeline: &wgpu::RenderPipeline| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lightmap),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        })
    };
    let usual_group = entity_group(&usual_celestial);
    let late_group = entity_group(&late.celestial);
    let indices = Renderer::quad_indices(&device, 1);
    let sun = Vec3::new(0.0, 0.2, -0.98).normalize();
    let mut uploads = Uploads::new(&device);
    let mut seen = [false; PARTS.len()];
    // Toward the sunrise fan (which lies around -X), above and below the horizon, and away from it.
    for look in [
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.4, 0.2),
        Vec3::new(-1.0, -1.2, -0.3),
        Vec3::new(0.3, 0.3, 1.0),
    ] {
        let look = look.normalize();
        let view = Mat4::look_to_rh(Vec3::ZERO, look, Vec3::Y);
        let tan = 35_f32.to_radians().tan();
        let projection = Mat4::perspective_infinite_reverse_rh(70_f32.to_radians(), 1.0, 0.05);
        let view_proj = projection * view;
        let mut globals = Globals::zeroed();
        globals.view_proj = view_proj.to_cols_array_2d();
        globals.inverse_view = view.inverse().to_cols_array_2d();
        globals.fog_color = [0.6, 0.7, 0.9, 1.0];
        globals.sky_color = [0.3, 0.5, 0.9, 1.0];
        globals.misc = [0.0, 0.0, 1.0, tan];
        globals.fog_ends = [128.0, 128.0, 1.0, 0.0];
        globals.sunrise = [1.0, 0.5, 0.2, SUNRISE_ALPHA];
        globals.sun = sun.extend(1.0).to_array();
        globals.hdr_light = [0.5, 0.5, 0.0, 0.0];
        queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&globals));
        let mut mesh = [[0.0f32; 5]; sunrise_fan::VERTICES];
        for (to, from) in mesh.iter_mut().zip(sunrise_fan::mesh(SUNRISE_ALPHA, sun)) {
            let clip = view_proj * glam::Vec4::new(from[0], from[1], from[2], 1.0);
            *to = [clip.x, clip.y, clip.z, clip.w, from[3]];
        }
        let sunrise_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&mesh),
            usage: wgpu::BufferUsages::VERTEX,
        });
        // A sun in front of the camera, off its centre.
        let right = look.cross(Vec3::Y).normalize();
        let up = right.cross(look);
        let centre = look * 100.0 + right * 20.0 + up * 25.0;
        let celestial_vertices =
            [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| Vertex {
                pos: (centre + right * x * 20.0 + up * y * 20.0).to_array(),
                uv: [x * 0.5 + 0.5, y * 0.5 + 0.5],
                layer: 2,
                color: 0xFFFF_FFFF,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0, 0.0],
            });
        let celestial_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&celestial_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut render = |late_order: bool, parts: &[Part]| {
            let size = wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            };
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(SIZE * SIZE * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            stars.prepare(&mut uploads, &mut encoder, view_proj, 1.0, &globals);
            uploads.finish(&encoder);
            {
                let view = target.create_view(&Default::default());
                let depth = depth.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.6,
                                g: 0.7,
                                b: 0.9,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                if late_order {
                    pass.set_pipeline(&occluder);
                    pass.draw(0..3, 0..1);
                }
                pass.set_bind_group(0, &terrain_group, &[]);
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                for part in parts {
                    match part {
                        Part::Sky => {
                            pass.set_pipeline(if late_order { &late.sky } else { &usual_base });
                            pass.draw(0..3, 0..1);
                        }
                        Part::Sunrise => {
                            pass.set_pipeline(if late_order {
                                &late.sunrise
                            } else {
                                &usual_sunrise
                            });
                            pass.set_vertex_buffer(0, sunrise_vertices.slice(..));
                            pass.draw(0..sunrise_fan::VERTICES as u32, 0..1);
                        }
                        Part::Celestial => {
                            if late_order {
                                pass.set_pipeline(&late.celestial);
                                pass.set_bind_group(0, &late_group, &[]);
                            } else {
                                pass.set_pipeline(&usual_celestial);
                                pass.set_bind_group(0, &usual_group, &[]);
                            }
                            pass.set_vertex_buffer(0, celestial_vertices.slice(..));
                            pass.draw_indexed(0..6, 0, 0..1);
                            pass.set_bind_group(0, &terrain_group, &[]);
                        }
                        Part::Stars => {
                            if late_order {
                                stars.draw_through(&mut pass, &late.stars);
                            } else {
                                stars.draw(&mut pass);
                            }
                            pass.set_bind_group(0, &terrain_group, &[]);
                        }
                        Part::DarkDisc => {
                            pass.set_pipeline(if late_order {
                                &late.dark_disc
                            } else {
                                &usual_dark
                            });
                            pass.draw(0..3, 0..1);
                        }
                    }
                }
                if !late_order {
                    pass.set_pipeline(&occluder);
                    pass.draw(0..3, 0..1);
                }
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: Some(SIZE),
                    },
                },
                size,
            );
            queue.submit([encoder.finish()]);
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            read.slice(..).get_mapped_range().unwrap().to_vec()
        };
        // A fan triangle reaching behind the camera is clipped at a different place, which can
        // round its weight one level off. Everything else is exact.
        let within = |a: &[u8], b: &[u8], levels: i16| {
            a.iter()
                .zip(b)
                .all(|(&a, &b)| (i16::from(a) - i16::from(b)).abs() <= levels)
        };
        for part in PARTS {
            let levels = i16::from(part == Part::Sunrise);
            assert!(
                within(&render(false, &[part]), &render(true, &[part]), levels),
                "part {} looking along {look}",
                part as u8
            );
        }
        let usual = render(false, &PARTS);
        assert!(
            within(&usual, &render(true, &PARTS), 1),
            "looking along {look}"
        );
        for (i, part) in PARTS.iter().enumerate() {
            let others: Vec<Part> = PARTS.iter().copied().filter(|p| p != part).collect();
            seen[i] |= render(false, &others) != usual;
        }
    }
    assert_eq!(seen, [true; PARTS.len()], "a part of the sky never showed");
}
