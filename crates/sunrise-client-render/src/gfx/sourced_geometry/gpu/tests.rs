use super::*;
use crate::gfx::{
    Renderer, TERRAIN_ATTRIBUTES, frustum,
    gpu_culling::tests::{on_each_device, read},
    lod, world_projection,
};
use crate::mesher::{Sides, TerrainVertex};
use glam::DVec3;

/// Each of `module`'s entry points written by every backend's writer.
fn translates_everywhere(module: &naga::Module, info: &naga::valid::ModuleInfo) {
    for entry in &module.entry_points {
        let (stage, name) = (entry.stage, entry.name.clone());
        let spv = naga::back::spv::PipelineOptions {
            shader_stage: stage,
            entry_point: name.clone(),
        };
        naga::back::spv::write_vec(module, info, &Default::default(), Some(&spv))
            .unwrap_or_else(|e| panic!("{name} (SPIR-V): {e}"));
        let pipeline = naga::back::hlsl::PipelineOptions {
            entry_point: Some((stage, name.clone())),
        };
        let mut hlsl = String::new();
        naga::back::hlsl::Writer::new(&mut hlsl, &Default::default(), &pipeline)
            .write(module, info, None)
            .unwrap_or_else(|e| panic!("{name} (HLSL): {e}"));
        let options = naga::back::msl::Options {
            fake_missing_bindings: true,
            ..Default::default()
        };
        let pipeline = naga::back::msl::PipelineOptions {
            entry_point: Some((stage, name.clone())),
            ..Default::default()
        };
        naga::back::msl::write_string(module, info, &options, &pipeline)
            .unwrap_or_else(|e| panic!("{name} (MSL): {e}"));
        let options = naga::back::glsl::Options {
            version: naga::back::glsl::Version::Embedded {
                version: 310,
                is_webgl: false,
            },
            ..Default::default()
        };
        let pipeline = naga::back::glsl::PipelineOptions {
            shader_stage: stage,
            entry_point: name.clone(),
            multiview: None,
        };
        let mut glsl = String::new();
        naga::back::glsl::Writer::new(
            &mut glsl,
            module,
            info,
            &options,
            &pipeline,
            Default::default(),
        )
        .and_then(|mut w| w.write())
        .unwrap_or_else(|e| panic!("{name} (GLSL): {e}"));
    }
}

/// The pass's shader, which sourced meshes run through (second table, `NO_RANGE`), needs no
/// capability and is written by every backend.
#[test]
fn the_shader_needs_no_capability_and_translates_everywhere() {
    let _gpu = crate::gfx::test_gpu::lock();
    let module = naga::front::wgsl::parse_str(include_str!("../../gpu_culling.wgsl")).unwrap();
    let flags = naga::valid::ValidationFlags::all();
    let info = naga::valid::Validator::new(flags, naga::valid::Capabilities::empty())
        .validate(&module)
        .unwrap();
    translates_everywhere(&module, &info);
}

/// A cell's key is its corner in sections, and its box step a sixteenth of its width; narrower
/// than a section, wider than the units hold or a corner past an i32 is not taken.
#[test]
fn cells_key_on_their_corner_in_sections() {
    let cell = |x, z, size_log2| Cell { x, z, size_log2 };
    assert_eq!(cell(3, -2, 9).key(), Some((96, 0, -64)));
    assert_eq!(cell(3, -2, 9).shift(), 5);
    assert_eq!(cell(-1, 1, 4).key(), Some((-1, 0, 1)));
    assert_eq!(cell(0, 0, 3).key(), None);
    assert_eq!(cell(0, 0, MAX_SIZE_LOG2 + 1).key(), None);
    assert_eq!(cell(1 << 20, 0, 19).key(), None);
}

#[test]
fn records_refuse_cells_narrower_than_a_section() {
    let mesh = Mesh {
        cell: Cell {
            x: 0,
            z: 0,
            size_log2: 3,
        },
        page: 0,
        start: 0,
        quads: 1,
        sides: &Sides::unsorted(1),
        bounds: [Vec3::ZERO, Vec3::ONE],
    };
    assert!(SourcedGpu::record(&mesh, false).is_none());
}

/// A mesh's box is its bounds rounded outward in the cell's units.
#[test]
fn boxes_round_outward_in_the_cells_units() {
    let cell = Cell {
        x: 0,
        z: 0,
        size_log2: 9,
    };
    // 32-block steps along x and z, 16 along y: 33 lies in the second, so the box ends at 64.
    let bounds = [Vec3::new(1.0, 17.0, 31.0), Vec3::new(33.0, 20.0, 40.0)];
    let wider = [Vec3::new(0.0, 16.0, 0.0), Vec3::new(64.0, 32.0, 64.0)];
    assert_eq!(cell_box(bounds, cell), cell_box(wider, cell));
}

/// A tile's colour from its vertices, white towards the fade's end, with the vertices' alpha
/// (the blended pipeline mixes water over what is behind it), drawn by a view and projection in
/// a uniform and depth tested, as the terrain pipelines draw.
const TILES: &str = "
@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
struct Tile {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}
@vertex fn vs(
    @location(0) position: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(5) instance: vec4<f32>,
) -> Tile {
    let fade = clamp(-instance.w / 16.0, 0.0, 1.0);
    return Tile(
        view_proj * vec4<f32>(position + instance.xyz, 1.0),
        vec4<f32>(mix(color.rgb, vec3<f32>(1.0), fade), color.a),
    );
}
@fragment fn fs(tile: Tile) -> @location(0) vec4<f32> {
    return tile.color;
}
";

const SIZE_LOG2: u8 = 9;
const SIDE: u32 = 128;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How a tile stands in a frame.
#[derive(Clone, Copy, PartialEq)]
enum State {
    Steady,
    /// Fading in, at this `lod::instance` code.
    Fading(u32),
    /// Its replacement is in, fading: this one stands in at its place, whole.
    StandIn,
}

struct Tile {
    cell: Cell,
    stand: State,
    /// The mesh the tile has: where its quad is in the arena, and the one a stand-in draws.
    mesh: (u32, u64),
    previous: Option<(u32, u64)>,
    origin: [f32; 3],
    id: Option<u32>,
}

impl Tile {
    /// What the CPU draws: the stand-in while there is one.
    fn drawn(&self) -> (u32, u64) {
        self.previous.unwrap_or(self.mesh)
    }
    fn instance(&self) -> [f32; 4] {
        match self.stand {
            State::Fading(code) => lod::instance(self.origin, code),
            _ => lod::instance(self.origin, 0),
        }
    }
}

fn quad(height: f32, color: u32) -> [TerrainVertex; 4] {
    let side = (1u32 << SIZE_LOG2) as f32;
    [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)].map(|(x, z)| TerrainVertex {
        pos: [x, height, z],
        color,
        ..bytemuck::Zeroable::zeroed()
    })
}

/// The tiles' draw rig: the `TILES` pipeline, whole and with alpha blending, its uniform,
/// its target, and the quad indices (`Renderer::quad_indices`).
struct Rig {
    pipeline: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    target: wgpu::Texture,
    extent: wgpu::Extent3d,
    pixels: wgpu::Buffer,
    indices: wgpu::Buffer,
}

/// The view and projection the rig's uniform carries, for the frames' frustum.
fn rig_view() -> glam::Mat4 {
    world_projection(100.0, 1.0, 4096.0) * crate::camera::view_matrix(0.0, 90.0)
}

/// The pass's picture: the target copied out and read back, the uploads with it.
fn copy_and_submit(
    device: &wgpu::Device,
    queue: &Queue,
    mut encoder: wgpu::CommandEncoder,
    rig: &Rig,
    uploads: &mut Uploads,
) -> Vec<u32> {
    encoder.copy_texture_to_buffer(
        rig.target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &rig.pixels,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: None,
            },
        },
        rig.extent,
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    read(device, queue, &rig.pixels, rig.pixels.size())
}

fn rig(device: &wgpu::Device, queue: &Queue, stride: u64) -> Rig {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(TILES.into()),
    });
    // One layout for both pipelines: auto layouts are one pipeline's alone.
    let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: Some(wgpu::BufferSize::new(64).unwrap()),
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&group_layout)],
        immediate_size: 0,
    });
    let instance = wgpu::vertex_attr_array![5 => Float32x4];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TERRAIN_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance,
        }),
    ];
    let whole = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: None,
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: None,
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let blended = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: None,
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: None,
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let lens = world_projection(100.0, 1.0, 4096.0);
    let view_proj = lens * crate::camera::view_matrix(0.0, 90.0);
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &uniform,
        0,
        bytemuck::cast_slice(&view_proj.to_cols_array()),
    );
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let extent = wgpu::Extent3d {
        width: SIDE,
        height: SIDE,
        depth_or_array_layers: 1,
    };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = texture(
        COLOR,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let pixels = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIDE * SIDE) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    Rig {
        blended,
        group,
        color_view: target.create_view(&Default::default()),
        depth_view: depth.create_view(&Default::default()),
        target,
        extent,
        pixels,
        indices: Renderer::quad_indices(device, 1),
        pipeline: whole,
    }
}

/// A render pass over the rig's target, cleared black, the tiles' pipeline bound.
fn cleared_pass<'a>(rig: &'a Rig, encoder: &'a mut wgpu::CommandEncoder) -> wgpu::RenderPass<'a> {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &rig.color_view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &rig.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    pass.set_pipeline(&rig.pipeline);
    pass.set_bind_group(0, &rig.group, &[]);
    pass.set_index_buffer(rig.indices.slice(..), wgpu::IndexFormat::Uint32);
    pass
}

/// The same frame drawn by the CPU alone (a draw per tile) and with the GPU path (steady tiles
/// through `SourcedGpu::draw`, after the CPU draws of the fading and standing-in ones) is the
/// same image, with a fading mesh and a stand-in among the tiles; the GPU frame without the
/// CPU's draws is not.
#[test]
#[ignore = "daily"]
fn the_gpu_path_draws_the_cpu_paths_image() {
    let _gpu = crate::gfx::test_gpu::lock();
    on_each_device(images_match);
}

fn images_match(device: &wgpu::Device, queue: &Queue) {
    let stride = std::mem::size_of::<TerrainVertex>() as u64;
    let grain = crate::gfx::arena::GRAIN;
    let mut arena = Arena::new(device, stride, grain, false);
    assert!(arena.open_page(device, 256 * grain));
    let camera = DVec3::new(100.5, 3000.25, -37.75);
    let side = f64::from(1u32 << SIZE_LOG2);
    // A 7 by 7 square of cells around the camera's, the sky over them: each tile one quad at
    // a height of its own, in a colour of its own.
    let (mut tiles, mut slot) = (Vec::new(), 0u64);
    let mut put = |height: f32, color: u32| {
        let start = slot * grain;
        slot += 1;
        queue.write_buffer(
            arena.buffer(0),
            start * stride,
            bytemuck::cast_slice(&quad(height, color)),
        );
        (0, start)
    };
    for (i, (x, z)) in (-3..=3)
        .flat_map(|x| (-3..=3).map(move |z| (x, z)))
        .enumerate()
    {
        let cell = Cell {
            x,
            z,
            size_log2: SIZE_LOG2,
        };
        let color = 0xFF00_0000 | ((i as u32 * 0x0005_1B27) & 0x00FF_FFFF);
        let mesh = put(40.0 + (i % 7) as f32 * 3.0, color);
        let (stand, previous) = match i {
            // Fading in, every few steps.
            9 | 24 | 38 => (State::Fading(1 + i as u32 % 15), None),
            // Standing in for a replacement, drawn at another height.
            12 | 30 => (State::StandIn, Some(put(80.0, color ^ 0x00FF_FF00))),
            _ => (State::Steady, None),
        };
        tiles.push(Tile {
            cell,
            stand,
            mesh,
            previous,
            origin: [
                (f64::from(x) * side - camera.x) as f32,
                (0.0 - camera.y) as f32,
                (f64::from(z) * side - camera.z) as f32,
            ],
            id: None,
        });
    }
    let sides = Sides::unsorted(1);
    let bounds = [
        Vec3::new(0.0, 30.0, 0.0),
        Vec3::new(side as f32, 90.0, side as f32),
    ];
    let step = 1.0 / crate::mesher::TILED_UV_STEPS;
    let near = GpuCulling::new(device, None).expect("the pass");
    let mut sourced = SourcedGpu::new(&near, device, step).expect("the table");
    for tile in &mut tiles {
        let mesh = Mesh {
            cell: tile.cell,
            page: tile.mesh.0,
            start: tile.mesh.1,
            quads: 1,
            sides: &sides,
            bounds,
        };
        tile.id = sourced.add(&mesh, tile.stand != State::Steady);
        assert!(tile.id.is_some());
    }

    let rig = rig(device, queue, stride);
    let rows: Vec<[f32; 4]> = tiles.iter().map(Tile::instance).collect();
    let rows_buffer = {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&rows),
            usage: wgpu::BufferUsages::VERTEX,
        })
    };
    let all = Frustum::from_planes(frustum(rig_view()));
    let mut uploads = Uploads::new(device);

    // `gpu`: whether sourced geometry draws the steady tiles; `cpu_lists`: whether the CPU
    // draws the rest. Neither: the CPU draws every tile.
    let mut image = |gpu: bool, cpu_lists: bool| {
        let mut encoder = device.create_command_encoder(&Default::default());
        if gpu {
            let prepared = sourced.prepare(
                device,
                queue,
                &mut uploads,
                &mut encoder,
                camera,
                -64,
                &all,
                None,
                None,
            );
            assert_eq!(prepared, Prepared::Gpu);
            sourced.dispatch(&mut encoder, None);
        }
        let mut pass = cleared_pass(&rig, &mut encoder);
        pass.set_vertex_buffer(1, rows_buffer.slice(..));
        for (i, tile) in tiles.iter().enumerate() {
            if gpu && (tile.stand == State::Steady || !cpu_lists) {
                continue;
            }
            let (page, start) = tile.drawn();
            pass.set_vertex_buffer(0, arena.buffer(page).slice(..));
            pass.draw_indexed(0..6, start as i32, i as u32..i as u32 + 1);
        }
        if gpu {
            sourced.draw(&mut pass, &arena, |_| None);
        }
        drop(pass);
        copy_and_submit(device, queue, encoder, &rig, &mut uploads)
    };
    let cpu = image(false, true);
    let gpu = image(true, true);
    let black = |image: &[u32]| image.iter().filter(|&&p| p == 0xFF00_0000).count();
    let (covered, total) = (cpu.len() - black(&cpu), cpu.len());
    assert!(covered > total / 4, "the tiles cover {covered} of {total}");
    let colors: std::collections::BTreeSet<_> = cpu.iter().collect();
    assert!(colors.len() > 20, "{} colours", colors.len());
    let wrong = cpu.iter().zip(&gpu).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels of {total} differ");
    // Without the CPU's draws the fading and standing-in tiles are missing.
    let without = image(true, false);
    assert_ne!(cpu, without, "the flagged tiles are not drawn by the GPU");
    assert!(sourced.draw_calls() > 0);
}

/// The residency, driven as `Renderer` drives it, draws the CPU's image: tiles come in
/// (`Sourced::record`), fade in over `lod::FADE` with their flags and places in the CPU's
/// list kept by `take_due` and `refresh`, a stand-in draws in a fading tile's place and is
/// meshed again (`unrecord`, `record`), and one tile carries translucent quads whose solid
/// quads the pass draws whole while the CPU draws the water. The frame's split is the
/// walk's (`walk_tiles`): the complete set drives the water, what the pass left the CPU
/// drives its draws and instances — none of it hand-assigned.
#[test]
#[ignore = "daily"]
fn the_residency_draws_the_cpu_paths_image() {
    let _gpu = crate::gfx::test_gpu::lock();
    on_each_device(residency_images_match);
}

fn residency_images_match(device: &wgpu::Device, queue: &Queue) {
    use crate::fast_hash::FxHashMap;
    use crate::gfx::SectionGpu;
    use std::time::Duration;

    let stride = std::mem::size_of::<TerrainVertex>() as u64;
    let grain = crate::gfx::arena::GRAIN;
    let mut arena = Arena::new(device, stride, grain, false);
    assert!(arena.open_page(device, 256 * grain));
    let camera = DVec3::new(100.5, 3000.25, -37.75);
    let side = f64::from(1u32 << SIZE_LOG2);
    let mut slot = 0u64;
    let mut quad_at = |queue: &Queue, height: f32, color: u32| {
        let at = slot * grain;
        slot += 1;
        queue.write_buffer(
            arena.buffer(0),
            at * stride,
            bytemuck::cast_slice(&quad(height, color)),
        );
        at as u32
    };
    let sides = Sides::unsorted(1);
    // A mesh of one quad in the arena, or of a solid quad with a translucent one.
    let mut mesh = |queue: &Queue, height: f32, color: u32, water: Option<(f32, u32)>| {
        let start = quad_at(queue, height, color);
        let water = water.map(|(height, color)| quad_at(queue, height, color));
        SectionGpu {
            page: 0,
            start: u64::from(start),
            translucent_at: (0, u64::from(water.unwrap_or(start)) * 4),
            solid: 1,
            solid_slots: 4,
            packed_far: false,
            packed: false,
            sides,
            translucent: u32::from(water.is_some()),
            sort: crate::gfx::translucency::SortState::new(Vec::new()),
            sorted_indices: None,
            flat_id: None,
            gpu_id: None,
            gpu_fading: false,
            fading: None,
            leaving: None,
            animated_layers: Box::new([]),
        }
    };
    let key = |x: i32, z: i32| crate::lod::TileKey {
        x,
        z,
        level: SIZE_LOG2 - 4,
    };
    let bounds = [
        Vec3::new(0.0, 30.0, 0.0),
        Vec3::new(side as f32, 90.0, side as f32),
    ];
    let near = GpuCulling::new(device, None).expect("the pass");
    let mut sourced = Sourced::default();
    let mut tiles: FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile> = Default::default();
    let mut grid = crate::gfx::lod::TileGrid::default();
    // Time starts a second out: the tiles' fades and their due ends are handed the
    // frames' instants, none waited for.
    let t0 = Instant::now() + Duration::from_secs(1);
    let whole = t0 - lod::FADE;
    // Whole: a tile the pass draws, with its quads in the arena and its record made.
    let mut steady =
        |queue: &Queue, x: i32, z: i32, height: f32, color: u32, water: Option<(f32, u32)>| {
            let key = key(x, z);
            tiles.insert(
                key,
                crate::gfx::lod::Tile::for_test(
                    mesh(queue, height, color, water),
                    bounds,
                    Some(whole),
                    None,
                ),
            );
            grid.insert_bounds(key, bounds);
            assert!(!sourced.record(key, tiles.get_mut(&key).unwrap(), Some(&near), device, t0));
        };
    steady(queue, 0, 0, 40.0, 0xFF30_5070, None);
    steady(queue, 2, 1, 34.0, 0xFF60_2070, None);
    // The tile with both solid and translucent quads: its solid quads are the pass's,
    // its water the CPU's, in every frame the pass draws.
    steady(queue, -1, 0, 46.0, 0xFF20_8040, Some((60.0, 0x8000_00FF)));
    // Fading in since `t0`: the pass leaves it to the CPU until its fade-in is due,
    // then draws it (`take_due`, `refresh`).
    let fading = key(1, 0);
    tiles.insert(
        fading,
        crate::gfx::lod::Tile::for_test(
            mesh(queue, 52.0, 0xFF90_1050, None),
            bounds,
            Some(t0),
            None,
        ),
    );
    grid.insert_bounds(fading, bounds);
    assert!(!sourced.record(
        fading,
        tiles.get_mut(&fading).unwrap(),
        Some(&near),
        device,
        t0
    ));
    // Standing in for its replacement: its previous mesh draws in its place.
    let stand_in = key(0, 1);
    tiles.insert(
        stand_in,
        crate::gfx::lod::Tile::for_test(
            mesh(queue, 44.0, 0xFF50_6070, None),
            bounds,
            Some(t0),
            Some((mesh(queue, 80.0, 0xFF10_A0A0, None), Some(t0))),
        ),
    );
    grid.insert_bounds(stand_in, bounds);
    assert!(!sourced.record(
        stand_in,
        tiles.get_mut(&stand_in).unwrap(),
        Some(&near),
        device,
        t0
    ));
    assert!(!sourced.off());

    let rig = rig(device, queue, stride);
    let view = Frustum::from_planes(frustum(rig_view()));
    let mut uploads = Uploads::new(device);
    use wgpu::util::DeviceExt;

    // One frame at `now`: the walk's complete set, the CPU's draws and instances of what
    // the pass left it (`pass`: the pass draws the rest, whole), and the water of every
    // tile with translucent quads, blended, back to front.
    let mut frame = |sourced: &mut Sourced,
                     tiles: &FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile>,
                     now: Instant,
                     pass: bool|
     -> Vec<u32> {
        sourced.take_due(tiles, now);
        let mut all = Vec::new();
        let mut list = |v| all.push(v);
        walk_tiles(&grid, tiles, camera, &view, now, None, &mut list);
        all.sort_unstable_by_key(|v| (v.distance, v.key));
        let listed: Vec<_> = if pass {
            all.iter()
                .filter(|v| sourced.lists(v.key))
                .cloned()
                .collect()
        } else {
            all.clone()
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        if pass {
            let prepared = sourced.prepare(
                device,
                queue,
                &mut uploads,
                &mut encoder,
                camera,
                -64,
                &view,
                None,
                None,
            );
            assert_eq!(prepared, Prepared::Gpu);
            sourced.dispatch(&mut encoder, None);
        }
        let mut world = cleared_pass(&rig, &mut encoder);
        // The CPU's solid draws and their instances: what the pass left it (all of it,
        // when it draws nothing), at the fades the walk gives them.
        let cpu_solid = |v: &&crate::gfx::lod::Visible| !pass || !sourced.draws_solid(v.key);
        let solids: Vec<[f32; 4]> = listed
            .iter()
            .filter(cpu_solid)
            .map(|v| lod::instance(v.origin, v.fade))
            .collect();
        let instances = |rows: &[[f32; 4]]| {
            (!rows.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(rows),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        if let Some(buffer) = instances(&solids) {
            world.set_vertex_buffer(1, buffer.slice(..));
        }
        for (base, v) in listed.iter().filter(cpu_solid).enumerate() {
            let s = tiles[&v.key].drawn();
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            world.draw_indexed(0..6, s.solid_base() as i32, base as u32..base as u32 + 1);
        }
        if pass {
            sourced.draw(&mut world, &arena, |_| None);
        }
        // The water, over the solid quads: every tile with translucent quads, whatever
        // draws its solid ones.
        world.set_pipeline(&rig.blended);
        let water: Vec<[f32; 4]> = all
            .iter()
            .filter(|v| tiles[&v.key].drawn().translucent > 0)
            .map(|v| lod::instance(v.origin, v.fade))
            .collect();
        if let Some(buffer) = instances(&water) {
            world.set_vertex_buffer(1, buffer.slice(..));
        }
        for (base, v) in all
            .iter()
            .filter(|v| tiles[&v.key].drawn().translucent > 0)
            .enumerate()
        {
            let s = tiles[&v.key].drawn();
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            world.draw_indexed(
                0..6,
                s.translucent_at.1 as i32,
                base as u32..base as u32 + 1,
            );
        }
        drop(world);
        copy_and_submit(device, queue, encoder, &rig, &mut uploads)
    };

    let frame_ms = Duration::from_millis(16);
    // While B fades in and D stands in: their solid quads are the CPU's, at the fades
    // the walk gives them.
    let at = t0 + frame_ms;
    let cpu = frame(&mut sourced, &tiles, at, false);
    let split = frame(&mut sourced, &tiles, at, true);
    let black = |image: &[u32]| image.iter().filter(|&&p| p == 0xFF00_0000).count();
    let (covered, total) = (cpu.len() - black(&cpu), cpu.len());
    assert!(covered > 250, "the tiles cover {covered} of {total}");
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels of {total} differ as they fade");
    assert!(sourced.lists(fading) && sourced.lists(stand_in));
    assert!(sourced.lists(key(-1, 0)) && sourced.draws_solid(key(-1, 0)));
    assert!(!sourced.lists(key(0, 0)) && sourced.draws_solid(key(0, 0)));
    assert!(sourced.draw_calls() > 0);

    // The fade-in is due: the pass draws B whole, the CPU leaves it; D's replacement
    // came in (`upload_lod`'s take and add), its stand-in gone.
    let at = t0 + lod::FADE + frame_ms;
    tiles.remove(&stand_in).expect("the standing tile");
    sourced.unrecord(stand_in);
    tiles.insert(
        stand_in,
        crate::gfx::lod::Tile::for_test(
            mesh(queue, 44.0, 0xFF50_6070, None),
            bounds,
            Some(t0),
            None,
        ),
    );
    assert!(!sourced.record(
        stand_in,
        tiles.get_mut(&stand_in).unwrap(),
        Some(&near),
        device,
        at
    ));
    let cpu = frame(&mut sourced, &tiles, at, false);
    let split = frame(&mut sourced, &tiles, at, true);
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels of {total} differ once whole");
    // The flags followed the fades: B and D the pass's, C's water still the CPU's.
    assert!(!sourced.lists(fading) && sourced.draws_solid(fading));
    assert!(!sourced.lists(stand_in) && sourced.draws_solid(stand_in));
    assert!(sourced.lists(key(-1, 0)) && sourced.draws_solid(key(-1, 0)));
    // The residency's own invariant, as the debug renderer walks it each 64th frame.
    #[cfg(debug_assertions)]
    sourced.check(&tiles, at, true);
}

/// GPU culling turned on while only water-bearing tiles are resident lists them the CPU's
/// though no table is made yet, and the table the first solid tile makes later leaves them
/// their places: the CPU keeps drawing their water (`#4769`'s first defect had the enabling
/// wipe the list, and the water with it).
#[test]
#[ignore = "daily"]
fn the_water_keeps_its_place_when_the_table_comes_late() {
    let _gpu = crate::gfx::test_gpu::lock();
    on_each_device(water_keeps_its_place_when_the_table_comes_late);
}

fn water_keeps_its_place_when_the_table_comes_late(device: &wgpu::Device, queue: &Queue) {
    use crate::fast_hash::FxHashMap;
    use crate::gfx::SectionGpu;
    use std::time::Duration;

    let stride = std::mem::size_of::<TerrainVertex>() as u64;
    let grain = crate::gfx::arena::GRAIN;
    let mut arena = Arena::new(device, stride, grain, false);
    assert!(arena.open_page(device, 256 * grain));
    let camera = DVec3::new(100.5, 3000.25, -37.75);
    let side = f64::from(1u32 << SIZE_LOG2);
    let mut slot = 0u64;
    let mut quad_at = |queue: &Queue, height: f32, color: u32| {
        let at = slot * grain;
        slot += 1;
        queue.write_buffer(
            arena.buffer(0),
            at * stride,
            bytemuck::cast_slice(&quad(height, color)),
        );
        at as u32
    };
    // A mesh of quads in the arena: none with solid quads, or one solid quad and a
    // translucent one over it.
    let mut mesh = |queue: &Queue, water: Option<(f32, u32)>, solid: Option<(f32, u32)>| {
        let sides = Sides::unsorted(u32::from(solid.is_some()));
        let water = water.map(|(height, color)| quad_at(queue, height, color));
        let solid = solid.map(|(height, color)| quad_at(queue, height, color));
        SectionGpu {
            page: 0,
            start: u64::from(solid.unwrap_or(0)),
            translucent_at: (0, u64::from(water.unwrap_or(0))),
            solid: u32::from(solid.is_some()),
            solid_slots: u64::from(solid.is_some()) * 4,
            packed_far: false,
            packed: false,
            sides,
            translucent: u32::from(water.is_some()),
            sort: crate::gfx::translucency::SortState::new(Vec::new()),
            sorted_indices: None,
            flat_id: None,
            gpu_id: None,
            gpu_fading: false,
            fading: None,
            leaving: None,
            animated_layers: Box::new([]),
        }
    };
    let key = |x: i32, z: i32| crate::lod::TileKey {
        x,
        z,
        level: SIZE_LOG2 - 4,
    };
    let bounds = [
        Vec3::new(0.0, 30.0, 0.0),
        Vec3::new(side as f32, 90.0, side as f32),
    ];
    let near = GpuCulling::new(device, None).expect("the pass");
    let mut sourced = Sourced::default();
    let mut tiles: FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile> = Default::default();
    let mut grid = crate::gfx::lod::TileGrid::default();
    let t0 = Instant::now() + Duration::from_secs(1);
    let whole = t0 - lod::FADE;
    // GPU culling comes on with one water-only tile resident: no table, but the tile keeps
    // its place in the CPU's list.
    let water_tile = key(0, 0);
    tiles.insert(
        water_tile,
        crate::gfx::lod::Tile::for_test(
            mesh(queue, Some((60.0, 0x8000_00FF)), None),
            bounds,
            Some(whole),
            None,
        ),
    );
    grid.insert_bounds(water_tile, bounds);
    assert!(!sourced.record_all(&mut tiles, Some(&near), device));
    assert!(sourced.off());
    assert!(sourced.lists(water_tile));
    #[cfg(debug_assertions)]
    sourced.check(&tiles, t0, true);
    // The first tile with solid quads makes the table; the water tile's place survives it.
    let solid_tile = key(1, 0);
    tiles.insert(
        solid_tile,
        crate::gfx::lod::Tile::for_test(
            mesh(queue, None, Some((40.0, 0xFF30_5070))),
            bounds,
            Some(whole),
            None,
        ),
    );
    grid.insert_bounds(solid_tile, bounds);
    assert!(!sourced.record(
        solid_tile,
        tiles.get_mut(&solid_tile).unwrap(),
        Some(&near),
        device,
        t0
    ));
    assert!(!sourced.off());
    assert!(sourced.lists(water_tile));
    assert!(!sourced.lists(solid_tile));

    let rig = rig(device, queue, stride);
    let view = Frustum::from_planes(frustum(rig_view()));
    let mut uploads = Uploads::new(device);

    // One frame at `now`: the walk's complete set, the CPU's draws and instances of what
    // the pass left it (`pass`: the pass draws the rest), and the water of the listed
    // tiles, at the slots `FarWater` pairs them (`draw_frame`'s far water).
    let mut frame = |sourced: &mut Sourced,
                     tiles: &FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile>,
                     now: Instant,
                     pass: bool|
     -> Vec<u32> {
        sourced.take_due(tiles, now);
        let mut all = Vec::new();
        let mut list = |v| all.push(v);
        walk_tiles(&grid, tiles, camera, &view, now, None, &mut list);
        all.sort_unstable_by_key(|v| (v.distance, v.key));
        let listed: Vec<_> = if pass {
            all.iter()
                .filter(|v| sourced.lists(v.key))
                .cloned()
                .collect()
        } else {
            all.clone()
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        if pass {
            let prepared = sourced.prepare(
                device,
                queue,
                &mut uploads,
                &mut encoder,
                camera,
                0,
                &view,
                None,
                None,
            );
            assert_eq!(prepared, Prepared::Gpu);
            sourced.dispatch(&mut encoder, None);
        }
        let mut world = cleared_pass(&rig, &mut encoder);
        let cpu_solid = |v: &&crate::gfx::lod::Visible| {
            tiles[&v.key].drawn().solid > 0 && (!pass || !sourced.draws_solid(v.key))
        };
        let solids: Vec<[f32; 4]> = listed
            .iter()
            .filter(cpu_solid)
            .map(|v| lod::instance(v.origin, 0))
            .collect();
        let instances = |rows: &[[f32; 4]]| {
            (!rows.is_empty()).then(|| {
                use wgpu::util::DeviceExt;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(rows),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        if let Some(buffer) = instances(&solids) {
            world.set_vertex_buffer(1, buffer.slice(..));
        }
        for (base, v) in listed.iter().filter(cpu_solid).enumerate() {
            let s = tiles[&v.key].drawn();
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            world.draw_indexed(0..6, s.solid_base() as i32, base as u32..base as u32 + 1);
        }
        if pass {
            sourced.draw(&mut world, &arena, |_| None);
        }
        // The water of the listed tiles, blended, at their paired slots.
        world.set_pipeline(&rig.blended);
        let waters: Vec<[f32; 4]> = listed
            .iter()
            .filter(|v| tiles[&v.key].drawn().translucent > 0)
            .map(|v| lod::instance(v.origin, v.fade))
            .collect();
        if let Some(buffer) = instances(&waters) {
            world.set_vertex_buffer(1, buffer.slice(..));
        }
        for (base, v) in listed
            .iter()
            .filter(|v| tiles[&v.key].drawn().translucent > 0)
            .enumerate()
        {
            let s = tiles[&v.key].drawn();
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            world.draw_indexed(
                0..6,
                s.translucent_at.1 as i32,
                base as u32..base as u32 + 1,
            );
        }
        drop(world);
        copy_and_submit(device, queue, encoder, &rig, &mut uploads)
    };

    let at = t0;
    let cpu = frame(&mut sourced, &tiles, at, false);
    let split = frame(&mut sourced, &tiles, at, true);
    let black = |image: &[u32]| image.iter().filter(|&&p| p == 0xFF00_0000).count();
    let (covered, total) = (cpu.len() - black(&cpu), cpu.len());
    assert!(covered > 100, "the tiles cover {covered} of {total}");
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(
        wrong, 0,
        "{wrong} pixels of {total} differ with the table new"
    );
    assert!(sourced.lists(water_tile) && !sourced.draws_solid(water_tile));
    assert!(sourced.draws_solid(solid_tile));
    #[cfg(debug_assertions)]
    sourced.check(&tiles, at, true);
}
/// A fade with the near sections' water and far water together keeps its order through the
/// split: the walk's complete set drives the re-sort's rotation (`Rotation`, a far tile's
/// place its own past the near sections'), a listed tile's instance slot names its place in
/// what the CPU draws (`FarWater::entries`), and a tile standing in for its replacement blends
/// by the tier of that same list (`FarWater::order`): the frames run `draw_frame`'s own decisions,
/// with translucent water, on both sides of a fade's handoff, and the CPU's frames and the
/// split's in worlds of their own. The far tile whose solid quads the pass drew
/// whole leaves the CPU's list, so the two lists differ, and the split still draws the CPU's
/// image. And when the camera steps a block across a tile's two water quads, the rotation
/// re-sorts that tile — its order moves with the camera, the other water tiles' stays — in
/// the CPU's frame and the split's alike (`#4769`'s second and third defects).
#[test]
#[ignore = "daily"]
fn near_and_far_water_keep_their_order_through_the_split() {
    let _gpu = crate::gfx::test_gpu::lock();
    on_each_device(near_and_far_water_keep_their_order);
}

fn near_and_far_water_keep_their_order(device: &wgpu::Device, queue: &Queue) {
    use crate::fast_hash::FxHashMap;
    use crate::gfx::SectionGpu;
    use crate::gfx::draw_frame::{FarWater, Rotation};
    use crate::gfx::translucency;
    use std::collections::BTreeSet;
    use std::time::Duration;
    use wgpu::util::DeviceExt;

    let stride = std::mem::size_of::<TerrainVertex>() as u64;
    let grain = crate::gfx::arena::GRAIN;
    let mut arena = Arena::new(device, stride, grain, false);
    assert!(arena.open_page(device, 512 * grain));
    let side = f64::from(1u32 << SIZE_LOG2);
    // The camera rests here for two frames, then steps a block along z: the step crosses
    // the two water quads' equidistant plane, and their distance order with it.
    let before = DVec3::new(100.5, 3000.25, -38.0);
    let after = DVec3::new(before.x, before.y, before.z - 1.0);
    let mut slot = 0u64;
    let mut put = |span: f32, at: [f32; 2], height: f32, color: u32| -> u32 {
        let word = slot * grain;
        slot += 1;
        let corners = [
            (at[0], at[1]),
            (at[0] + span, at[1]),
            (at[0] + span, at[1] + span),
            (at[0], at[1] + span),
        ];
        let verts: [TerrainVertex; 4] = corners.map(|(x, z)| TerrainVertex {
            pos: [x, height, z],
            color,
            ..bytemuck::Zeroable::zeroed()
        });
        queue.write_buffer(arena.buffer(0), word * stride, bytemuck::cast_slice(&verts));
        word as u32
    };
    let sides = Sides::unsorted(1);
    // A mesh of one solid quad and water quads over it, with the water's centres for the
    // sort: the shape the near sections and the far tiles share.
    let mut mesh =
        |span: f32, solid: (f32, u32), waters: &[([f32; 2], f32, f32, u32)]| -> SectionGpu {
            let solid_at = put(span, [0.0; 2], solid.0, solid.1);
            let mut first_water = None;
            let mut centers = Vec::new();
            for &(at, span, height, color) in waters {
                let water = put(span, at, height, color);
                first_water = first_water.or(Some(u64::from(water)));
                centers.push([at[0] + span / 2.0, height, at[1] + span / 2.0]);
            }
            SectionGpu {
                page: 0,
                start: u64::from(solid_at),
                translucent_at: (0, first_water.unwrap_or(0)),
                solid: 1,
                solid_slots: 4,
                packed_far: false,
                packed: false,
                sides,
                translucent: waters.len() as u32,
                sort: translucency::SortState::new(centers),
                sorted_indices: (!waters.is_empty()).then_some((0, 0)),
                flat_id: None,
                gpu_id: None,
                gpu_fading: false,
                fading: None,
                leaving: None,
                animated_layers: Box::new([]),
            }
        };
    let key = |x: i32, z: i32| crate::lod::TileKey {
        x,
        z,
        level: SIZE_LOG2 - 4,
    };

    let bounds = [
        Vec3::new(0.0, 30.0, 0.0),
        Vec3::new(side as f32, 90.0, side as f32),
    ];
    let near_pass = GpuCulling::new(device, None).expect("the pass");
    let t0 = Instant::now() + Duration::from_secs(1);
    let whole = t0 - lod::FADE;
    // The camera's own tile, with two water quads the step's plane crosses.
    let straddle = key(0, -1);
    // Standing in for its replacement until `t0 + FADE`.
    let stand_in = key(0, -2);
    // Fading in from `t0 + FADE / 2`: the CPU's, then the pass's from `t0 + FADE * 3 / 2`.
    let fading = key(-1, -1);
    let steady_water = key(1, -1);
    let dropped = key(1, 0);
    // One world: the near sections, the far tiles as the residency records them and the
    // water's last sorted orders. Built twice, so the CPU's frames and the split's each keep
    // their own sort state, as two renderers would.
    struct World {
        sections_near: Vec<((i32, i32, i32), [f64; 3], SectionGpu)>,
        sourced: Sourced,
        tiles: FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile>,
        grid: crate::gfx::lod::TileGrid,
        orders: FxHashMap<crate::lod::TileKey, Vec<u32>>,
    }
    let mut build = || -> World {
        // The near sections, each one solid quad and one water quad over it, all within two
        // sections of the camera's own (`nearby`): their water is always re-sorted.
        let mut sections_near: Vec<((i32, i32, i32), [f64; 3], SectionGpu)> = Vec::new();
        for (i, &(x, z)) in [(5, -3), (6, -3), (5, -2), (6, -2)].iter().enumerate() {
            sections_near.push((
                (x, 0, z),
                [f64::from(x * 16), 0.0, f64::from(z * 16)],
                mesh(
                    16.0,
                    (42.0, 0xFF40_4060 + 0x0001_0100 * u32::try_from(i).unwrap()),
                    &[([0.0, 0.0], 16.0, 60.0, 0x8090_60A0)],
                ),
            ));
        }

        let mut sourced = Sourced::default();
        let mut tiles: FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile> = Default::default();
        let mut grid = crate::gfx::lod::TileGrid::default();
        // Recorded as the residency records them: the first solid tile makes the table.
        let add = |k: crate::lod::TileKey,
                   s: SectionGpu,
                   shown: Option<Instant>,
                   previous: Option<(SectionGpu, Option<Instant>)>,
                   tiles: &mut FxHashMap<crate::lod::TileKey, crate::gfx::lod::Tile>,
                   grid: &mut crate::gfx::lod::TileGrid,
                   sourced: &mut Sourced| {
            tiles.insert(
                k,
                crate::gfx::lod::Tile::for_test(s, bounds, shown, previous),
            );
            grid.insert_bounds(k, bounds);
            assert!(!sourced.record(k, tiles.get_mut(&k).unwrap(), Some(&near_pass), device, t0));
        };

        // The camera's own tile: two water quads the step's plane crosses, over a solid quad
        // the pass draws.
        add(
            straddle,
            mesh(
                side as f32,
                (40.0, 0xFF70_7070),
                &[
                    ([150.0, 465.75], 16.0, 55.0, 0x80FF_3050),
                    ([150.0, 464.75], 16.0, 55.0, 0x8030_FF50),
                ],
            ),
            Some(whole),
            None,
            &mut tiles,
            &mut grid,
            &mut sourced,
        );
        // Standing in for its replacement: its previous mesh draws, and its water blends by
        // the tier that puts it over the others' (2, in the list the CPU draws).
        let water_full = [([0.0, 0.0], side as f32, 70.0, 0x80FF_8030)];
        let own = mesh(side as f32, (46.0, 0xFF70_7070), &water_full);
        let previous = mesh(side as f32, (44.0, 0xFF70_7070), &water_full);
        add(
            stand_in,
            own,
            Some(t0),
            Some((previous, Some(t0))),
            &mut tiles,
            &mut grid,
            &mut sourced,
        );
        // Fading in over the step: the pass leaves it the CPU's until its fade-in is due.
        add(
            fading,
            mesh(
                side as f32,
                (40.0, 0xFF70_7070),
                &[([0.0, 0.0], side as f32, 70.0, 0x8030_80FF)],
            ),
            Some(t0 + lod::FADE / 2),
            None,
            &mut tiles,
            &mut grid,
            &mut sourced,
        );
        // Whole and water-bearing: the pass draws its solid quads, the CPU its water.
        add(
            steady_water,
            mesh(
                side as f32,
                (40.0, 0xFF70_7070),
                &[([0.0, 0.0], side as f32, 70.0, 0x80FF_CC30)],
            ),
            Some(whole),
            None,
            &mut tiles,
            &mut grid,
            &mut sourced,
        );
        // Whole without water: steady, so not listed — the tile that shifts the CPU's list
        // against the walk's, which the tier's lookup must not cross.
        add(
            dropped,
            mesh(side as f32, (40.0, 0xFF70_7070), &[]),
            Some(whole),
            None,
            &mut tiles,
            &mut grid,
            &mut sourced,
        );
        // Filler around them, to give the walk more entries than the rotation's budget:
        // steady, no water, the pass's.
        for &(x, z) in [
            (-3, 0),
            (-2, 0),
            (-1, 0),
            (2, 0),
            (3, 0),
            (-2, 1),
            (-1, 1),
            (0, 1),
            (1, 1),
            (2, 1),
            (-1, -2),
            (1, -2),
        ]
        .iter()
        {
            add(
                key(x, z),
                mesh(side as f32, (40.0, 0xFF70_7070), &[]),
                Some(whole),
                None,
                &mut tiles,
                &mut grid,
                &mut sourced,
            );
        }
        assert!(!sourced.off());
        assert!(sourced.lists(straddle) && sourced.lists(stand_in));
        assert!(sourced.lists(fading) && sourced.lists(steady_water));
        assert!(!sourced.lists(dropped));
        World {
            sections_near,
            sourced,
            tiles,
            grid,
            orders: Default::default(),
        }
    };
    let mut cpu_world = build();
    let mut split_world = build();

    let rig = rig(device, queue, stride);
    let view = Frustum::from_planes(frustum(rig_view()));
    let mut uploads = Uploads::new(device);
    let mut sorter = translucency::Sorter::default();
    // What a frame decided, for the asserts on `draw_frame`'s own decisions (`#4769`'s second
    // and third defects): the far tiles the rotation re-sorted, and the stand-in's blend tier.
    struct Split {
        resorted: Vec<crate::lod::TileKey>,
        stand_in_tier: Option<u32>,
    }

    // One frame: the near sections and the walk's complete set drive the re-sort's rotation
    // and the water's order, what the pass left the CPU (`pass`) drives the draws and
    // instances, and the picture comes back with the far tiles' draw order.
    let mut frame = |world: &mut World,
                     camera: DVec3,
                     now: Instant,
                     pass: bool|
     -> (Vec<u32>, Vec<crate::lod::TileKey>, Split) {
        let World {
            sections_near,
            sourced,
            tiles,
            grid,
            orders,
        } = world;
        sourced.take_due(tiles, now);
        let mut visible: Vec<(u64, (i32, i32, i32), usize)> = sections_near
            .iter()
            .enumerate()
            .map(|(i, &(k, o, _))| {
                (
                    camera.distance(DVec3::from_array(o) + DVec3::new(8.0, 60.0, 8.0)) as u64,
                    k,
                    i,
                )
            })
            .collect();
        visible.sort_unstable_by_key(|&(d, k, _)| (d, k));
        // The walk's complete set, and the CPU's list: what the pass left it, which keeps
        // the walk's order.
        let mut far: Vec<lod::Visible> = Vec::new();
        let mut list = |v| far.push(v);
        walk_tiles(grid, tiles, camera, &view, now, None, &mut list);
        far.sort_unstable_by_key(|v| (v.distance, v.key));
        let visible_lods: Vec<lod::Visible> = if pass {
            far.iter()
                .filter(|v| sourced.lists(v.key))
                .copied()
                .collect()
        } else {
            far.clone()
        };
        let near_len = visible.len();
        // The rotation rests on the straddle tile: whichever frame walks it re-sorts it.
        let cursor = near_len
            + far
                .iter()
                .position(|v| v.key == straddle)
                .expect("the straddle tile in view");
        let rotation = Rotation::new(near_len, far.len(), cursor);

        let mut encoder = device.create_command_encoder(&Default::default());
        if pass {
            let prepared = sourced.prepare(
                device,
                queue,
                &mut uploads,
                &mut encoder,
                camera,
                -64,
                &view,
                None,
                None,
            );
            assert_eq!(prepared, Prepared::Gpu);
            sourced.dispatch(&mut encoder, None);
        }
        let mut world = cleared_pass(&rig, &mut encoder);
        let mut rows: Vec<[f32; 4]> = visible
            .iter()
            .map(|&(_, _, i)| {
                let (_, o, _) = &sections_near[i];
                lod::instance((DVec3::from_array(*o) - camera).as_vec3().to_array(), 0)
            })
            .collect();
        rows.extend(visible_lods.iter().map(|v| lod::instance(v.origin, v.fade)));
        let rows_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&rows),
            usage: wgpu::BufferUsages::VERTEX,
        });
        world.set_vertex_buffer(1, rows_buffer.slice(..));
        // The near sections' solid quads, then what the pass leaves the CPU of the far
        // tiles', then the pass's own solid draws.
        for (base, &(_, _, i)) in visible.iter().enumerate() {
            let (_, _, s) = &sections_near[i];
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            world.draw_indexed(0..6, s.solid_base() as i32, base as u32..base as u32 + 1);
        }
        for (i, v) in visible_lods.iter().enumerate() {
            if pass && sourced.draws_solid(v.key) {
                continue;
            }
            let s = tiles[&v.key].drawn();
            world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
            let index = (near_len + i) as u32;
            world.draw_indexed(0..6, s.solid_base() as i32, index..index + 1);
        }
        if pass {
            sourced.draw(&mut world, &arena, |_| None);
        }
        // The near sections' water and the far tiles', one back-to-front order: the
        // complete set drives the rotation, the CPU's list the slots and the tiers.
        world.set_pipeline(&rig.blended);
        // The pass left its origins at slot 1: the CPU's instances go back for the water.
        world.set_vertex_buffer(1, rows_buffer.slice(..));
        let mut translucent_draws: Vec<(u64, usize)> = Vec::new();
        for (i, &(distance, _, s_i)) in visible.iter().enumerate() {
            let (_, o, s) = &mut sections_near[s_i];
            if s.sorted_indices.is_some() {
                let scheduled = rotation.near(i);
                let origin = [o[0] as i32, o[1] as i32, o[2] as i32];
                if scheduled {
                    let _ = s.sort.update(
                        camera.to_array(),
                        camera.to_array(),
                        origin,
                        false,
                        &mut sorter,
                    );
                }
                let pov = translucency::viewpoint(camera.to_array(), origin);
                let (_, quads) = s.sort.drawn(s.translucent, false, pov);
                if quads > 0 {
                    translucent_draws.push((distance, i));
                }
            }
        }
        let mut resorted = Vec::new();
        let far_water = FarWater::new(&far, &visible_lods, rotation);
        for (i, instance, scheduled) in far_water.entries() {
            let v = &far[i];
            if scheduled {
                resorted.push(v.key);
            }
            let s = tiles
                .get_mut(&v.key)
                .expect("visible resident tile")
                .drawn_mut();
            if s.sorted_indices.is_some() {
                if (!s.sort.is_sorted() || scheduled)
                    && let Some(indices) =
                        s.sort
                            .update_lod(camera.to_array(), v.key.origin(), &mut sorter)
                {
                    orders.insert(v.key, indices.to_vec());
                }
                if s.translucent > 0 {
                    translucent_draws.push((v.distance, instance));
                }
            }
        }
        translucent_draws
            .sort_unstable_by_key(|&(distance, instance)| far_water.order(distance, instance));
        let stand_in_tier = visible_lods
            .iter()
            .position(|v| v.key == stand_in)
            .map(|slot| far_water.order(0, near_len + slot).0);
        let mut far_order: Vec<crate::lod::TileKey> = Vec::new();
        for &(_, instance) in &translucent_draws {
            if instance < near_len {
                let &(_, _, s_i) = &visible[instance];
                let (_, _, s) = &sections_near[s_i];
                world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
                world.draw_indexed(
                    0..6,
                    s.translucent_at.1 as i32,
                    instance as u32..instance as u32 + 1,
                );
            } else {
                let v = &visible_lods[instance - near_len];
                let s = tiles[&v.key].drawn();
                world.set_vertex_buffer(0, arena.buffer(s.page).slice(..));
                for quad in orders[&v.key].iter().step_by(6) {
                    world.draw_indexed(
                        0..6,
                        (s.translucent_at.1 + u64::from(*quad / 4)) as i32,
                        instance as u32..instance as u32 + 1,
                    );
                }
                far_order.push(v.key);
            }
        }
        drop(world);
        (
            copy_and_submit(device, queue, encoder, &rig, &mut uploads),
            far_order,
            Split {
                resorted,
                stand_in_tier,
            },
        )
    };

    let at = t0 + lod::FADE;
    // Before the handoff: the fading tile is the CPU's whole, the stand-in stands in.
    let (cpu, cpu_order, cf) = frame(&mut cpu_world, before, at, false);
    let (split, split_order, ff) = frame(&mut split_world, before, at, true);
    assert!(!split_world.sourced.draws_solid(fading));
    let black = |image: &[u32]| image.iter().filter(|&&p| p == 0xFF00_0000).count();
    let (covered, total) = (cpu.len() - black(&cpu), cpu.len());
    assert!(covered > 250, "the tiles cover {covered} of {total}");
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(
        wrong, 0,
        "{wrong} pixels of {total} differ with the pass on"
    );
    // The stand-in's water blends last, and the other water tiles keep the walk's order.
    assert_eq!(split_order.last(), Some(&stand_in));
    let without_stand_in = |o: &[crate::lod::TileKey]| {
        o.iter()
            .filter(|k| **k != stand_in)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(without_stand_in(&cpu_order), without_stand_in(&split_order));
    // Nothing was sorted yet: every water tile's order came out of its first sort, the same
    // in each world.
    assert_eq!(split_world.orders.len(), 4);
    assert_eq!(cpu_world.orders, split_world.orders);
    let before_orders = split_world.orders.clone();
    // `draw_frame`'s rotation puts a far tile past the near sections: the straddle tile, at
    // the cursor, re-sorts in each frame, with the pass on or off (the second defect left it
    // out of the window).
    assert!(cf.resorted.contains(&straddle) && ff.resorted.contains(&straddle));
    // `draw_frame`'s order reads the stand-in's tier from the list its slot names, the CPU's
    // (the third defect read the walk's complete list, where the slot is another tile's).
    assert_eq!(ff.stand_in_tier, Some(2));

    // The camera steps a block along z, across the straddle pair's plane: the rotation
    // re-sorts the pair in both worlds, the other water tiles' order stays, and the split
    // still draws the CPU's image.
    let (cpu, cpu_order, _) = frame(&mut cpu_world, after, at, false);
    let (split, split_order, _) = frame(&mut split_world, after, at, true);
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels of {total} differ over the step");
    assert_eq!(split_order.last(), Some(&stand_in));
    assert_eq!(without_stand_in(&cpu_order), without_stand_in(&split_order));
    let moved = |world: &World| -> BTreeSet<_> {
        world
            .orders
            .iter()
            .filter(|(k, o)| before_orders.get(k) != Some(*o))
            .map(|(k, _)| *k)
            .collect()
    };
    assert_eq!(moved(&cpu_world), BTreeSet::from([straddle]));
    assert_eq!(moved(&split_world), BTreeSet::from([straddle]));

    // After the handoff: the fading tile is whole and its solid quads the pass's, its water
    // still the CPU's; the stand-in's replacement is in. The split still draws the CPU's
    // image, the water in one order.
    let later = t0 + lod::FADE * 2;
    let (cpu, cpu_order, _) = frame(&mut cpu_world, after, later, false);
    let (split, split_order, _) = frame(&mut split_world, after, later, true);
    assert!(split_world.sourced.draws_solid(fading) && split_world.sourced.lists(fading));
    let wrong = cpu.iter().zip(&split).filter(|(a, b)| a != b).count();
    assert_eq!(
        wrong, 0,
        "{wrong} pixels of {total} differ after the handoff"
    );
    assert_eq!(cpu_order, split_order);
    assert!(split_world.sourced.draw_calls() > 0);
    #[cfg(debug_assertions)]
    split_world.sourced.check(&split_world.tiles, later, true);
}
