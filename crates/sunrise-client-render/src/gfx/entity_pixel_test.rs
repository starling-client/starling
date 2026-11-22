//! Offscreen entity pipeline readback for cross-crate rendering tests.
use super::*;
use bytemuck::Zeroable;

const SIZE: u32 = 64;

struct Frame {
    device: wgpu::Device,
    queue: wgpu::Queue,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    group: wgpu::BindGroup,
    bindings: wgpu::BindGroupLayout,
    atlas: wgpu::TextureView,
    atlas_texture: wgpu::Texture,
}

/// The software Vulkan adapter (lavapipe, llvmpipe) the pixel tests draw on.
fn software_adapter() -> std::result::Result<wgpu::Adapter, String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    }))
    .map_err(|error| format!("no software Vulkan adapter: {error}"))?;
    if adapter.get_info().device_type != wgpu::DeviceType::Cpu {
        return Err("no software Vulkan adapter".into());
    }
    Ok(adapter)
}

/// The lightmap most pixel tests draw with: every light level the same colour.
const FLAT_LIGHTMAP: [u8; 4] = [128, 192, 255, 255];

/// A lightmap whose grey rises with the block level (x) plus the sky level (y), so a quad's
/// brightness shows its light: 0 at no light, 64 at sky and block 2, 240 at full light.
pub fn graded_lightmap() -> Vec<u8> {
    (0..16u8)
        .flat_map(|sky| {
            (0..16u8).flat_map(move |block| {
                let grey = (block + sky) * 8;
                [grey, grey, grey, 255]
            })
        })
        .collect()
}

fn software_frame(
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    sky: f32,
    size: [u32; 2],
) -> std::result::Result<Frame, String> {
    software_frame_lit(
        atlas_size,
        atlas,
        view_proj,
        sky,
        size,
        &FLAT_LIGHTMAP.repeat(16 * 16),
    )
}

/// [`software_frame`] with a 16 by 16 RGBA `lightmap`, block level across, sky level down.
fn software_frame_lit(
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    sky: f32,
    size: [u32; 2],
    lightmap_pixels: &[u8],
) -> std::result::Result<Frame, String> {
    let adapter = software_adapter()?;
    let mut required_limits = wgpu::Limits::default();
    required_limits.max_texture_dimension_2d = required_limits
        .max_texture_dimension_2d
        .max(atlas_size[0])
        .max(atlas_size[1]);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits,
        ..Default::default()
    }))
    .map_err(|error| format!("software Vulkan device request failed: {error}"))?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = |width, height, format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
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
        size[0],
        size[1],
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        size[0],
        size[1],
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let atlas_texture = texture(
        atlas_size[0],
        atlas_size[1],
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        atlas_texture.as_image_copy(),
        atlas,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas_size[0] * 4),
            rows_per_image: None,
        },
        atlas_texture.size(),
    );
    let lightmap = texture(
        16,
        16,
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    queue.write_texture(
        lightmap.as_image_copy(),
        lightmap_pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: None,
        },
        lightmap.size(),
    );
    let sampler = device.create_sampler(&Default::default());
    let mut globals = Globals::zeroed();
    globals.view_proj = view_proj.to_cols_array_2d();
    globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
    // Sky factor for `hdr_scene`. Zero keeps the older pixel tests on colour only.
    globals.hdr_light = [sky, sky, 1.0, 0.0];
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entity_binding_entries(),
    });
    let atlas_view = atlas_texture.create_view(&Default::default());
    let light_view = lightmap.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bindings,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&light_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
        ],
    });
    Ok(Frame {
        device,
        queue,
        color,
        depth,
        group,
        bindings,
        atlas: atlas_view,
        atlas_texture,
    })
}

fn readback(frame: &Frame, encoder: wgpu::CommandEncoder) -> std::result::Result<Vec<u8>, String> {
    let (width, height) = (frame.color.width(), frame.color.height());
    let row_bytes = width * 4;
    let pitch = row_bytes.div_ceil(256) * 256;
    let readback = frame.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(pitch) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = encoder;
    encoder.copy_texture_to_buffer(
        frame.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: None,
            },
        },
        frame.color.size(),
    );
    frame.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    Ok(mapped
        .chunks_exact(pitch as usize)
        .flat_map(|row| row[..row_bytes as usize].iter().copied())
        .collect())
}

/// Renders CPU entity quads. A missing software Vulkan adapter fails the test.
pub fn render(
    vertices: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    hdr: bool,
) -> std::result::Result<Vec<u8>, String> {
    render_layers(vertices, &[], atlas_size, atlas, view_proj, hdr)
}

/// [`render`], then `see_through` quads as the game draws them (`RenderPipelines.TEXT_SEE_THROUGH`:
/// no depth test or writes, culled, blended), over the entity quads.
pub fn render_see_through(
    vertices: &[Vertex],
    see_through: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
) -> std::result::Result<Vec<u8>, String> {
    render_layers(vertices, see_through, atlas_size, atlas, view_proj, false)
}

/// [`render_see_through`] lit by `lightmap` (16 by 16 RGBA, block level across, sky level down;
/// [`graded_lightmap`] shows each quad's light), so a test can tell light levels apart.
pub fn render_see_through_lit(
    vertices: &[Vertex],
    see_through: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    lightmap: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    render_layers_lit(
        vertices,
        see_through,
        atlas_size,
        atlas,
        view_proj,
        false,
        lightmap,
    )
}

fn render_layers(
    vertices: &[Vertex],
    see_through: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    hdr: bool,
) -> std::result::Result<Vec<u8>, String> {
    let flat = FLAT_LIGHTMAP.repeat(16 * 16);
    render_layers_lit(
        vertices,
        see_through,
        atlas_size,
        atlas,
        view_proj,
        hdr,
        &flat,
    )
}

fn render_layers_lit(
    vertices: &[Vertex],
    see_through: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    hdr: bool,
    lightmap: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame_lit(atlas_size, atlas, view_proj, 0.0, [SIZE, SIZE], lightmap)?;
    let shader = frame
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
    let layout = frame
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&frame.bindings)],
            immediate_size: 0,
        });
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    let reuse = reuse::Reuse::default();
    let kept = reuse.on(&frame.device);
    let make_with = |fragment, depth_write, bias, blend, write_mask, compare, cull_mode| {
        entity_pipeline_factory(
            &kept,
            &layout,
            &layout,
            &shader,
            &buffers,
            wgpu::TextureFormat::Rgba8Unorm,
            false,
        )(EntityPipelineOptions {
            label: "Entities",
            cull_mode,
            depth_write,
            compare,
            bias,
            blend,
            clouds: false,
            vertex: "vs_entity",
            fragment,
            write_mask,
        })
    };
    let make = |fragment, depth_write, bias, blend, write_mask| {
        let compare = wgpu::CompareFunction::GreaterEqual;
        make_with(
            fragment,
            depth_write,
            bias,
            blend,
            write_mask,
            compare,
            None,
        )
    };
    let pipeline = make(
        "fs_entity",
        true,
        wgpu::DepthBiasState {
            constant: 128,
            slope_scale: 0.0,
            clamp: 0.0,
        },
        wgpu::BlendState::ALPHA_BLENDING,
        wgpu::ColorWrites::ALL,
    );
    let hdr_pipeline = hdr.then(|| {
        make(
            "fs_hdr_mark",
            false,
            wgpu::DepthBiasState::default(),
            wgpu::BlendState::REPLACE,
            wgpu::ColorWrites::ALPHA,
        )
    });
    let see_through_pipeline = make_with(
        "fs_entity_unmarked",
        false,
        wgpu::DepthBiasState::default(),
        wgpu::BlendState::ALPHA_BLENDING,
        wgpu::ColorWrites::ALL,
        wgpu::CompareFunction::Always,
        Some(wgpu::Face::Back),
    );
    let entity_quads = vertices.len() as u32 / 4;
    let vertices = [vertices, see_through].concat();
    let vertex = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let indices: Vec<u32> = (0..vertices.len() as u32 / 4)
        .flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3])
        .collect();
    let index = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    {
        let color_view = frame.color.create_view(&Default::default());
        let depth_view = frame.depth.create_view(&Default::default());
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
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        // An empty scene still clears and reads back the target, but wgpu refuses
        // a slice of a zero-sized buffer. There is nothing to bind or draw here.
        if !indices.is_empty() {
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &frame.group, &[]);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
            if let Some(hdr_pipeline) = &hdr_pipeline {
                pass.set_pipeline(hdr_pipeline);
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
            }
            if indices.len() as u32 > entity_quads * 6 {
                pass.set_pipeline(&see_through_pipeline);
                pass.draw_indexed(entity_quads * 6..indices.len() as u32, 0, 0..1);
            }
        }
    }
    readback(&frame, encoder)
}

#[cfg(test)]
mod see_through_tests {
    use super::*;

    /// A quad in clip space from `x0` to `x1`, half the height, at reversed depth `z`.
    fn quad(x0: f32, x1: f32, z: f32, color: u32) -> [Vertex; 4] {
        let corner = |x, y| Vertex {
            pos: [x, y, z],
            uv: [0.5, 0.5],
            layer: 0,
            color,
            light: 0xF0,
            normal: [0, 0, 127, 127],
            glint_uv: [0.0, 0.0],
        };
        [
            corner(x0, -0.5),
            corner(x1, -0.5),
            corner(x1, 0.5),
            corner(x0, 0.5),
        ]
    }

    fn centre(pixels: &[u8]) -> [u8; 4] {
        pixels.as_chunks::<4>().0[(SIZE / 2 * SIZE + SIZE / 2) as usize]
    }

    /// A wall hides an entity quad behind it; the same quad drawn see-through shows over the
    /// wall from the front, and not from behind (culled, as `TEXT_SEE_THROUGH` is).
    #[test]
    #[ignore = "daily"]
    fn see_through_quads_draw_over_the_wall_that_hides_entity_quads() {
        let atlas = [255u8; 4].repeat(16);
        let draw = |world: &[Vertex], see_through: &[Vertex]| {
            render_see_through(world, see_through, [4, 4], &atlas, Mat4::IDENTITY)
                .expect("software Vulkan see-through pixels")
        };
        // Reversed depth: the wall (0.5) is nearer than the red quad (0.25).
        let wall = quad(-1.0, 1.0, 0.5, 0xFF80_8080);
        let red = quad(-0.5, 0.5, 0.25, 0xFF00_00FF);
        let only_wall = draw(&wall, &[]);
        assert_eq!(
            draw(&[wall, red].concat(), &[]),
            only_wall,
            "hidden by the wall"
        );
        let shown = centre(&draw(&wall, &red));
        assert!(
            shown[0] > 0 && shown[1] == 0 && shown[2] == 0,
            "red over the wall: {shown:?}"
        );
        let mut back = red;
        back.reverse();
        assert_eq!(draw(&wall, &back), only_wall, "its back is culled");
    }

    /// Under [`graded_lightmap`], a lit quad's brightness follows its light and the unlit
    /// see-through layer (`TEXT_SEE_THROUGH`, layer 2) ignores it.
    #[test]
    #[ignore = "daily"]
    fn graded_light_shows_on_lit_quads_and_not_on_unlit_see_through_ones() {
        let atlas = [255u8; 4].repeat(16);
        let lightmap = graded_lightmap();
        let lit = |light: u8, layer: u32, see_through: bool| {
            let mut quad = quad(-0.5, 0.5, 0.25, 0xFFFF_FFFF);
            for v in &mut quad {
                v.light = crate::block_entity_render::vertex_light(light);
                v.layer = layer;
                v.normal = [0; 4];
            }
            let (world, over): (&[Vertex], &[Vertex]) = if see_through {
                (&[], &quad)
            } else {
                (&quad, &[])
            };
            centre(
                &render_see_through_lit(world, over, [4, 4], &atlas, Mat4::IDENTITY, &lightmap)
                    .expect("software Vulkan lit pixels"),
            )
        };
        let [dark, two, full] = [0x00, 0x22, 0xFF].map(|light| lit(light, 0, false)[0]);
        assert!(dark < two && two < full, "{dark} < {two} < {full}");
        assert_eq!(
            lit(0x00, 2, true),
            lit(0xFF, 2, true),
            "unlit over the wall"
        );
        assert_eq!(lit(0x00, 2, true)[0], 255);
    }
}

/// Inputs for one offscreen prepared instance. Fields stay named so the draw is not a long argument list.
pub struct PreparedInstanceDraw<'a> {
    pub vertices: &'a [super::prepared_model::PreparedModelVertex],
    pub ranges: Vec<Option<(u32, u32)>>,
    pub uploaded: u64,
    pub requested: u64,
    pub slot: u32,
    pub instances: &'a [super::prepared_model::PreparedModelInstance],
    /// When set, each copy is its own draw. Otherwise they are one run.
    pub per_copy: bool,
    /// Colour pass, then the HDR mark. Off draws the colour pass only.
    pub hdr: bool,
    pub atlas_size: [u32; 2],
    pub atlas: &'a [u8],
    pub view_proj: Mat4,
}

/// One prepared instance plus its HDR mark. The geometry is dropped before submit.
pub fn render_instance(draw: PreparedInstanceDraw<'_>) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let PreparedInstanceDraw {
        vertices,
        ranges,
        uploaded,
        requested,
        slot,
        instances,
        per_copy,
        hdr,
        atlas_size,
        atlas,
        view_proj,
    } = draw;
    let frame = software_frame(atlas_size, atlas, view_proj, 0.0, [SIZE, SIZE])?;
    let models = super::prepared_model::PreparedModels::create(
        &frame.device,
        &reuse::Reuse::default(),
        &frame.bindings,
        wgpu::TextureFormat::Rgba8Unorm,
        uploaded,
        vertices,
        ranges,
    )
    .map_err(|error| error.to_string())?;
    let range = models
        .indices(requested, slot)
        .map_err(|error| error.to_string())?;
    let instance = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    let passes = if hdr { 2 } else { 1 };
    for (mark, clear) in [(false, true), (true, false)].into_iter().take(passes) {
        let color_view = frame.color.create_view(&Default::default());
        let depth_view = frame.depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(0.0)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &frame.group, &[]);
        let runs = if per_copy {
            (0..instances.len() as u32)
                .map(|first| super::prepared_model::InstanceRun { first, count: 1 })
                .collect::<Vec<_>>()
        } else {
            vec![super::prepared_model::InstanceRun {
                first: 0,
                count: instances.len() as u32,
            }]
        };
        for run in runs {
            models.draw_run(&mut pass, mark, &instance, range, run);
        }
    }
    drop(models);
    readback(&frame, encoder)
}

#[cfg(test)]
fn prepared_quad(
    frame: &Frame,
    light: u32,
) -> std::result::Result<
    (
        super::prepared_model::PreparedModels,
        wgpu::Buffer,
        (u32, u32),
    ),
    String,
> {
    let vertex = |x, y| super::prepared_model::PreparedModelVertex {
        pos: [x, y, 0.5, 1.0],
        uv: [0.5, 0.5, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0, 0.0],
        flags: [1, 0, 0, 0],
    };
    let vertices = [
        vertex(-0.2, -0.2),
        vertex(0.2, -0.2),
        vertex(0.2, 0.2),
        vertex(-0.2, 0.2),
    ];
    let mut normal = [[0.0; 4]; 3];
    normal[0][0] = 1.0;
    normal[1][1] = 1.0;
    normal[2][2] = 1.0;
    let instance = super::prepared_model::PreparedModelInstance {
        model: Mat4::IDENTITY.to_cols_array_2d(),
        normal,
        tint: [1.0, 48.0 / 255.0, 64.0 / 255.0, 1.0],
        light,
        _pad: [0; 3],
    };
    let models = super::prepared_model::PreparedModels::create(
        &frame.device,
        &reuse::Reuse::default(),
        &frame.bindings,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        &vertices,
        vec![Some((0, 6))],
    )
    .map_err(|error| error.to_string())?;
    let range = models.indices(1, 0).map_err(|error| error.to_string())?;
    let buffer = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&instance),
            usage: wgpu::BufferUsages::VERTEX,
        });
    Ok((models, buffer, range))
}

#[cfg(test)]
fn cpu_quad(
    frame: &Frame,
    light: u32,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::Buffer,
    wgpu::Buffer,
) {
    let reuse = reuse::Reuse::default();
    let kept = reuse.on(&frame.device);
    let shader = frame
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
    let layout = frame
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&frame.bindings)],
            immediate_size: 0,
        });
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    let make = |fragment, depth_write, bias, write_mask| {
        entity_pipeline_factory(
            &kept,
            &layout,
            &layout,
            &shader,
            &buffers,
            wgpu::TextureFormat::Rgba8Unorm,
            false,
        )(EntityPipelineOptions {
            label: "Entities",
            cull_mode: None,
            depth_write,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias,
            blend: if write_mask == wgpu::ColorWrites::ALPHA {
                wgpu::BlendState::REPLACE
            } else {
                wgpu::BlendState::ALPHA_BLENDING
            },
            clouds: false,
            vertex: "vs_entity",
            fragment,
            write_mask,
        })
    };
    let pipeline = make(
        "fs_entity",
        true,
        wgpu::DepthBiasState {
            constant: 128,
            slope_scale: 0.0,
            clamp: 0.0,
        },
        wgpu::ColorWrites::ALL,
    );
    let mark = make(
        "fs_hdr_mark",
        false,
        wgpu::DepthBiasState::default(),
        wgpu::ColorWrites::ALPHA,
    );
    let corner = |x, y| Vertex {
        pos: [x, y, 0.5],
        uv: [0.5, 0.5],
        layer: 0,
        color: 0xFF40_30FF,
        light,
        normal: [0, 0, 127, 127],
        glint_uv: [0.0, 0.0],
    };
    let vertices = [
        corner(-0.2, -0.2),
        corner(0.2, -0.2),
        corner(0.2, 0.2),
        corner(-0.2, 0.2),
    ];
    let vertex = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let index = frame
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });
    (pipeline, mark, vertex, index)
}

/// Centre scRGB red after the real HDR composite. The CPU mark uses bias 0, matching
/// `hdr_mark_pipeline`. The instanced mark uses the prepared pipeline's own bias.
#[cfg(test)]
fn scrgb_center(instanced: bool, light: u32) -> std::result::Result<f32, String> {
    let atlas = [255u8, 255, 255, 255].repeat(16);
    let mut frame = software_frame([4, 4], &atlas, Mat4::IDENTITY, 0.1, [SIZE, SIZE])?;
    let mut hdr = super::hdr::Hdr::new(
        &frame.device,
        &frame.queue,
        wgpu::TextureFormat::Rgba8Unorm,
        super::hdr::Output::ScRgb,
        false,
    );
    hdr.paper_white = 203.0;
    hdr.peak = 1000.0;
    hdr.lighting = true;
    hdr.look = true;
    frame.color = hdr.frame(&frame.device, [SIZE, SIZE]);
    let gpu = instanced
        .then(|| prepared_quad(&frame, light))
        .transpose()?;
    let cpu = (!instanced).then(|| cpu_quad(&frame, light));
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    {
        let color_view = frame.color.create_view(&Default::default());
        let depth_view = frame.depth.create_view(&Default::default());
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
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &frame.group, &[]);
        if let Some((models, buffer, range)) = &gpu {
            let run = super::prepared_model::InstanceRun { first: 0, count: 1 };
            models.draw_run(&mut pass, false, buffer, *range, run);
            models.draw_run(&mut pass, true, buffer, *range, run);
        }
        if let Some((pipeline, mark, vertex, index)) = &cpu {
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
            pass.set_pipeline(mark);
            pass.draw_indexed(0..6, 0, 0..1);
        }
    }
    hdr.keep_world(&mut encoder, None);
    let target = frame.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: frame.color.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    hdr.present(&frame.queue, &mut encoder, &target, true, None);
    let row = (SIZE * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = frame.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    frame.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    let bytes = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    let pixels = row / 8;
    let at = ((32 * pixels + 32) * 8) as usize;
    let bits = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    Ok(half::f16::from_bits(bits).to_f32())
}

#[cfg(test)]
#[rustfmt::skip]
mod instance_run {
    use super::super::prepared_model::{PreparedModelInstance, PreparedModelVertex};
    use super::{PreparedInstanceDraw, SIZE};
    use glam::{Mat4, Vec3};

    fn corner(x: f32, y: f32) -> PreparedModelVertex {
        PreparedModelVertex { pos: [x, y, 0.5, 1.0], uv: [0.5, 0.5, 0.0, 0.0], normal: [0.0, 0.0, 1.0, 0.0], flags: [1, 0, 0, 0] }
    }

    fn instance_at(x: f32, tint: [f32; 4]) -> PreparedModelInstance {
        let mut normal = [[0.0; 4]; 3];
        normal[0][0] = 1.0;
        normal[1][1] = 1.0;
        normal[2][2] = 1.0;
        PreparedModelInstance { model: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)).to_cols_array_2d(), normal, tint, light: 0xff00, _pad: [0; 3] }
    }

    fn render(per_copy: bool, hdr: bool) -> Result<Vec<u8>, String> {
        let instances = [instance_at(-0.45, [1.0, 0.0, 0.0, 1.0]), instance_at(0.45, [0.0, 0.0, 1.0, 1.0])];
        let vertices = [corner(-0.2, -0.2), corner(0.2, -0.2), corner(0.2, 0.2), corner(-0.2, 0.2)];
        super::render_instance(PreparedInstanceDraw {
            vertices: &vertices, ranges: vec![Some((0, 6))], uploaded: 1, requested: 1, slot: 0,
            instances: &instances, per_copy, hdr, atlas_size: [4, 4],
            atlas: &[255, 255, 255, 255].repeat(16), view_proj: Mat4::IDENTITY,
        })
    }

    #[test]
    #[ignore = "daily"]
    fn an_instanced_run_matches_per_copy_pixels_with_hdr() {
        let per_copy = render(true, true).expect("per-copy software Vulkan draw");
        let instanced = render(false, true).expect("instanced software Vulkan draw");
        assert_eq!(per_copy, instanced);
        assert_eq!(per_copy.len(), (SIZE * SIZE * 4) as usize);
        let covered = per_copy.as_chunks::<4>().0.iter().filter(|p| p[0] != 0 || p[2] != 0).count();
        assert!(covered > 8, "the run covered {covered} pixels");
        let plain = render(false, false).expect("color-only software Vulkan draw");
        let alpha = |pixels: &[u8]| pixels.iter().skip(3).step_by(4).copied().collect::<Vec<_>>();
        assert_eq!(alpha(&per_copy), alpha(&plain), "a bias-0 mark must not change alpha");
    }
}

/// What our probe pack's `gbuffers_entities`, `gbuffers_block` and `gbuffers_textured` each write
/// ([`render_pack`]), in that order.
pub const PACK_COLOURS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

/// `vertices`, in the runs `spans` gives them (as the game marks them), drawn on software Vulkan
/// by a probe pack of our own: each of its gbuffers programs writes one flat colour of
/// [`PACK_COLOURS`] and puts a vertex at half its camera-relative x and y. Runs the pack leaves
/// to Sunrise stay black.
pub fn render_pack(
    vertices: &[Vertex],
    spans: &[pack_draw::EntitySpan],
) -> std::result::Result<Vec<[u8; 4]>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    use std::io::Write;
    let colour = |[r, g, b, _]: [u8; 4]| format!("vec4({r}.0, {g}.0, {b}.0, 255.0) / 255.0");
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| -> std::io::Result<()> {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(source.as_bytes())
    };
    let programs = ["gbuffers_entities", "gbuffers_block", "gbuffers_textured"];
    for (program, rgba) in programs.into_iter().zip(PACK_COLOURS) {
        add(
            &format!("{program}.vsh"),
            "#version 120\nvoid main() { gl_Position = vec4(gl_Vertex.xy * 0.5, 0.5, 1.0); }\n",
        )
        .and_then(|()| {
            add(
                &format!("{program}.fsh"),
                &format!(
                    "#version 120\nvoid main() {{ gl_FragData[0] = {}; }}\n",
                    colour(rgba)
                ),
            )
        })
        .map_err(|error| error.to_string())?;
    }
    add(
        "final.vsh",
        "#version 120\nvarying vec2 uv;\n\
         void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }\n",
    )
    .and_then(|()| {
        add(
            "final.fsh",
            "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = texture2D(colortex0, uv); }\n",
        )
    })
    .map_err(|error| error.to_string())?;
    let bytes = zip.finish().map_err(|error| error.to_string())?;
    let pack = sunrise_shaders::Pack::from_zip_bytes("original-owner-probe", bytes.into_inner())
        .map_err(|error| format!("{error:?}"))?;
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let inputs = sunrise_shaders::GameInputs {
        entities: Some(pack_entities::inputs()),
        entity_geometry: Some(pack_entities::geometry_inputs()),
        ..Default::default()
    };
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    let adapter = software_adapter()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = |size: u32, format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original owner probe"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let mut post = pack_post::PackPost::new(
        &device,
        &queue,
        &adapter,
        post,
        format,
        pack_post::World {
            atlas: None,
            material_maps: None,
            lightmap: texture(1, format),
            terrain: None,
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: [SIZE; 2],
            clouds: None,
            weather: false,
            hand: false,
        },
        &Default::default(),
    )
    .map_err(|refused| refused.reason)?;
    let mut rest = Vec::new();
    post.prepare_entities(
        &device,
        &queue,
        vertices,
        spans,
        [0.0; 4],
        [0.0; 4],
        &mut rest,
        None,
        (&[], &[]),
    );
    let (world, depth, screen) = (
        texture(SIZE, format),
        texture(SIZE, wgpu::TextureFormat::Depth32Float),
        texture(SIZE, format),
    );
    let hud = HudBatch::scratch([SIZE; 2]);
    let mut encoder = device.create_command_encoder(&Default::default());
    post.run(
        &device,
        &queue,
        &mut encoder,
        &world,
        &depth,
        &screen,
        &probe_frame(&hud),
        &Default::default(),
        [SIZE; 2],
        [SIZE; 2],
        &[],
        &mut |_, _, _, _| {},
        &std::sync::atomic::AtomicBool::new(false),
        super::in_flight::poll_live,
    )
    .unwrap();
    let (color, read) = (screen.texture(), SIZE * 4);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(read * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(read),
                rows_per_image: None,
            },
        },
        color.size(),
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    Ok(mapped.as_chunks::<4>().0.to_vec())
}

/// A world frame with nothing in it but the pack's entities (`render_pack`).
fn probe_frame(hud: &HudBatch) -> super::Frame<'_> {
    super::Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_degrees: 90.0,
        bob: Mat4::IDENTITY,
        spin: Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        explicit_particles: None,
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        celestial: &[],
        celestial_bodies: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
        menu_blur_cache: false,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}

#[cfg(test)]
mod scrgb {
    #[test]
    #[ignore = "daily"]
    fn an_instanced_lit_texel_matches_cpu_scrgb() {
        let _gpu = crate::gfx::test_gpu::lock();
        let dark =
            |instanced| super::scrgb_center(instanced, 0).expect("software Vulkan scRGB draw");
        let (cpu_dark, gpu_dark) = (dark(false), dark(true));
        assert!(
            cpu_dark > 0.0 && gpu_dark > 0.0,
            "the unlit quad did not cover the centre: cpu {cpu_dark} gpu {gpu_dark}"
        );
        let dark_ratio = gpu_dark / cpu_dark;
        assert!(
            (dark_ratio - 1.0).abs() < 0.02,
            "light 0 should match, ratio {dark_ratio} (gpu {gpu_dark} cpu {cpu_dark})"
        );
        let cpu = super::scrgb_center(false, 0x00F0).expect("cpu scRGB");
        let gpu = super::scrgb_center(true, 0x00F0).expect("instanced scRGB");
        assert!(cpu > 0.0, "the lit cpu quad did not cover the centre");
        let ratio = gpu / cpu;
        assert!(
            (ratio - 1.0).abs() < 0.02,
            "instanced scRGB {gpu} != cpu {cpu} at tint 0xFF3040 light 0x00F0 (ratio {ratio})"
        );
    }
}

#[test]
#[ignore = "daily"]
fn explicit_particle_layers_render_in_device_sized_draws() {
    use super::particle_draws::{EXPLICIT_PARTICLE_OVERLAY, ParticleDraws, ParticlePipelines};
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame_lit(
        [1, 1],
        &[255; 4],
        Mat4::IDENTITY,
        0.0,
        [SIZE; 2],
        &[255; 16 * 16 * 4],
    )
    .expect("software Vulkan frame");
    let shader = frame
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
    let layout = frame
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&frame.bindings)],
            immediate_size: 0,
        });
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    let reuse = reuse::Reuse::default();
    let kept = reuse.on(&frame.device);
    let pipelines = ParticlePipelines::new(entity_pipeline_factory(
        &kept,
        &layout,
        &layout,
        &shader,
        &buffers,
        wgpu::TextureFormat::Rgba8Unorm,
        false,
    ));
    let quad = |x: f32, y: f32, z, color, back: bool| {
        let mut corners = [(0.25, -0.25), (0.25, 0.25), (-0.25, 0.25), (-0.25, -0.25)];
        if back {
            corners.reverse();
        }
        corners.map(|(dx, dy)| Vertex {
            pos: [x + dx, y + dy, z],
            uv: [0.5; 2],
            layer: EXPLICIT_PARTICLE_OVERLAY,
            color,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        })
    };
    let opaque = [
        quad(-0.5, 0.5, 0.75, 0x8000_00ff, false),
        quad(-0.5, 0.5, 0.5, 0xff00_ff00, false),
        quad(-0.5, -0.5, 0.75, 0x08ff_ffff, false),
        quad(0.5, -0.5, 0.75, 0xffff_ffff, true),
        quad(0.0, -0.5, 0.75, 0xffff_0000, false),
    ];
    let translucent = [
        quad(0.5, 0.5, 0.75, 0x8000_00ff, false),
        quad(0.5, 0.5, 0.5, 0xff00_ff00, false),
    ];
    let mut draws = [ParticleDraws::default(), ParticleDraws::default()];
    let mut uploads = super::upload::Uploads::new(&frame.device);
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    for (draws, vertices) in draws
        .iter_mut()
        .zip([opaque.as_flattened(), translucent.as_flattened()])
    {
        let limit = size_of::<[Vertex; 4]>() as u64;
        assert_eq!(
            draws.prepare(&frame.device, &mut uploads, &mut encoder, vertices, limit),
            1
        );
    }
    let index = super::Renderer::quad_indices(&frame.device, 1);
    {
        let color = frame.color.create_view(&Default::default());
        let depth = frame.depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                stencil_ops: None,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &frame.group, &[]);
        pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
        for (draws, pipeline) in draws
            .iter()
            .zip([&pipelines.opaque, &pipelines.translucent])
        {
            pass.set_pipeline(pipeline);
            draws.draw(&mut pass);
        }
    }
    uploads.finish(&encoder);
    let pixels = readback(&frame, encoder).expect("particle output readback");
    let rgb = |x, y| {
        let at = ((y * SIZE + x) * 4) as usize;
        &pixels[at..at + 3]
    };
    assert_eq!(
        rgb(SIZE / 4, SIZE / 4),
        [255, 0, 0],
        "opaque replaces covered pixels and writes depth"
    );
    assert_eq!(
        rgb(SIZE * 3 / 4, SIZE / 4),
        [128, 0, 0],
        "translucent blends and writes depth"
    );
    assert_eq!(rgb(SIZE / 4, SIZE * 3 / 4), [0, 0, 0], "alpha cutout");
    assert_eq!(
        rgb(SIZE * 3 / 4, SIZE * 3 / 4),
        [0, 0, 0],
        "back face culling"
    );
    assert_eq!(
        rgb(SIZE / 2, SIZE * 3 / 4),
        [0, 0, 255],
        "the final upload partition was drawn"
    );
}

/// Draws a real screen batch through the engine HUD pipeline on software Vulkan.
/// Uses the same adapter, atlas upload and readback as the entity pixel fixtures.
pub fn render_hud(
    batch: &HudBatch,
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame(
        [atlas.width, atlas.height],
        &atlas.pixels(),
        Mat4::IDENTITY,
        0.0,
        size,
    )?;
    let sampler = frame.device.create_sampler(&Default::default());
    let mut hud = crate::hud::HudRenderer::new(
        &frame.device,
        wgpu::TextureFormat::Rgba8Unorm,
        &frame.atlas,
        &frame.atlas,
        &frame.atlas,
        &sampler,
        false,
    )
    .map_err(|error| error.to_string())?;
    // As the game's renderer has it (gfx.rs): the batch's atlas patches, a map's or a mod's
    // image, upload before the draw.
    hud.atlas_texture = Some(frame.atlas_texture.clone());
    let mut uploads = super::upload::Uploads::new(&frame.device);
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    let color = frame.color.create_view(&Default::default());
    let depth = frame.depth.create_view(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    hud.prepare(&frame.device, &mut uploads, &mut encoder, batch, size);
    uploads.finish(&encoder);
    hud.draw(&mut encoder, &color, &depth, batch, size);
    readback(&frame, encoder)
}
