//! Regression tests for #5370: mod `RenderLevelStage` world draws (`WorldPrimitives`)
//! inside an active shader pack (`AfterOpaqueBlocks` against `colortex0` + `depthtex0`
//! before `final`) and post-pack consumers depth-testing against the exported reversed-Z
//! `world_depth` after `final`.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::Vertex;
use crate::gfx::world_primitives::{self, Batch, DrawRanges, WorldPrimitives};
use crate::hud::HudBatch;
use glam::Mat4;
use wgpu::{TextureFormat as T, util::DeviceExt};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn rgba(rgb: [u8; 3]) -> u32 {
    u32::from_le_bytes([rgb[0], rgb[1], rgb[2], 255])
}

/// Screen-space quad in `Mat4::IDENTITY` clip space (wgpu NDC: `+y` is screen top, reversed-Z
/// depth in `[0..1]` where larger is nearer).
fn ndc_quad([x0, y0, x1, y1]: [f32; 4], reverse_z: f32, rgb: [u8; 3]) -> [Vertex; 4] {
    let color = rgba(rgb);
    [[x0, y0], [x0, y1], [x1, y1], [x1, y0]].map(|[x, y]| Vertex {
        pos: [x, y, reverse_z],
        uv: [0.5, 0.5],
        layer: 0,
        color,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0, 0.0],
    })
}

/// Runs a shader pack whose `gbuffers_terrain` paints the left half (`x in [-1, 0]`) green at
/// GL depth `0.5` and whose `final` swaps R and B of `colortex0`. When `inside_pack` is true,
/// `quads` draw at `WorldBoundary::AfterOpaqueBlocks` against `colortex0` + `depthtex0`; when
/// false, `quads` draw onto `screen` against the exported reversed-Z `world_depth` after `final`.
/// Returns the screen pixels `[r, g, b]` at top-left `(2, 2)`, bottom-left `(2, 12)`, and
/// bottom-right `(12, 12)`.
fn render_stage_quads(inside_pack: bool, quads: &[Vertex]) -> Option<[[u8; 3]; 3]> {
    let Some(gpu) = Gpu::new() else {
        eprintln!("MOD_STAGE_NO_ADAPTER");
        return None;
    };
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragColor=vec4(c.b,c.g,c.r,1.0);}"
                .to_owned(),
        ),
    ];
    let terrain_inputs = sunrise_shaders::GbuffersInputs {
        declarations: "layout(location=0)in vec3 fixture_position;\n\
                       layout(location=2)in uint hz_v_layer;\n\
                       layout(location=3)in vec4 hz_v_color;\n"
            .into(),
        body: "hz_Vertex=vec4(fixture_position,1.0);hz_Color=vec4(1.0);hz_MultiTexCoord0=vec4(0.0);\
               hz_MultiTexCoord1=vec4(240.0);hz_MultiTexCoord2=vec4(240.0);hz_Normal=vec3(0.0,0.0,1.0);\n"
            .into(),
        ..Default::default()
    };
    let inputs = crate::gfx::pack_atlas::game_terrain(Some(terrain_inputs), &[]);
    const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,2=>Uint32,3=>Unorm8x4];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files,
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                prepass: None,
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        inputs,
    );
    assert!(post.has_terrain(), "pack terrain refused: {:?}", post.notes);

    // One solid terrain quad covering the left half of the frame (`x in [-1, 0]`).
    let opaque = u32::from(crate::mesher::OPAQUE_LAYER) | 1;
    let corners = [[-1.0f32, -1.0], [0.0, -1.0], [0.0, 1.0], [-1.0, 1.0]];
    let mut terrain_verts: Vec<u32> = Vec::new();
    for c in [0, 1, 2, 0, 2, 3] {
        let [x, y] = corners[c];
        terrain_verts.extend([
            x.to_bits(),
            y.to_bits(),
            0f32.to_bits(),
            opaque,
            0xFFFF_FFFFu32,
        ]);
    }
    let terrain_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod stage terrain fixture"),
            contents: bytemuck::cast_slice(&terrain_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

    let atlas = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("Mod stage white atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: T::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255, 255, 255, 255],
    );
    let atlas_view = atlas.create_view(&Default::default());
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);

    let quad_count = (quads.len() / 4) as u32;
    let stage = if inside_pack {
        world_primitives::STAGE_AFTER_OPAQUE_BLOCKS
    } else {
        world_primitives::STAGE_CUSTOM_GEOMETRY
    };
    let mut stage_ends = [[0u32; 4]; world_primitives::STAGES - 1];
    for end in stage_ends.iter_mut().skip(stage) {
        end[0] = quad_count;
    }
    let batch = Batch {
        quads,
        stage_ends,
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));

    let draws = [DrawRanges {
        primitives: [0..quad_count, 0..0, 0..0, 0..0],
        ..Default::default()
    }];
    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod stage readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut boundary_called = false;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            opaque_blocks: inside_pack,
            ..Default::default()
        },
        |enc, boundary| {
            if let WorldBoundary::AfterOpaqueBlocks { color, depth } = boundary {
                boundary_called = true;
                return primitives.draw_pack_prepared(enc, color, depth, &draws);
            }
            false
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(boundary_called, inside_pack);
    if !inside_pack {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Post-pack exported depth draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &screen,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        assert!(primitives.draw_prepared(&mut pass, None, &draws));
    }
    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * stride as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    let out = [sample(2, 2), sample(2, 12), sample(12, 12)];
    drop(mapped);
    read.unmap();
    Some(out)
}

#[test]
#[ignore = "daily"]
fn after_opaque_blocks_depth_tests_against_pack_terrain_and_runs_before_final() {
    let _gpu = crate::gfx::test_gpu::lock();
    // Farther full-screen red quad (reversed-Z 0.25 -> GL 0.75 > terrain 0.5) plus a nearer
    // top-half red quad (reversed-Z 0.75 -> GL 0.25 < terrain 0.5). `final` swaps R <-> B so
    // any red fragment drawn inside the pack reads back as blue.
    let mut quads = Vec::new();
    quads.extend_from_slice(&ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    quads.extend_from_slice(&ndc_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let Some([top_left, bottom_left, bottom_right]) = render_stage_quads(true, &quads) else {
        return;
    };
    assert_eq!(
        top_left, BLUE,
        "nearer AfterOpaqueBlocks quad must draw in front of pack terrain and pass through final"
    );
    assert_eq!(
        bottom_left, GREEN,
        "farther AfterOpaqueBlocks quad must be occluded by pack terrain depth"
    );
    assert_eq!(
        bottom_right, BLUE,
        "farther AfterOpaqueBlocks quad must appear where pack terrain is absent and pass through final"
    );
}

#[test]
#[ignore = "daily"]
fn post_pack_world_depth_export_depth_tests_against_pack_terrain() {
    let _gpu = crate::gfx::test_gpu::lock();
    // Same geometry drawn after `final` (red stays unswapped) against reversed-Z `world_depth`
    // exported from `depthtex0` by `fs_export_depth`.
    let mut quads = Vec::new();
    quads.extend_from_slice(&ndc_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    quads.extend_from_slice(&ndc_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let Some([top_left, bottom_left, bottom_right]) = render_stage_quads(false, &quads) else {
        return;
    };
    assert_eq!(
        top_left, RED,
        "nearer post-pack quad must draw in front of exported pack depth without final's swap"
    );
    assert_eq!(
        bottom_left, GREEN,
        "farther post-pack quad must be occluded by exported pack depth"
    );
    assert_eq!(
        bottom_right, RED,
        "farther post-pack quad must appear where pack terrain is absent without final's swap"
    );
}

#[test]
#[ignore = "daily"]
fn pack_boundaries_fire_in_order_and_draw_pack_custom_depth_tests_mod_pipelines() {
    use crate::gfx::mod_pipelines::{Batch as PipelineBatch, PipelineRun, PipelineVertex};
    use crate::mod_pipelines::Pipelines;
    use mod_api::{Identifier, PipelineBlend, RenderPipeline, WorldDepth};

    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        eprintln!("MOD_STAGE_NO_ADAPTER");
        return;
    };
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragColor=vec4(c.b,c.g,c.r,1.0);}"
                .to_owned(),
        ),
    ];
    let terrain_inputs = sunrise_shaders::GbuffersInputs {
        declarations: "layout(location=0)in vec3 fixture_position;\n\
                       layout(location=2)in uint hz_v_layer;\n\
                       layout(location=3)in vec4 hz_v_color;\n"
            .into(),
        body: "hz_Vertex=vec4(fixture_position,1.0);hz_Color=vec4(1.0);hz_MultiTexCoord0=vec4(0.0);\
               hz_MultiTexCoord1=vec4(240.0);hz_MultiTexCoord2=vec4(240.0);hz_Normal=vec3(0.0,0.0,1.0);\n"
            .into(),
        ..Default::default()
    };
    let inputs = crate::gfx::pack_atlas::game_terrain(Some(terrain_inputs), &[]);
    const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,2=>Uint32,3=>Unorm8x4];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files,
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                prepass: None,
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        inputs,
    );
    assert!(post.has_terrain(), "pack terrain refused: {:?}", post.notes);

    let opaque = u32::from(crate::mesher::OPAQUE_LAYER) | 1;
    let corners = [[-1.0f32, -1.0], [0.0, -1.0], [0.0, 1.0], [-1.0, 1.0]];
    let mut terrain_verts: Vec<u32> = Vec::new();
    for c in [0, 1, 2, 0, 2, 3] {
        let [x, y] = corners[c];
        terrain_verts.extend([
            x.to_bits(),
            y.to_bits(),
            0f32.to_bits(),
            opaque,
            0xFFFF_FFFFu32,
        ]);
    }
    let terrain_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod stage terrain fixture"),
            contents: bytemuck::cast_slice(&terrain_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let atlas = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("Mod stage white atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: T::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255, 255, 255, 255],
    );
    let atlas_view = atlas.create_view(&Default::default());

    let id = Identifier::parse("test:pack_custom").unwrap();
    let shader = Identifier::parse("test:core/pack_custom").unwrap();
    let registered =
        [
            RenderPipeline::new(id, shader, PipelineBlend::Translucent, WorldDepth::Tested)
                .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
                .optional(),
        ];
    let fsh = "#version 330\n#include <minecraft:dynamictransforms.glsl>\n\
               in vec2 texCoord0;\nin vec4 vertexColor;\nout vec4 fragColor;\n\
               void main() { fragColor = vertexColor * ColorModulator; }\n";
    let set = Pipelines::build(&registered, 1, |file| match file {
        "assets/test/shaders/core/pack_custom.fsh" => Ok(Some(fsh.to_owned())),
        other => {
            let rest = other.strip_prefix("assets/").unwrap();
            let (ns, path) = rest.split_once('/').unwrap();
            Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
        }
    })
    .unwrap();
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.build(&gpu.device, &set, None).unwrap();
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);

    let pipe_quad = |[x0, y0, x1, y1]: [f32; 4], reverse_z: f32, rgb: [u8; 3]| {
        let color = rgba(rgb);
        [[x0, y0], [x0, y1], [x1, y1], [x1, y0]].map(|[x, y]| PipelineVertex {
            pos: [x, y, reverse_z],
            uv: [0.5, 0.5],
            color,
        })
    };
    let mut pipe_verts = Vec::new();
    pipe_verts.extend_from_slice(&pipe_quad([-1.0, -1.0, 1.0, 1.0], 0.25, RED));
    pipe_verts.extend_from_slice(&pipe_quad([-1.0, 0.0, 1.0, 1.0], 0.75, RED));
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_CUSTOM_GEOMETRY as u32,
        quads: 2,
        modifier_stack: 0,
    }];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            vertices: &pipe_verts,
            runs: &runs,
            time: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));

    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod stage custom readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut seen = Vec::new();
    let mut custom_drew = false;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            sky: true,
            opaque_blocks: true,
            features: false,
            translucent_blocks: true,
            weather: true,
            custom_geometry: true,
        },
        |enc, boundary| match boundary {
            WorldBoundary::AfterSky => {
                seen.push("AfterSky");
                false
            }
            WorldBoundary::AfterOpaqueBlocks { .. } => {
                seen.push("AfterOpaqueBlocks");
                false
            }
            WorldBoundary::Features { .. } => {
                seen.push("Features");
                false
            }
            WorldBoundary::AfterTranslucentBlocks { .. } => {
                seen.push("AfterTranslucentBlocks");
                false
            }
            WorldBoundary::AfterWeather { .. } => {
                seen.push("AfterWeather");
                false
            }
            WorldBoundary::CustomGeometry { color, depth } => {
                seen.push("CustomGeometry");
                custom_drew = primitives.draw_pack_custom(enc, color, depth);
                custom_drew
            }
            WorldBoundary::AfterLevel => {
                seen.push("AfterLevel");
                false
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(
        seen,
        [
            "AfterSky",
            "AfterOpaqueBlocks",
            "AfterTranslucentBlocks",
            "AfterWeather",
            "CustomGeometry",
            "AfterLevel",
        ]
    );
    assert!(
        custom_drew,
        "draw_pack_custom must draw prepared ModPipelines runs"
    );

    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * stride as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    assert_eq!(
        sample(2, 2),
        BLUE,
        "nearer ModPipelines quad in draw_pack_custom must beat pack terrain and pass through final"
    );
    assert_eq!(
        sample(2, 12),
        GREEN,
        "farther ModPipelines quad in draw_pack_custom must be occluded by pack terrain depth"
    );
    assert_eq!(
        sample(12, 12),
        BLUE,
        "farther ModPipelines quad in draw_pack_custom must appear where terrain is absent"
    );
    drop(mapped);
    read.unmap();
}

/// §3a design test: with a pack that draws terrain and a composite pass that inverts colour, a
/// mod GLSL pipeline quad submitted at `AfterOpaqueBlocks` draws once at `CustomGeometry`
/// (before the pack's composite chain) and is inverted by the composite pass.
#[test]
#[ignore = "daily"]
fn pack_composite_inverts_mod_glsl_quad_drawn_once_after_opaque_blocks() {
    use crate::gfx::mod_pipelines::{Batch as PipelineBatch, PipelineRun, PipelineVertex};
    use crate::mod_pipelines::Pipelines;
    use mod_api::{Identifier, PipelineBlend, RenderPipeline, WorldDepth};

    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        eprintln!("MOD_STAGE_NO_ADAPTER");
        return;
    };
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".to_owned(),
        ),
        (
            "gbuffers_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("composite.vsh".to_owned(), VERTEX.to_owned()),
        (
            "composite.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){vec4 c=texture2D(colortex0,uv);gl_FragData[0]=vec4(1.0-c.rgb,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             const vec4 colortex0ClearColor = vec4(0.0, 0.0, 0.0, 1.0);\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let terrain_inputs = sunrise_shaders::GbuffersInputs {
        declarations: "layout(location=0)in vec3 fixture_position;\n\
                       layout(location=2)in uint hz_v_layer;\n\
                       layout(location=3)in vec4 hz_v_color;\n"
            .into(),
        body: "hz_Vertex=vec4(fixture_position,1.0);hz_Color=vec4(1.0);hz_MultiTexCoord0=vec4(0.0);\
               hz_MultiTexCoord1=vec4(240.0);hz_MultiTexCoord2=vec4(240.0);hz_Normal=vec3(0.0,0.0,1.0);\n"
            .into(),
        ..Default::default()
    };
    let inputs = crate::gfx::pack_atlas::game_terrain(Some(terrain_inputs), &[]);
    const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Float32x3,2=>Uint32,3=>Unorm8x4];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files,
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                prepass: None,
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        inputs,
    );
    assert!(post.has_terrain(), "pack terrain refused: {:?}", post.notes);

    let opaque = u32::from(crate::mesher::OPAQUE_LAYER) | 1;
    let corners = [[-1.0f32, -1.0], [0.0, -1.0], [0.0, 1.0], [-1.0, 1.0]];
    let mut terrain_verts: Vec<u32> = Vec::new();
    for c in [0, 1, 2, 0, 2, 3] {
        let [x, y] = corners[c];
        terrain_verts.extend([
            x.to_bits(),
            y.to_bits(),
            0f32.to_bits(),
            opaque,
            0xFFFF_FFFFu32,
        ]);
    }
    let terrain_buf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod composite test terrain"),
            contents: bytemuck::cast_slice(&terrain_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let atlas = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("Mod composite white atlas"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: T::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255, 255, 255, 255],
    );
    let atlas_view = atlas.create_view(&Default::default());

    // Additive red (100, 0, 0) quad: drawn once over black gives (100, 0, 0), inverted by
    // composite to (155, 255, 255); drawn twice would give (200, 0, 0) -> (55, 255, 255).
    let id = Identifier::parse("test:after_opaque_mod").unwrap();
    let shader = Identifier::parse("test:core/after_opaque_mod").unwrap();
    let registered = [
        RenderPipeline::new(id, shader, PipelineBlend::Additive, WorldDepth::Tested)
            .vertex_shader(Identifier::parse("minecraft:core/position_tex_color").unwrap())
            .optional(),
    ];
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
               void main() { fragColor = vertexColor; }\n";
    let set = Pipelines::build(&registered, 1, |file| match file {
        "assets/test/shaders/core/after_opaque_mod.fsh" => Ok(Some(fsh.to_owned())),
        other => {
            let rest = other.strip_prefix("assets/").unwrap();
            let (ns, path) = rest.split_once('/').unwrap();
            Ok(sunrise_mod_pipeline::stubs::read(&format!("{ns}:{path}")).map(str::to_owned))
        }
    })
    .unwrap();
    let mut primitives = WorldPrimitives::new(&gpu.device, (T::Rgba8Unorm, false));
    primitives.pipelines.build(&gpu.device, &set, None).unwrap();
    primitives.prepare_pack_pass(&gpu.device, post.formats[0], post.rgb & 1 != 0, false);

    // Additive red (100, 0, 0) quad over `gpu.clear`'s (16, 16, 16) background: drawn once
    // before composite gives (116, 16, 16), inverted to (139, 239, 239). Drawn twice would give
    // (216, 16, 16) -> (39, 239, 239); drawn after composite would not be inverted.
    let quad_color = rgba([100, 0, 0]);
    let pipe_verts: Vec<PipelineVertex> = [[0.0f32, -1.0], [0.0, 1.0], [0.5, 1.0], [0.5, -1.0]]
        .into_iter()
        .map(|[x, y]| PipelineVertex {
            pos: [x, y, 0.75],
            uv: [0.5, 0.5],
            color: quad_color,
        })
        .collect();
    let runs = [PipelineRun {
        slot: 0,
        stage: world_primitives::STAGE_AFTER_OPAQUE_BLOCKS as u32,
        quads: 1,
        modifier_stack: 0,
    }];
    let batch = Batch {
        pipelines: PipelineBatch {
            set: Some(&set),
            vertices: &pipe_verts,
            runs: &runs,
            time: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(primitives.prepare(
        &gpu.device,
        &mut |buffer, bytes| gpu.queue.write_buffer(buffer, 0, bytes),
        &batch,
        Mat4::IDENTITY,
        SIZE,
        &atlas_view,
        None,
    ));

    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod composite readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let mut draw_count = 0usize;
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &color,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |rp, list, _, _| {
            if list == TerrainList::Solid {
                rp.set_vertex_buffer(0, terrain_buf.slice(..));
                rp.draw(0..6, 0..1);
            }
        },
        WorldBoundaries {
            custom_geometry: true,
            ..Default::default()
        },
        |enc, boundary| {
            if let WorldBoundary::CustomGeometry { color, depth } = boundary {
                draw_count += 1;
                return primitives.draw_pack_stages(
                    enc,
                    color,
                    depth,
                    0,
                    world_primitives::STAGE_CUSTOM_GEOMETRY,
                );
            }
            false
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(draw_count, 1, "mod quad must draw once before composite");

    encoder.copy_texture_to_buffer(
        screen.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        screen.texture().size(),
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    let sample = |x: usize, y: usize| -> [u8; 3] {
        let base = y * stride as usize + x * 4;
        [mapped[base], mapped[base + 1], mapped[base + 2]]
    };
    assert_eq!(
        sample(2, 8),
        [255, 0, 255],
        "pack terrain green inverted by composite to magenta"
    );
    assert_eq!(
        sample(14, 8),
        [239, 239, 239],
        "untouched background (16, 16, 16) inverted by composite to (239, 239, 239)"
    );
    assert_eq!(
        sample(10, 8),
        [139, 239, 239],
        "AfterOpaqueBlocks mod quad must draw once before composite over (16,16,16) and invert to (139,239,239)"
    );
    drop(mapped);
    read.unmap();
}
