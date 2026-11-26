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
    lightmap: wgpu::TextureView,
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
        0.0,
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
    glint_alpha: f32,
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
    globals.fog_ends[3] = glint_alpha;
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
        lightmap: light_view,
    })
}

fn readback(frame: &Frame, encoder: wgpu::CommandEncoder) -> std::result::Result<Vec<u8>, String> {
    readback_texture(&frame.device, &frame.queue, &frame.color, encoder)
}

fn readback_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    color: &wgpu::Texture,
    mut encoder: wgpu::CommandEncoder,
) -> std::result::Result<Vec<u8>, String> {
    let (width, height) = (color.width(), color.height());
    let row_bytes = width * 4;
    let pitch = row_bytes.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(pitch) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
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
        None,
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
        None,
    )
}

/// `view_clip` selects the secondary-view entity pass, with or without oblique clipping.
fn render_layers_lit(
    vertices: &[Vertex],
    see_through: &[Vertex],
    atlas_size: [u32; 2],
    atlas: &[u8],
    view_proj: Mat4,
    hdr: bool,
    lightmap: &[u8],
    view_clip: Option<bool>,
) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame_lit(
        atlas_size,
        atlas,
        view_proj,
        0.0,
        [SIZE, SIZE],
        lightmap,
        if view_clip.is_some() { 1.0 } else { 0.0 },
    )?;
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
    let make_pipeline = entity_pipeline_factory(
        &kept,
        &layout,
        &layout,
        &shader,
        &buffers,
        wgpu::TextureFormat::Rgba8Unorm,
        false,
    );
    let make_with = |fragment, depth_write, bias, blend, write_mask, compare, cull_mode| {
        make_pipeline(EntityPipelineOptions {
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
        if view_clip == Some(true) {
            "fs_entity_clipped"
        } else {
            "fs_entity"
        },
        true,
        wgpu::DepthBiasState {
            constant: if view_clip == Some(true) { 0 } else { 128 },
            slope_scale: 0.0,
            clamp: 0.0,
        },
        wgpu::BlendState::ALPHA_BLENDING,
        wgpu::ColorWrites::ALL,
    );
    let (world_view, vertices) = if let Some(clipped) = view_clip {
        let mut batch = super::entity_batch::EntityBatch::default();
        let [ordinary, ..] = batch
            .update_within(vertices, &[], &[], &[], &[], false, usize::MAX)
            .materials;
        let draws = super::world_views::EntityDraws {
            ordinary,
            name_tags: 0..0,
            foil: batch.armor_foil,
        };
        (
            Some((draws, make_pipeline(energy_swirl_options(clipped)))),
            batch.combined,
        )
    } else {
        (None, [vertices, see_through].concat())
    };
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
    let entity_quads = (vertices.len() - see_through.len()) as u32 / 4;
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
        if let Some((draws, foil)) = &world_view {
            if !indices.is_empty() {
                pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                draws.draw(&mut pass, [&pipeline, foil], &frame.group, &vertex);
            }
        } else if !indices.is_empty() {
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

    #[test]
    #[ignore = "daily"]
    fn secondary_views_draw_foil_without_unrelated_energy_swirl() {
        let foil = quad(-0.3, 0.3, 0.5, 0xFF00_FF00).map(|mut v| {
            v.layer = ARMOR_GLINT_ONLY_OVERLAY;
            v
        });
        let swirl = quad(0.5, 1.0, 0.5, 0xFFFF_FFFF).map(|mut v| {
            v.layer = ENERGY_SWIRL_OVERLAY;
            v
        });
        for clipped in [false, true] {
            let pixels = render_layers_lit(
                &[foil, swirl].concat(),
                &[],
                [1, 1],
                &[128, 0, 0, 255],
                Mat4::IDENTITY,
                false,
                &FLAT_LIGHTMAP.repeat(16 * 16),
                Some(clipped),
            )
            .expect("software Vulkan secondary-view pixels");
            // GLINT adds the square of a 128/255 red texel, without writing alpha.
            assert_eq!(centre(&pixels), [64, 0, 0, 255]);
            let right = (SIZE / 2 * SIZE + 7 * SIZE / 8) as usize;
            assert_eq!(pixels.as_chunks::<4>().0[right], [0, 0, 0, 255]);
        }
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
    render_pack_with(vertices, spans, None)
}

/// [`render_pack`], with `gbuffers_entities` writing what it samples from `dynamic_texture` instead of its
/// flat colour when given: a 1 x 1 dynamic texture of that colour (handle 1) the draws of that
/// handle (`DYNAMIC_TEXTURE_*_OVERLAY` quads naming it) bind as their texture.
fn render_pack_with(
    vertices: &[Vertex],
    spans: &[pack_draw::EntitySpan],
    dynamic_texture: Option<[u8; 4]>,
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
    for (i, (program, rgba)) in programs.into_iter().zip(PACK_COLOURS).enumerate() {
        let sampled = dynamic_texture.is_some() && i == 0;
        let (vertex, fragment) = if sampled {
            (
                "#version 120\nvarying vec2 uv;\nvoid main() { uv = gl_MultiTexCoord0.xy; \
                 gl_Position = vec4(gl_Vertex.xy * 0.5, 0.5, 1.0); }\n"
                    .to_string(),
                "#version 120\nuniform sampler2D texture;\nvarying vec2 uv;\n\
                 void main() { gl_FragData[0] = texture2D(texture, uv); }\n"
                    .to_string(),
            )
        } else {
            (
                "#version 120\nvoid main() { gl_Position = vec4(gl_Vertex.xy * 0.5, 0.5, 1.0); }\n"
                    .to_string(),
                format!(
                    "#version 120\nvoid main() {{ gl_FragData[0] = {}; }}\n",
                    colour(rgba)
                ),
            )
        };
        add(&format!("{program}.vsh"), &vertex)
            .and_then(|()| add(&format!("{program}.fsh"), &fragment))
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
    let store = crate::hud::dynamic_textures::Store::default();
    if let Some(rgba) = dynamic_texture {
        let mut uploads = super::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        store.apply(
            &device,
            &mut uploads,
            &mut encoder,
            None,
            &[crate::hud::dynamic_textures::Command::Create {
                handle: 1,
                size: [1, 1],
                mip_levels: 1,
                rgba: rgba.to_vec(),
            }],
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
    }
    post.resolve_entity_textures(|handle| store.view(handle).expect("the texture was created"));
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
        Default::default(),
        |_, _| false,
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
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 90.0,
        cull_fov_degrees: 90.0,
        bob: Mat4::IDENTITY,
        view_transform: Mat4::IDENTITY,
        spin: Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        shadow_distance: None,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: false,
        sky_pass: false,
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

    #[test]
    #[ignore = "daily"]
    fn block_display_83224_has_no_seam_at_x1026() {
        use super::super::*;
        use bytemuck::Zeroable;
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(adapter) = crate::gfx::test_gpu::adapter() else {
            return;
        };

        // Pack a minimal HudAtlas containing opaque block/lime_concrete so its UV placement
        // and border padding match real client builds.
        let root = std::env::temp_dir().join(format!("sunrise-83224-seam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let blocks = root.join("assets/minecraft/textures/block");
        std::fs::create_dir_all(&blocks).unwrap();
        let file = std::fs::File::create(blocks.join("lime_concrete.png")).unwrap();
        let mut enc = png::Encoder::new(file, 16, 16);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .unwrap()
            .write_image_data(&[100u8, 180, 30, 255].repeat(16 * 16))
            .unwrap();
        let atlas = crate::hud::HudAtlas::build(
            &crate::resource_source::ResourceSource::directory(root.clone()),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let sprite = atlas.sprites["block/lime_concrete"];

        // Build EID 83224's exact 6 faces from its deterministic center-billboard pose.
        let ent_y = f64::from_bits(0x4072_d9eb_851e_b852);
        let cam_y = f64::from_bits(0x4072_d9eb_94c0_0000);
        let mut pose = crate::camera::joml::Mat4::IDENTITY;
        pose.translate(0.0, (ent_y - cam_y) as f32, 0.0);
        pose.rotate(crate::camera::joml::Quat::rotation_y(-std::f32::consts::PI));
        let mut tx = crate::camera::joml::Mat4::IDENTITY;
        tx.translate(
            f32::from_bits(0x4039_c28f),
            f32::from_bits(0x4001_0000),
            f32::from_bits(0xc089_eb85),
        );
        tx.scale(
            f32::from_bits(0x3fce_6666),
            f32::from_bits(0x3d5c_28f6),
            f32::from_bits(0x3ca3_d70a),
        );
        pose = pose.mul_affine(&tx);

        let faces: [([[f32; 3]; 4], [i8; 4]); 6] = [
            // down, up, north, south, west, east after 180° Y billboard orientation.
            (
                [
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 0.0, 1.0],
                ],
                [0, -127, 0, crate::mesher::SHADED_CULL_W],
            ),
            (
                [
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0],
                    [1.0, 1.0, 0.0],
                ],
                [0, 127, 0, crate::mesher::SHADED_CULL_W],
            ),
            (
                [
                    [1.0, 1.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
                [0, 0, 127, crate::mesher::SHADED_CULL_W],
            ),
            (
                [
                    [0.0, 1.0, 1.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                ],
                [0, 0, -127, crate::mesher::SHADED_CULL_W],
            ),
            (
                [
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, 1.0, 1.0],
                ],
                [127, 0, 0, crate::mesher::SHADED_CULL_W],
            ),
            (
                [
                    [1.0, 1.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                ],
                [-127, 0, 0, crate::mesher::SHADED_CULL_W],
            ),
        ];
        let mut verts = Vec::with_capacity(24);
        let uvs = [
            [sprite.u0, sprite.v0],
            [sprite.u0, sprite.v1],
            [sprite.u1, sprite.v1],
            [sprite.u1, sprite.v0],
        ];
        for (corners, nb) in faces {
            for (k, c) in corners.into_iter().enumerate() {
                verts.push(Vertex {
                    pos: pose.transform_position(c),
                    uv: uvs[k],
                    layer: 0,
                    color: u32::MAX,
                    light: 0x00F0_00F0,
                    normal: nb,
                    glint_uv: [0.0, 0.0],
                });
            }
        }

        let view_proj =
            world_projection(70.0, 1280.0 / 800.0, 2048.0) * crate::camera::view_matrix(0.0, 0.0);

        let mut limits = wgpu::Limits::default();
        limits.max_texture_dimension_2d = limits
            .max_texture_dimension_2d
            .max(atlas.width)
            .max(atlas.height);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        }))
        .expect("device");

        let tex = |w, h, fmt, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage,
                view_formats: &[],
            })
        };
        let color = tex(
            1280,
            800,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = tex(
            1280,
            800,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let atlas_tex = tex(
            atlas.width,
            atlas.height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            atlas_tex.as_image_copy(),
            &atlas.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas.width * 4),
                rows_per_image: None,
            },
            atlas_tex.size(),
        );
        let lm_tex = tex(
            16,
            16,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            lm_tex.as_image_copy(),
            &[255u8; 16 * 16 * 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: None,
            },
            lm_tex.size(),
        );
        let sampler = device.create_sampler(&Default::default());
        let mut globals = Globals::zeroed();
        globals.view_proj = view_proj.to_cols_array_2d();
        globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entity_binding_entries(),
        });
        let atlas_view = atlas_tex.create_view(&Default::default());
        let lm_view = lm_tex.create_view(&Default::default());
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
                    resource: wgpu::BindingResource::TextureView(&lm_view),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ENTITY_ATTRIBUTES,
        })];
        let reuse = reuse::Reuse::default();
        let kept = reuse.on(&device);
        let pipeline = entity_pipeline_factory(
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
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: wgpu::DepthBiasState {
                constant: 128,
                slope_scale: 0.0,
                clamp: 0.0,
            },
            blend: wgpu::BlendState::ALPHA_BLENDING,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity",
            write_mask: wgpu::ColorWrites::ALL,
        });
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices: Vec<u32> = (0..6u32)
            .flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3])
            .collect();
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let cv = color.create_view(&Default::default());
            let dv = depth.create_view(&Default::default());
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &cv,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..36, 0, 0..1);
        }
        let pitch = (1280 * 4u32).div_ceil(256) * 256;
        let rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(pitch) * 800,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &rb,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: None,
                },
            },
            color.size(),
        );
        queue.submit([enc.finish()]);
        rb.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = rb.slice(..).get_mapped_range().unwrap();
        let px_at = |x: u32, y: u32| -> [u8; 3] {
            let off = (y * pitch + x * 4) as usize;
            [mapped[off], mapped[off + 1], mapped[off + 2]]
        };
        assert_ne!(px_at(1025, 126), [0, 0, 0], "shaded west face at x=1025");
        assert_ne!(px_at(1026, 132), [0, 0, 0], "shaded down face at y=132");
        for y in 124..=131 {
            assert_eq!(
                px_at(1026, y),
                px_at(1027, y),
                "seam at (1026, {y}) on {}: got {:?}, front face at x=1027 is {:?}",
                adapter.get_info().name,
                px_at(1026, y),
                px_at(1027, y),
            );
        }
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
        0.0,
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

#[test]
#[ignore = "daily"]
fn additive_particles_brighten_when_overlapping_without_writing_depth() {
    use super::particle_draws::{EXPLICIT_PARTICLE_OVERLAY, ParticleDraws, ParticlePipelines};
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame_lit(
        [1, 1],
        &[255; 4],
        Mat4::IDENTITY,
        0.0,
        [SIZE; 2],
        &[255; 16 * 16 * 4],
        0.0,
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
    let quad = |x: f32, y: f32, z, color| {
        [(0.25, -0.25), (0.25, 0.25), (-0.25, 0.25), (-0.25, -0.25)].map(|(dx, dy)| Vertex {
            pos: [x + dx, y + dy, z],
            uv: [0.5; 2],
            layer: EXPLICIT_PARTICLE_OVERLAY,
            color,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        })
    };
    let opaque = [quad(-0.5, -0.5, 0.75, 0xff00_0040)];
    let translucent = [quad(0.5, -0.5, 0.5, 0x80ff_0000)];
    let additive = [
        // Single additive particle over black.
        quad(-0.5, 0.5, 0.75, 0x8000_0080),
        // Two overlapping additive particles (second further in reverse-Z, testing no depth write).
        quad(0.5, 0.5, 0.75, 0x8000_0080),
        quad(0.5, 0.5, 0.5, 0x8000_0080),
        // Behind the opaque blocker (testing depth test on).
        quad(-0.5, -0.5, 0.5, 0xff00_ff00),
        // In front of the translucent particle, drawn after it.
        quad(0.5, -0.5, 0.75, 0x8000_00ff),
    ];
    let mut draws = [
        ParticleDraws::default(),
        ParticleDraws::default(),
        ParticleDraws::default(),
    ];
    let mut uploads = super::upload::Uploads::new(&frame.device);
    let mut encoder = frame.device.create_command_encoder(&Default::default());
    for (draws, vertices) in draws.iter_mut().zip([
        opaque.as_flattened(),
        translucent.as_flattened(),
        additive.as_flattened(),
    ]) {
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
        for (draws, pipeline) in draws.iter().zip([
            &pipelines.opaque,
            &pipelines.translucent,
            &pipelines.additive,
        ]) {
            pass.set_pipeline(pipeline);
            draws.draw(&mut pass);
        }
    }
    uploads.finish(&encoder);
    let pixels = readback(&frame, encoder).expect("additive particle output readback");
    let rgb = |x, y| {
        let at = ((y * SIZE + x) * 4) as usize;
        &pixels[at..at + 3]
    };
    let one = rgb(SIZE / 4, SIZE / 4);
    let two = rgb(SIZE * 3 / 4, SIZE / 4);
    assert!(
        one[0] > 0 && one[1] == 0 && one[2] == 0,
        "one additive particle lights up over black: {one:?}"
    );
    assert!(
        two[0] > one[0] && two[1] == 0 && two[2] == 0,
        "two overlapping additive particles brighten further without depth write: one={one:?}, two={two:?}"
    );
    assert_eq!(
        rgb(SIZE / 4, SIZE * 3 / 4),
        [64, 0, 0],
        "additive particle behind an opaque surface is depth-rejected"
    );
    let over_translucent = rgb(SIZE * 3 / 4, SIZE * 3 / 4);
    assert!(
        over_translucent[0] > 0 && over_translucent[2] > 0,
        "additive particle drawn after translucent adds onto it: {over_translucent:?}"
    );
}

/// Draws a real screen batch through the engine HUD pipeline on software Vulkan.
/// Uses the same adapter, atlas upload and readback as the entity pixel fixtures.
pub fn render_hud(
    batch: &HudBatch,
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
) -> std::result::Result<Vec<u8>, String> {
    render_hud_lit(batch, atlas, size, &[255u8; 16 * 16 * 4])
}

/// [`render_hud`] with a custom 16x16 RGBA `lightmap` (block level across, sky level down)
/// bound to the HUD lightmap slot for lightmapped special-model GUI draws.
pub fn render_hud_lit(
    batch: &HudBatch,
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
    lightmap: &[u8],
) -> std::result::Result<Vec<u8>, String> {
    let mut frames = render_hud_frames_lit(&[batch], atlas, size, lightmap)?;
    Ok(frames.remove(0))
}

/// [`render_hud`] for successive frames of one renderer, each batch drawn after the one before
/// and read back on its own: what a batch's dynamic texture commands made in an earlier frame is
/// still there in the next.
pub fn render_hud_frames(
    batches: &[&HudBatch],
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
) -> std::result::Result<Vec<Vec<u8>>, String> {
    render_hud_frames_lit(batches, atlas, size, &[255u8; 16 * 16 * 4])
}

/// [`render_hud_frames`] with a custom 16x16 RGBA `lightmap`, as [`render_hud_lit`].
pub fn render_hud_frames_lit(
    batches: &[&HudBatch],
    atlas: &crate::hud::HudAtlas,
    size: [u32; 2],
    lightmap: &[u8],
) -> std::result::Result<Vec<Vec<u8>>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let frame = software_frame_lit(
        [atlas.width, atlas.height],
        &atlas.pixels(),
        Mat4::IDENTITY,
        0.0,
        size,
        lightmap,
        0.0,
    )?;
    let sampler = frame.device.create_sampler(&Default::default());
    let mut hud = crate::hud::HudRenderer::new(
        &frame.device,
        wgpu::TextureFormat::Rgba8Unorm,
        &frame.atlas,
        &frame.atlas,
        &frame.atlas,
        &frame.lightmap,
        &sampler,
        false,
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    // As the game's renderer has it (gfx.rs): the batch's atlas patches, a map's or a mod's
    // image, upload before the draw.
    hud.atlas_texture = Some(frame.atlas_texture.clone());
    let mut uploads = super::upload::Uploads::new(&frame.device);
    let color = frame.color.create_view(&Default::default());
    let depth = frame.depth.create_view(&Default::default());
    let mut out = Vec::new();
    for batch in batches {
        let mut encoder = frame.device.create_command_encoder(&Default::default());
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
        out.push(readback(&frame, encoder)?);
    }
    Ok(out)
}

/// Exposed reversed-Z world projection for cross-crate entity and OIT pixel tests.
pub fn test_world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
    super::world_projection(fov_degrees, aspect, far)
}

/// Inputs for [`render_entity_world`], running `EntityBatch`, banner-pattern / glint-only
/// overlays, and (when `oit` is true) the production improved-transparency stages on software Vulkan.
pub struct EntityWorldDraw<'a> {
    pub vertices: &'a [Vertex],
    pub atlas_size: [u32; 2],
    pub atlas: &'a [u8],
    pub glint_size: [u32; 2],
    pub glint: &'a [u8],
    pub view_proj: Mat4,
    pub glint_alpha: f32,
    pub oit: bool,
}

/// Renders world entity quads through `EntityBatch`, the production entity / banner-pattern /
/// glint-only pipelines, and (when `draw.oit` is true) the production OIT shaders.
pub fn render_entity_world(draw: EntityWorldDraw<'_>) -> std::result::Result<Vec<u8>, String> {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = software_adapter()?;
    let required_features = if draw.oit {
        super::oit::features(&adapter)
    } else {
        wgpu::Features::empty()
    };
    let mut required_limits = wgpu::Limits::default();
    required_limits.max_texture_dimension_2d = required_limits
        .max_texture_dimension_2d
        .max(draw.atlas_size[0])
        .max(draw.atlas_size[1])
        .max(draw.glint_size[0])
        .max(draw.glint_size[1]);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        required_limits,
        ..Default::default()
    }))
    .map_err(|error| format!("software Vulkan device request failed: {error}"))?;
    if draw.oit && !super::oit::supported(&device) {
        return Err("software Vulkan adapter does not support OIT (FLOAT32_BLENDABLE)".into());
    }
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
    let upload = |tex: &wgpu::Texture, size: [u32; 2], pixels: &[u8]| {
        queue.write_texture(
            tex.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: None,
            },
            tex.size(),
        );
    };
    let color = texture(
        SIZE,
        SIZE,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        SIZE,
        SIZE,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let atlas_tex = texture(
        draw.atlas_size[0],
        draw.atlas_size[1],
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    upload(&atlas_tex, draw.atlas_size, draw.atlas);
    let glint_tex = texture(
        draw.glint_size[0],
        draw.glint_size[1],
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    upload(&glint_tex, draw.glint_size, draw.glint);
    let lightmap = texture(
        16,
        16,
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    upload(&lightmap, [16, 16], &[255u8; 4].repeat(16 * 16));
    let sampler = device.create_sampler(&Default::default());
    let glint_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        ..Default::default()
    });
    let mut globals = Globals::zeroed();
    globals.view_proj = draw.view_proj.to_cols_array_2d();
    globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
    globals.fog_ends[3] = draw.glint_alpha;
    globals.hdr_light = [0.0, 0.0, 1.0, 0.0];
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&globals),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entity_binding_entries(),
    });
    let atlas_view = atlas_tex.create_view(&Default::default());
    let glint_view = glint_tex.create_view(&Default::default());
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
                resource: wgpu::BindingResource::TextureView(&glint_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&glint_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&glint_view),
            },
        ],
    });
    let mut batch = EntityBatch::default();
    let counts = batch
        .update_within(draw.vertices, &[], &[], &[], &[], draw.oit, usize::MAX)
        .materials;
    let [
        entity_quads,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        translucent_item_quads,
        oit_entity_quads,
        fluid_quads,
        world_text_quads,
        world_text_offset_quads,
        native_solid_quads,
        native_cutout_quads,
        native_cutout_mipped_quads,
        native_translucent_quads,
        banner_pattern_quads,
        glint_only_quads,
        text_offset_solid_quads,
        text_normal_solid_quads,
        dynamic_solid_quads,
        dynamic_cutout_quads,
        dynamic_translucent_quads,
        name_tag_solid_quads,
        name_tag_quads,
    ] = counts;
    let text_solid_quads = text_offset_solid_quads + text_normal_solid_quads;
    let world_quads: u32 = counts.iter().sum();
    let base_quads = world_quads
        - oit_entity_quads
        - fluid_quads
        - world_text_quads
        - world_text_offset_quads
        - native_solid_quads
        - native_cutout_quads
        - native_cutout_mipped_quads
        - native_translucent_quads
        - dynamic_solid_quads
        - dynamic_cutout_quads
        - dynamic_translucent_quads
        - banner_pattern_quads
        - glint_only_quads
        - text_solid_quads
        - name_tag_solid_quads
        - name_tag_quads;
    let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&batch.combined),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices: Vec<u32> = (0..world_quads)
        .flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3])
        .collect();
    let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bindings)],
        immediate_size: 0,
    });
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ENTITY_ATTRIBUTES,
    })];
    let reuse = reuse::Reuse::default();
    let kept = reuse.on(&device);
    let make_pipeline = |fragment, depth_write, compare, bias, blend| {
        entity_pipeline_factory(&kept, &layout, &layout, &shader, &buffers, format, false)(
            EntityPipelineOptions {
                label: fragment,
                cull_mode: None,
                depth_write,
                compare,
                bias,
                blend,
                clouds: false,
                vertex: "vs_entity",
                fragment,
                write_mask: wgpu::ColorWrites::ALL,
            },
        )
    };
    let entity_bias = wgpu::DepthBiasState {
        constant: 128,
        slope_scale: 0.0,
        clamp: 0.0,
    };
    let glint_blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    let entity_pipeline = make_pipeline(
        "fs_entity",
        true,
        wgpu::CompareFunction::GreaterEqual,
        entity_bias,
        wgpu::BlendState::ALPHA_BLENDING,
    );
    let banner_pattern_pipeline = make_pipeline(
        "fs_banner_pattern",
        false,
        wgpu::CompareFunction::GreaterEqual,
        entity_bias,
        wgpu::BlendState::ALPHA_BLENDING,
    );
    let glint_only_pipeline = make_pipeline(
        "fs_glint",
        false,
        wgpu::CompareFunction::Equal,
        entity_bias,
        glint_blend,
    );
    let translucent_item_pipeline = make_pipeline(
        "fs_entity",
        true,
        wgpu::CompareFunction::GreaterEqual,
        wgpu::DepthBiasState::default(),
        wgpu::BlendState::ALPHA_BLENDING,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    {
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
        if world_quads > 0 {
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            if entity_quads > 0 {
                pass.set_pipeline(&entity_pipeline);
                pass.draw_indexed(0..entity_quads * 6, 0, 0..1);
            }
            if banner_pattern_quads > 0 {
                pass.set_pipeline(&banner_pattern_pipeline);
                pass.draw_indexed(
                    0..banner_pattern_quads * 6,
                    ((world_quads
                        - name_tag_solid_quads
                        - name_tag_quads
                        - text_solid_quads
                        - glint_only_quads
                        - banner_pattern_quads)
                        * 4) as i32,
                    0..1,
                );
            }
            if glint_only_quads > 0 {
                pass.set_pipeline(&glint_only_pipeline);
                pass.draw_indexed(
                    0..glint_only_quads * 6,
                    ((world_quads
                        - name_tag_solid_quads
                        - name_tag_quads
                        - text_solid_quads
                        - glint_only_quads)
                        * 4) as i32,
                    0..1,
                );
            }
            if !draw.oit && translucent_item_quads > 0 {
                pass.set_pipeline(&translucent_item_pipeline);
                pass.draw_indexed(
                    0..translucent_item_quads * 6,
                    ((base_quads - translucent_item_quads) * 4) as i32,
                    0..1,
                );
            }
        }
    }
    if draw.oit && (translucent_item_quads > 0 || oit_entity_quads > 0) {
        let oit_entity_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                (entity_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_features.wgsl"))
                .into(),
            ),
        });
        let oit_world_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                (terrain_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_world.wgsl"))
                .into(),
            ),
        });
        let oit_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[
                draw.view_proj.z_axis.z,
                draw.view_proj.w_axis.z,
                0.0f32,
                0.0f32,
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let att = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let bounds = texture(
            SIZE,
            SIZE,
            wgpu::TextureFormat::Rgba32Float,
            att | wgpu::TextureUsages::COPY_SRC,
        );
        let culled = texture(
            SIZE,
            SIZE,
            wgpu::TextureFormat::Rgba32Float,
            att | wgpu::TextureUsages::COPY_DST,
        );
        let coeff0 = texture(SIZE, SIZE, wgpu::TextureFormat::Rgba16Float, att);
        let coeff1 = texture(SIZE, SIZE, wgpu::TextureFormat::Rgba16Float, att);
        let accumulated = texture(SIZE, SIZE, wgpu::TextureFormat::Rgba16Float, att);
        let bounds_view = bounds.create_view(&Default::default());
        let culled_view = culled.create_view(&Default::default());
        let coeff0_view = coeff0.create_view(&Default::default());
        let coeff1_view = coeff1.create_view(&Default::default());
        let accum_view = accumulated.create_view(&Default::default());
        let oit_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let oit_tex = |binding| {
            oit_entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let oit_params = oit_entry(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        let make_oit_layout = |entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        };
        let oit_layouts = [
            make_oit_layout(&[oit_params]),
            make_oit_layout(&[oit_params, oit_tex(1)]),
            make_oit_layout(&[oit_params, oit_tex(1), oit_tex(2), oit_tex(3)]),
            make_oit_layout(&[oit_params, oit_tex(1), oit_tex(2), oit_tex(3), oit_tex(4)]),
        ];
        let make_oit_bg = |idx: usize, views: &[&wgpu::TextureView]| {
            let mut entries = Vec::with_capacity(1 + views.len());
            entries.push(wgpu::BindGroupEntry {
                binding: 0,
                resource: oit_uniform.as_entire_binding(),
            });
            for (i, view) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: (i + 1) as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &oit_layouts[idx],
                entries: &entries,
            })
        };
        let bg_bounds = make_oit_bg(0, &[]);
        let bg_cull = make_oit_bg(1, &[&bounds_view]);
        let bg_transmittance = make_oit_bg(1, &[&culled_view]);
        let bg_accumulate = make_oit_bg(2, &[&culled_view, &coeff0_view, &coeff1_view]);
        let bg_composite = make_oit_bg(3, &[&culled_view, &coeff0_view, &coeff1_view, &accum_view]);
        let oit_entity = [
            (super::oit::Stage::DepthBounds, "fs_oit_entity_bounds"),
            (
                super::oit::Stage::Transmittance,
                "fs_oit_entity_transmittance",
            ),
            (super::oit::Stage::Accumulate, "fs_oit_entity_accumulate"),
        ]
        .map(|(stage, fragment)| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&bindings), None, Some(&oit_layouts[stage as usize])],
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &oit_entity_shader,
                    entry_point: Some("vs_entity"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &oit_entity_shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &super::oit::stage_targets(stage),
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let oit_screen = |group_idx: usize,
                          fragment: &str,
                          targets: &[Option<wgpu::ColorTargetState>],
                          depth_compare: wgpu::CompareFunction| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[None, None, Some(&oit_layouts[group_idx])],
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &oit_world_shader,
                    entry_point: Some("vs_sky"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &oit_world_shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let cull_pipeline = oit_screen(
            1,
            "fs_oit_cull",
            &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            wgpu::CompareFunction::GreaterEqual,
        );
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let composite_pipeline = oit_screen(
            3,
            "fs_oit_composite",
            &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: premultiplied,
                    alpha: premultiplied,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            wgpu::CompareFunction::Always,
        );
        let color_att = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let draw_stage = |pass: &mut wgpu::RenderPass<'_>,
                          pipeline: &wgpu::RenderPipeline,
                          oit_bg: &wgpu::BindGroup| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(2, oit_bg, &[]);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            if translucent_item_quads > 0 {
                pass.draw_indexed(
                    0..translucent_item_quads * 6,
                    ((base_quads - translucent_item_quads) * 4) as i32,
                    0..1,
                );
            }
            if oit_entity_quads > 0 {
                pass.draw_indexed(0..oit_entity_quads * 6, (base_quads * 4) as i32, 0..1);
            }
        };
        let read_depth = || {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: None,
                stencil_ops: None,
            })
        };
        let rw_depth = || {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            })
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[color_att(
                    &bounds_view,
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: -(f32::MAX as f64),
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                )],
                depth_stencil_attachment: read_depth(),
                ..Default::default()
            });
            draw_stage(&mut pass, &oit_entity[0], &bg_bounds);
        }
        encoder.copy_texture_to_texture(
            bounds.as_image_copy(),
            culled.as_image_copy(),
            bounds.size(),
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[color_att(&culled_view, wgpu::LoadOp::Load)],
                depth_stencil_attachment: rw_depth(),
                ..Default::default()
            });
            pass.set_pipeline(&cull_pipeline);
            pass.set_bind_group(2, &bg_cull, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[
                    color_att(&coeff0_view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                    color_att(&coeff1_view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                ],
                depth_stencil_attachment: read_depth(),
                ..Default::default()
            });
            draw_stage(&mut pass, &oit_entity[1], &bg_transmittance);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[color_att(
                    &accum_view,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                )],
                depth_stencil_attachment: read_depth(),
                ..Default::default()
            });
            draw_stage(&mut pass, &oit_entity[2], &bg_accumulate);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[color_att(&color_view, wgpu::LoadOp::Load)],
                depth_stencil_attachment: rw_depth(),
                ..Default::default()
            });
            pass.set_pipeline(&composite_pipeline);
            pass.set_bind_group(2, &bg_composite, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    readback_texture(&device, &queue, &color, encoder)
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn hud_lightmapped_special_item_samples_frame_lightmap_without_disturbing_neighbour() {
    let atlas = crate::hud::HudAtlas::boot();
    let r = atlas.white;
    let uv = [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]];
    let mut batch = HudBatch::new(
        [64, 16],
        crate::hud::GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    let light_low = 0x0100_0000u32 | (2u32 * 16) | ((3u32 * 16) << 8);
    let light_high = 0x0100_0000u32 | (14u32 * 16) | ((15u32 * 16) << 8);
    // Low explicit packed light (block=2, sky=3).
    batch.quad_lit(
        [[0.0, 0.0], [16.0, 0.0], [16.0, 16.0], [0.0, 16.0]],
        uv,
        u32::MAX,
        light_low,
    );
    // High explicit packed light (block=14, sky=15).
    batch.quad_lit(
        [[16.0, 0.0], [32.0, 0.0], [32.0, 16.0], [16.0, 16.0]],
        uv,
        u32::MAX,
        light_high,
    );
    // Default unlightmapped HUD neighbour (`light = 0`): must remain full white.
    batch.quad(
        [[32.0, 0.0], [48.0, 0.0], [48.0, 16.0], [32.0, 16.0]],
        uv,
        u32::MAX,
    );

    let pixels = render_hud_lit(&batch, &atlas, [64, 16], &graded_lightmap())
        .expect("software Vulkan HUD lightmap render");
    let px = |x: u32, y: u32| {
        let i = ((y * 64 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    let low = px(8, 8);
    let high = px(24, 8);
    let neighbour = px(40, 8);
    assert!(
        low[0] < high[0],
        "higher packed light must sample brighter from graded lightmap: low={low:?}, high={high:?}"
    );
    assert!(
        high[0] <= neighbour[0],
        "lightmapped quad cannot exceed full-white neighbour: high={high:?}, neighbour={neighbour:?}"
    );
    assert_eq!(
        neighbour,
        [255, 255, 255, 255],
        "default HUD neighbour must remain unaffected by frame lightmap"
    );
}

/// `BlockEntityDrawContext::textured_quad`'s quads through `EntityBatch`, the production
/// `DynamicEntity` bind groups and entity pipeline, on software Vulkan.
#[cfg(test)]
mod dynamic_quads {
    use super::*;
    use crate::hud::dynamic_textures::{Command, Store};
    use dynamic_entity::DynamicEntity;
    use sunrise_render_core::entity::{
        DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY,
        DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, dynamic_texture_bits,
    };

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const HANDLE: u64 = 9;

    /// A quad over the whole target showing a texture once: top left, bottom left, bottom right,
    /// top right, as `textured_quad` takes its corners.
    fn full_quad(overlay: u32, handle: u64) -> Vec<Vertex> {
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
            pos: [x, y, 0.5],
            uv: [u, v],
            layer: overlay,
            color: 0xFFFF_FFFF,
            light: 0xF0F0,
            normal: [0; 4],
            glint_uv: dynamic_texture_bits(handle),
        };
        vec![
            corner(-1.0, 1.0, 0.0, 0.0),
            corner(-1.0, -1.0, 0.0, 1.0),
            corner(1.0, -1.0, 1.0, 1.0),
            corner(1.0, 1.0, 1.0, 0.0),
        ]
    }

    fn texels(texels: [[u8; 4]; 4]) -> Vec<u8> {
        texels.concat()
    }

    fn create(rgba: Vec<u8>) -> Command {
        Command::Create {
            handle: HANDLE,
            size: [2, 2],
            mip_levels: 1,
            rgba,
        }
    }

    /// One frame for each of `frames`' command lists, all drawing the same `vertices` with one
    /// store of textures between them, each read back as RGBA8 over the 64 x 64 target.
    fn render_frames(
        vertices: &[Vertex],
        frames: &[Vec<Command>],
    ) -> std::result::Result<Vec<Vec<u8>>, String> {
        let _gpu = crate::gfx::test_gpu::lock();
        let adapter = software_adapter()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .map_err(|error| format!("software Vulkan device request failed: {error}"))?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
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
        let color = texture(
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let white = texture(
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            white.as_image_copy(),
            &[255u8; (SIZE * SIZE * 4) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: None,
            },
            white.size(),
        );
        let white_view = white.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let mut globals = Globals::zeroed();
        globals.view_proj = Mat4::IDENTITY.to_cols_array_2d();
        globals.fog = [1000.0, 2000.0, 1000.0, 2000.0];
        globals.hdr_light = [0.0, 0.0, 1.0, 0.0];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entity_binding_entries(),
        });
        let mut dynamic = DynamicEntity::new(
            &device,
            bindings.clone(),
            uniform,
            (white_view.clone(), sampler.clone()),
            (white_view.clone(), sampler.clone()),
            white_view,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(entity_shader().into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ENTITY_ATTRIBUTES,
        })];
        let reuse = reuse::Reuse::default();
        let kept = reuse.on(&device);
        let pipeline = entity_pipeline_factory(
            &kept, &layout, &layout, &shader, &buffers, format, false,
        )(EntityPipelineOptions {
            label: "dynamic quads",
            cull_mode: None,
            depth_write: true,
            compare: wgpu::CompareFunction::GreaterEqual,
            bias: Default::default(),
            blend: wgpu::BlendState::ALPHA_BLENDING,
            clouds: false,
            vertex: "vs_entity",
            fragment: "fs_entity",
            write_mask: wgpu::ColorWrites::ALL,
        });
        let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quads = vertices.len() as u32 / 4;
        let indices: Vec<u32> = (0..quads)
            .flat_map(|i| [4 * i, 4 * i + 1, 4 * i + 2, 4 * i, 4 * i + 2, 4 * i + 3])
            .collect();
        let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let store = Store::default();
        let mut uploads = super::upload::Uploads::new(&device);
        let mut batch = EntityBatch::default();
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let mut out = Vec::new();
        for commands in frames {
            let mut encoder = device.create_command_encoder(&Default::default());
            store.apply(&device, &mut uploads, &mut encoder, None, commands);
            let counts = batch
                .update_within(vertices, &[], &[], &[], &[], false, usize::MAX)
                .materials;
            let [solid, cutout, translucent] = counts[crate::gfx::entity_batch::DYNAMIC_MATERIALS]
            else {
                unreachable!("three dynamic-texture materials")
            };
            // The dynamic materials are the only ones here, so they start the buffer.
            dynamic.prepare(
                &device,
                &mut uploads,
                &mut encoder,
                &store,
                &batch.combined,
                0,
            );
            {
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
                pass.set_pipeline(&pipeline);
                pass.set_vertex_buffer(0, vertex.slice(..));
                pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                for (overlay, count) in [
                    (DYNAMIC_TEXTURE_SOLID_OVERLAY, solid),
                    (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, cutout),
                    (DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, translucent),
                ] {
                    if count > 0 {
                        dynamic.draw(&mut pass, overlay);
                    }
                }
            }
            uploads.finish(&encoder);
            out.push(readback_texture(&device, &queue, &color, encoder)?);
        }
        Ok(out)
    }

    fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        frame[((y * SIZE + x) * 4) as usize..][..4]
            .try_into()
            .unwrap()
    }

    /// The four quadrants' centres: top left, top right, bottom left, bottom right.
    const QUADRANTS: [(u32, u32); 4] = [(16, 16), (48, 16), (16, 48), (48, 48)];

    /// A 2 x 2 texture of four colours, through each texture-taking render type: the texel
    /// colours come back exactly, `unlit` light and tint white leaving them as they are.
    #[test]
    #[ignore = "daily"]
    fn each_texture_taking_render_type_draws_the_four_texels_of_its_texture() {
        for overlay in [
            DYNAMIC_TEXTURE_SOLID_OVERLAY,
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
            DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
        ] {
            let frames = render_frames(
                &full_quad(overlay, HANDLE),
                &[vec![create(texels([RED, GREEN, BLUE, YELLOW]))]],
            )
            .expect("software Vulkan draw");
            let seen = QUADRANTS.map(|(x, y)| pixel(&frames[0], x, y));
            assert_eq!(seen, [RED, GREEN, BLUE, YELLOW], "overlay {overlay}");
        }
    }

    /// Cutout discards a texel of alpha 0, so the background shows; translucent blends a texel of
    /// alpha 128 over it (red at 128 over opaque black is 128).
    #[test]
    #[ignore = "daily"]
    fn cutout_discards_clear_texels_and_translucent_blends_over_the_background() {
        let clear = [255, 255, 255, 0];
        let cutout = render_frames(
            &full_quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, HANDLE),
            &[vec![create(texels([clear, GREEN, BLUE, YELLOW]))]],
        )
        .expect("software Vulkan draw");
        assert_eq!(pixel(&cutout[0], 16, 16), BLACK);
        assert_eq!(pixel(&cutout[0], 48, 16), GREEN);
        let half = [255, 0, 0, 128];
        let translucent = render_frames(
            &full_quad(DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, HANDLE),
            &[vec![create(texels([half, GREEN, BLUE, YELLOW]))]],
        )
        .expect("software Vulkan draw");
        assert_eq!(pixel(&translucent[0], 16, 16), [128, 0, 0, 255]);
    }

    /// A region written between two frames shows in the second with the same geometry; the first
    /// is as it was made.
    #[test]
    #[ignore = "daily"]
    fn a_region_update_shows_on_the_next_frame_with_no_new_geometry() {
        let frames = render_frames(
            &full_quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, HANDLE),
            &[
                vec![create(texels([RED, GREEN, BLUE, YELLOW]))],
                vec![Command::Write {
                    handle: HANDLE,
                    mip_level: 0,
                    origin: [1, 1],
                    size: [1, 1],
                    rgba: WHITE.to_vec(),
                }],
            ],
        )
        .expect("software Vulkan draw");
        assert_eq!(pixel(&frames[0], 48, 48), YELLOW);
        assert_eq!(pixel(&frames[1], 48, 48), WHITE);
        assert_eq!(pixel(&frames[1], 16, 16), RED);
    }

    /// Under a shader pack, a `textured_quad` is drawn by the pack's entity program with the run's
    /// texture bound as the entity texture: the pack's sampler reads the texture's colour back.
    #[test]
    #[ignore = "daily"]
    fn a_shader_pack_draws_a_quad_with_its_texture_through_its_entity_program() {
        const COLOUR: [u8; 4] = [10, 200, 30, 255];
        let spans = [pack_draw::EntitySpan {
            start: 0,
            id: 12,
            stage: sunrise_shaders::render_stage("ENTITIES"),
            ..Default::default()
        }];
        let frame = render_pack_with(
            &full_quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 1),
            &spans,
            Some(COLOUR),
        )
        .expect("software Vulkan pack");
        let at = |x: u32, y: u32| frame[(y * SIZE + x) as usize];
        assert_eq!(at(32, 32), COLOUR);
        assert_eq!(at(20, 44), COLOUR);
        assert_eq!(
            at(4, 4),
            [0, 0, 0, 0],
            "outside the quad the cleared target is untouched"
        );
    }

    /// A texture released after its quad was submitted draws vanilla's missing texture
    /// (`generateMissingImage`: black top left, magenta top right).
    #[test]
    #[ignore = "daily"]
    fn a_texture_released_after_submit_draws_the_missing_texture() {
        let frames = render_frames(
            &full_quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, HANDLE),
            &[
                vec![create(texels([RED, GREEN, BLUE, YELLOW]))],
                vec![Command::Release { handle: HANDLE }],
            ],
        )
        .expect("software Vulkan draw");
        assert_eq!(pixel(&frames[0], 16, 16), RED);
        assert_eq!(pixel(&frames[1], 16, 16), BLACK);
        assert_eq!(pixel(&frames[1], 48, 16), [248, 0, 248, 255]);
    }

    /// A sign line's inline dynamic texture, laid out by the sign text path (`push_text`), reads
    /// back as the texture's four texels where the glyph is, with no sign geometry changing when
    /// a region is written; released after submit it is vanilla's missing texture.
    #[test]
    #[ignore = "daily"]
    fn a_sign_lines_inline_dynamic_texture_draws_where_the_glyph_is() {
        let call = crate::block_entity_render::TextCall {
            // One font pixel is 1/8 of the target's half width; y runs down as the font's does.
            matrix: Mat4::from_scale(glam::Vec3::new(0.125, -0.125, 1.0))
                * Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.5)),
            x: 0.0,
            y: 0.0,
            text: "\u{fffc}".into(),
            inline_sprite: None,
            inline_dynamic: Some(HANDLE),
            style: Default::default(),
            color: 0xFFFF_FFFF,
            outline: 0,
            light: 0xF0F0,
            tier: 0,
        };
        let mut vertices = Vec::new();
        crate::block_entity_render::push_text(
            &|_| Default::default(),
            None,
            &call.run(),
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        let frames = render_frames(
            &vertices,
            &[
                vec![create(texels([RED, GREEN, BLUE, YELLOW]))],
                vec![Command::Write {
                    handle: HANDLE,
                    mip_level: 0,
                    origin: [1, 1],
                    size: [1, 1],
                    rgba: WHITE.to_vec(),
                }],
                vec![Command::Release { handle: HANDLE }],
            ],
        )
        .expect("software Vulkan draw");
        // The glyph spans x 0..8 and y -1..7 font pixels: pixels 32..64 across, 28..60 down.
        let glyph = [(40, 36), (56, 36), (40, 52), (56, 52)];
        assert_eq!(
            glyph.map(|(x, y)| pixel(&frames[0], x, y)),
            [RED, GREEN, BLUE, YELLOW]
        );
        assert_eq!(pixel(&frames[0], 16, 16), BLACK);
        assert_eq!(pixel(&frames[1], 56, 52), WHITE);
        assert_eq!(pixel(&frames[1], 40, 36), RED);
        assert_eq!(pixel(&frames[2], 40, 36), BLACK);
        assert_eq!(pixel(&frames[2], 56, 36), [248, 0, 248, 255]);
    }
}
