//! GPU tests for #5371: translating and executing mod `RenderPipeline` pack bindings inside
//! `PackPost` (custom candidate programs, `gbuffers_*` fallback, lazy auxiliary depth targets,
//! typed per-frame uniforms with previous-frame tracking, type-mismatch tolerance, and unbound
//! pipeline reporting).
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::mod_pipelines::PipelineVertex;
use crate::hud::HudBatch;
use wgpu::{TextureFormat as T, util::DeviceExt};

fn mod_quad_buffers(device: &wgpu::Device, z_gl: f32) -> (wgpu::Buffer, wgpu::Buffer) {
    let color = u32::from_le_bytes([255, 255, 255, 255]);
    // Full-screen quad in clip space (`gl_Position = vec4(mod_position, 1.0)`).
    let verts = [
        [-1.0f32, -1.0, z_gl],
        [-1.0, 1.0, z_gl],
        [1.0, 1.0, z_gl],
        [1.0, -1.0, z_gl],
    ]
    .map(|pos| PipelineVertex {
        pos,
        uv: [0.5, 0.5],
        color,
    });
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mod pack test vertices"),
        contents: bytemuck::cast_slice(&verts),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mod pack test indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vbuf, ibuf)
}

fn mod_gbuffers_inputs() -> sunrise_shaders::GbuffersInputs {
    sunrise_shaders::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 mod_position;\n\
             layout(location = 1) in vec2 mod_uv;\n\
             layout(location = 2) in vec4 mod_color;\n"
            .into(),
        body: "hz_Vertex = vec4(mod_position, 1.0);\n\
             hz_Color = mod_color;\n\
             hz_MultiTexCoord0 = vec4(mod_uv, 0.0, 1.0);\n\
             hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n\
             hz_MultiTexCoord2 = hz_MultiTexCoord1;\n\
             hz_Normal = vec3(0.0, 1.0, 0.0);\n"
            .into(),
        ..Default::default()
    }
}

fn load_with_mods(
    gpu: &Gpu,
    files: Vec<(String, String)>,
    requests: Vec<sunrise_shaders::ModPackProgramRequest>,
    runtime: &OwnedModPackRuntime,
) -> (PackPost, wgpu::TextureView, wgpu::TextureView) {
    load_with_mod_stages(gpu, files, requests, runtime, &[])
}

fn load_with_mod_stages(
    gpu: &Gpu,
    files: Vec<(String, String)>,
    requests: Vec<sunrise_shaders::ModPackProgramRequest>,
    runtime: &OwnedModPackRuntime,
    stages: &[Option<ModVertexStage>],
) -> (PackPost, wgpu::TextureView, wgpu::TextureView) {
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "mod-pack-binding-fixture",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (props, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let inputs = sunrise_shaders::GameInputs {
        mod_programs: requests,
        ..Default::default()
    };
    let post = sunrise_shaders::load_post(&pack, "", &props, &macros, &inputs);
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let world = World {
        atlas: None,
        material_maps: None,
        lightmap: color.clone(),
        terrain: None,
        block_ids: &[],
        sprite_rects: &[],
        celestial: None,
        gui_size: SIZE,
        clouds: None,
        weather: false,
        hand: false,
    };
    let mod_buffers = [Some(PipelineVertex::buffer_layout())];
    let built = PackPost::new_with_mods(
        (&gpu.device, &gpu.queue, &gpu.adapter),
        post,
        T::Rgba8Unorm,
        world,
        runtime.as_runtime(&mod_buffers).with_vertex_stages(stages),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("pack with mod bindings refused");
    (built, color, depth)
}

fn run_frame_and_sample_center(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) -> [u8; 4] {
    run_frame_and_sample(gpu, post, color, depth, false)
}

fn run_frame_and_sample(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    shadow_cache: bool,
) -> [u8; 4] {
    run_frame_and_sample_at(
        gpu,
        post,
        color,
        depth,
        shadow_cache,
        &[[SIZE[0] / 2, SIZE[1] / 2]],
    )[0]
}

/// Runs one frame and reads the final image at each `[x, y]` of `points`.
fn run_frame_and_sample_at(
    gpu: &Gpu,
    post: &mut PackPost,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    shadow_cache: bool,
    points: &[[u32; 2]],
) -> Vec<[u8; 4]> {
    let screen = gpu.texture(T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mod pack binding readback"),
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let hud = HudBatch::scratch(SIZE);
    let frame = Frame {
        shadow_cache,
        ..fixture_frame(&hud)
    };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        color,
        depth,
        &screen,
        &frame,
        &Default::default(),
        SIZE,
        SIZE,
        &[],
        &mut |_, _, _, _| {},
        WorldBoundaries::default(),
        |_, _| false,
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
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
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().unwrap();
    let px = points
        .iter()
        .map(|&[x, y]| {
            let offset = y as usize * stride as usize + x as usize * 4;
            [
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]
        })
        .collect();
    drop(data);
    read.unmap();
    px
}

#[test]
#[ignore = "daily"]
fn mod_pack_program_draws_aux_depth_and_tracks_previous_uniform() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform sampler2D modDepth0;\n\
             uniform sampler2D modDepth1;\n\
             uniform float modScale;\n\
             uniform float prevModScale;\n\
             varying vec2 uv;\n\
             void main(){\n\
                 float d0 = texture2D(modDepth0, uv).r;\n\
                 float d1 = texture2D(modDepth1, uv).r;\n\
                 float saw = (d0 < 0.7 && d1 < 0.7) ? 1.0 : 0.0;\n\
                 gl_FragColor = vec4(saw, modScale, prevModScale, 1.0);\n\
             }"
            .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: vec![ModDepthTargetDecl {
            label: "example:aux_depth".into(),
            depth_samplers: vec!["modDepth0".into()],
            opaque_depth_samplers: vec!["modDepth1".into()],
        }],
        uniforms: vec![ModUniformDecl {
            name: "modScale".into(),
            ty: ModUniformType::Float,
            previous_name: Some("prevModScale".into()),
        }],
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: Some(0),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: Default::default(),
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(
        post.is_mod_pipeline_bound(0),
        "mod_terrain not bound: {:?}",
        post.notes
    );

    // GL NDC z = -0.5 maps to window depth 0.25 (< 0.7).
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    post.set_mod_draws(vec![ModDrawChunk::quads(
        0,
        vbuf.clone(),
        ibuf.clone(),
        0..1,
    )]);

    // Frame 1: prev equals current (`0.25` -> ~64).
    assert!(post.set_mod_uniform("modScale", ModUniformValue::Float(0.25)));
    let px1 = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(post.allocated_mod_depth_targets(), (1, 1));
    assert!(
        px1[0] >= 250,
        "expected aux depth visible in final: {px1:?}"
    );
    assert!(
        (px1[1] as i16 - 64).abs() <= 2,
        "frame 1 current modScale mismatch: {px1:?}"
    );
    assert!(
        (px1[2] as i16 - 64).abs() <= 2,
        "frame 1 prevModScale should equal current: {px1:?}"
    );

    // Frame 2: resubmit per-frame mod draws; current updates to `0.75` (~191) and prev retains Frame 1's `0.25` (~64).
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    assert!(post.set_mod_uniform("modScale", ModUniformValue::Float(0.75)));
    let px2 = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!(px2[0] >= 250, "frame 2 aux depth lost: {px2:?}");
    assert!(
        (px2[1] as i16 - 191).abs() <= 2,
        "frame 2 current modScale mismatch: {px2:?}"
    );
    assert!(
        (px2[2] as i16 - 64).abs() <= 2,
        "frame 2 prevModScale should retain frame 1: {px2:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_linked_pass_honors_depth_compare_and_write() {
    use crate::mod_pipelines::{pack_program_requests_of, pack_runtime_specs_of};
    use mod_api::{
        DepthCompare, Identifier, PackBinding, PackDepthTarget, PackPassSlot, PipelineBlend,
        RenderPipeline, WorldDepth,
    };

    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = [
        (
            "mod_depth.vsh",
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}",
        ),
        (
            "mod_depth.fsh",
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}",
        ),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform sampler2D depthtex0;\n\
             varying vec2 uv;\n\
             void main(){\n\
                 bool drew = texture2D(colortex0, uv).r > 0.5;\n\
                 float depth = texture2D(depthtex0, uv).r;\n\
                 gl_FragColor = vec4(drew ? 1.0 : 0.0, depth == 0.5 ? 1.0 : 0.0,\n\
                                     depth == 0.75 ? 1.0 : 0.0, 1.0);\n\
             }",
        ),
    ]
    .map(|(name, source)| (name.into(), source.into()))
    .into();
    // All declarations draw in the same slot. A late pass without an explicit target may
    // test against main depth but cannot write it, even if it requests a write.
    let states = [
        (DepthCompare::Equal, false, true),
        (DepthCompare::Never, false, true),
        (DepthCompare::Less, false, true),
        (DepthCompare::Greater, false, true),
        (DepthCompare::Always, true, true),
        (DepthCompare::Always, true, false),
    ];
    let pipelines: Vec<_> = states
        .iter()
        .enumerate()
        .map(|(slot, &(compare, write, names_main))| {
            let id = Identifier::parse(&format!("example:linked_depth_{slot}")).unwrap();
            let mut binding = PackBinding::new()
                .program("mod_depth")
                .slot(PackPassSlot::AfterTranslucentGbuffers);
            if names_main {
                binding = binding.depth_target(PackDepthTarget::Main);
            }
            RenderPipeline::new(id.clone(), id, PipelineBlend::Opaque, WorldDepth::Tested)
                .pack_binding(binding)
                .depth_compare(compare)
                .depth_write(write)
        })
        .collect();
    let (mut post, color, depth) = load_with_mods(
        &gpu,
        files,
        pack_program_requests_of(&pipelines),
        &pack_runtime_specs_of(&pipelines),
    );
    for slot in 0..pipelines.len() {
        assert!(
            post.is_mod_pipeline_bound(slot),
            "slot {slot}: {:?}",
            post.notes
        );
    }

    // Seed 0.5 anew each frame. GL clip-space z in [-1, 1] maps to pack depth in [0, 1].
    // Read red for a draw, green for the seed depth and blue for the far draw's depth.
    for (name, slot, clip_z, drew, stored_far) in [
        ("Equal at seed", 0, 0.0, true, false),
        ("Equal nearer", 0, -0.5, false, false),
        ("Never nearer", 1, -0.5, false, false),
        ("Less farther (reversed-Z)", 2, 0.5, true, false),
        ("Greater farther (reversed-Z)", 3, 0.5, false, false),
        ("Greater nearer without write", 3, -0.5, true, false),
        ("Always farther with write", 4, 0.5, true, true),
        ("Always farther in read-only slot", 5, 0.5, true, false),
    ] {
        gpu.clear(&color, &depth, 0.5);
        let (vertices, indices) = mod_quad_buffers(&gpu.device, clip_z);
        post.set_mod_draws(vec![ModDrawChunk::quads(slot, vertices, indices, 0..1)]);
        let pixel = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
        assert_eq!(
            pixel,
            [
                if drew { 255 } else { 0 },
                if stored_far { 0 } else { 255 },
                if stored_far { 255 } else { 0 },
                255,
            ],
            "{name}",
        );
    }
}

#[test]
#[ignore = "daily"]
fn mod_pack_falls_back_to_gbuffers_terrain_when_custom_program_absent() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "gbuffers_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
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
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into(), "gbuffers_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: Default::default(),
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(
        post.is_mod_pipeline_bound(0),
        "expected fallback to gbuffers_terrain: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!(
        px[1] >= 250 && px[0] <= 5 && px[2] <= 5,
        "expected green from gbuffers_terrain: {px:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_aux_depth_is_lazy_when_pack_never_samples_it() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,0.0,1.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: vec![ModDepthTargetDecl {
            label: "example:unused_aux_depth".into(),
            depth_samplers: vec!["modDepth0".into()],
            opaque_depth_samplers: vec!["modDepth1".into()],
        }],
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: Some(0),
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: Default::default(),
        }],
    };
    let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(post.is_mod_pipeline_bound(0));
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, 0.0);
    post.set_mod_draws(vec![ModDrawChunk::quads(0, vbuf, ibuf, 0..1)]);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert_eq!(
        post.allocated_mod_depth_targets(),
        (0, 0),
        "unsampled aux depth target must not allocate GPU textures"
    );
    assert!(
        px[2] >= 250 && px[0] <= 5 && px[1] <= 5,
        "expected blue from mod_terrain: {px:?}"
    );
}

#[test]
#[ignore = "daily"]
fn mod_pack_type_mismatch_uniform_and_unbound_pipeline_never_panic() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,1.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\n\
             uniform sampler2D colortex0;\n\
             uniform int modScale;\n\
             varying vec2 uv;\n\
             void main(){gl_FragColor=texture2D(colortex0,uv) + vec4(float(modScale));}"
                .to_owned(),
        ),
    ];
    let requests = vec![
        sunrise_shaders::ModPackProgramRequest {
            slot: 0,
            programs: vec!["mod_terrain".into()],
            group: sunrise_shaders::Group::Gbuffers,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        },
        sunrise_shaders::ModPackProgramRequest {
            slot: 1,
            programs: vec!["missing_only_program".into()],
            group: sunrise_shaders::Group::Gbuffers,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        },
    ];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: vec![ModUniformDecl {
            name: "modScale".into(),
            ty: ModUniformType::Float,
            previous_name: None,
        }],
        pipelines: vec![
            ModPipelinePackSpec {
                slot: 0,
                pass_slot: ModPassSlot::AfterOpaqueGbuffers,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                cull: false,
                program_mode: Default::default(),
            },
            ModPipelinePackSpec {
                slot: 1,
                pass_slot: ModPassSlot::AfterOpaqueGbuffers,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                cull: false,
                program_mode: Default::default(),
            },
        ],
    };
    let (post, _, _) = load_with_mods(&gpu, files, requests, &runtime);
    assert!(post.is_mod_pipeline_bound(0));
    assert!(!post.is_mod_pipeline_bound(1));
    assert!(
        post.notes
            .iter()
            .any(|n| n.contains("modScale") && n.contains("left unbound")),
        "expected mismatch diagnostic in notes: {:?}",
        post.notes
    );
}

/// A mod pipeline bound to the pack's shadow pass draws into the shadow map, and its geometry
/// forces the pass to run again while the shadow cache is on: the quad's colour comes from a
/// per-frame uniform, so a kept map would show the first frame's colour on the second. Covers
/// the transient quad chunk and the retained non-indexed chunk a `MeshRef` draw makes.
#[test]
#[ignore = "daily"]
fn mod_shadow_pass_geometry_is_drawn_and_invalidates_the_shadow_cache() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    for retained in [false, true] {
        let files = vec![
            (
                "shadow.vsh".to_owned(),
                "#version 120\nvoid main(){gl_Position=vec4(2.0,2.0,2.0,1.0);}".to_owned(),
            ),
            (
                "shadow.fsh".to_owned(),
                "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0);}"
                    .to_owned(),
            ),
            (
                "mod_shadow.vsh".to_owned(),
                "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
            ),
            (
                "mod_shadow.fsh".to_owned(),
                "#version 120\n/* DRAWBUFFERS:0 */\nuniform float modColor;\n\
                 void main(){gl_FragData[0]=vec4(modColor,0.0,0.0,1.0);}"
                    .to_owned(),
            ),
            ("final.vsh".to_owned(), VERTEX.to_owned()),
            (
                "final.fsh".to_owned(),
                "#version 120\nuniform sampler2D shadowcolor0;varying vec2 uv;\n\
                 void main(){gl_FragColor=vec4(texture2D(shadowcolor0,uv).rgb,1.0);}"
                    .to_owned(),
            ),
        ];
        let requests = vec![sunrise_shaders::ModPackProgramRequest {
            slot: 0,
            programs: vec!["mod_shadow".into(), "shadow".into()],
            group: sunrise_shaders::Group::Shadow,
            inputs: mod_gbuffers_inputs(),
            default_blend: None,
        }];
        let runtime = OwnedModPackRuntime {
            depth_targets: Vec::new(),
            uniforms: vec![ModUniformDecl {
                name: "modColor".into(),
                ty: ModUniformType::Float,
                previous_name: None,
            }],
            pipelines: vec![ModPipelinePackSpec {
                slot: 0,
                pass_slot: ModPassSlot::Shadow,
                depth_target: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                cull: false,
                program_mode: Default::default(),
            }],
        };
        let (mut post, color, depth) = load_with_mods(&gpu, files, requests, &runtime);
        assert!(
            post.is_mod_pipeline_bound(0),
            "mod_shadow not bound: {:?}",
            post.notes
        );
        let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
        let draw = || {
            if retained {
                ModDrawChunk {
                    slot: 0,
                    vertex_buffers: vec![(0, vbuf.clone(), 0..vbuf.size())],
                    kind: ModDrawKind::Indexed {
                        buffer: ibuf.clone(),
                        format: wgpu::IndexFormat::Uint32,
                        indices: 0..6,
                        base_vertex: 0,
                    },
                    instances: 0..1,
                    mod_bind_group: None,
                    mod_globals: None,
                }
            } else {
                ModDrawChunk::quads(0, vbuf.clone(), ibuf.clone(), 0..1)
            }
        };
        for (value, expected) in [(0.25, 64), (0.75, 191)] {
            post.set_mod_draws(vec![draw()]);
            assert!(post.set_mod_uniform("modColor", ModUniformValue::Float(value)));
            let px = run_frame_and_sample(&gpu, &mut post, &color, &depth, true);
            assert!(
                (px[0] as i16 - expected).abs() <= 2,
                "retained {retained}: shadow map shows {px:?}, expected {expected}"
            );
        }
    }
}

/// A pack whose `mod_terrain` fragment stage reads `uv` (and `extra` when `extra` is set), with a
/// `final` that shows `colortex0`.
fn own_stage_pack(extra: bool) -> Vec<(String, String)> {
    let (decl, read) = if extra {
        (
            "varying vec2 uv;\nvarying vec3 extra;\n",
            "vec4(uv, extra.x, 1.0)",
        )
    } else {
        ("varying vec2 uv;\n", "vec4(uv, 0.0, 1.0)")
    };
    let write = if extra {
        "uv = vec2(0.0); extra = vec3(0.0);"
    } else {
        "uv = vec2(0.0);"
    };
    vec![
        (
            "mod_terrain.vsh".to_owned(),
            format!(
                "#version 120\n{decl}void main(){{gl_Position=vec4(gl_Vertex.xyz,1.0);{write}}}"
            ),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            format!(
                "#version 120\n/* DRAWBUFFERS:0 */\n{decl}void main(){{gl_FragData[0]={read};}}"
            ),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ]
}

/// The mod's vertex stage `wgsl`, over the engine's mod vertex format, with `set1_layout` as its
/// own set 1 and an empty frame-uniform group.
fn vertex_stage_from(
    device: &wgpu::Device,
    wgsl: &str,
    set1_layout: Option<wgpu::BindGroupLayout>,
) -> ModVertexStage {
    let layout = PipelineVertex::buffer_layout();
    ModVertexStage {
        module: naga::front::wgsl::parse_str(wgsl).expect("own vertex stage parses"),
        buffers: vec![ModVertexBuffer {
            binding: 0,
            stride: layout.array_stride,
            step_mode: layout.step_mode,
            attributes: layout.attributes.to_vec(),
        }],
        globals_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack test globals"),
            entries: &[],
        }),
        set1_layout,
    }
}

/// The mod's own vertex stage: writing the constant varying `uv = (0.5, 0.25)` at location 0 and
/// nothing else.
fn own_vertex_stage(device: &wgpu::Device) -> ModVertexStage {
    vertex_stage_from(
        device,
        "struct Out {
            @builtin(position) position: vec4<f32>,
            @location(0) uv: vec2<f32>,
        };
        @vertex fn main(@location(0) p: vec3<f32>) -> Out {
            var o: Out;
            o.position = vec4<f32>(p.xy, p.z + 0.75, 1.0);
            o.uv = vec2<f32>(0.5, 0.25);
            return o;
        }",
        None,
    )
}

fn own_stage_runtime() -> (
    Vec<sunrise_shaders::ModPackProgramRequest>,
    OwnedModPackRuntime,
) {
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: ModProgramMode::ModVertexPackFragment,
        }],
    };
    (requests, runtime)
}

#[test]
#[ignore = "daily"]
fn mod_vertex_pack_fragment_draws_the_mods_vertices_through_the_packs_fragment_stage() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let (requests, runtime) = own_stage_runtime();
    let stages = [Some(own_vertex_stage(&gpu.device))];
    let (mut post, color, depth) =
        load_with_mod_stages(&gpu, own_stage_pack(false), requests, &runtime, &stages);
    assert!(
        post.is_mod_pipeline_bound(0),
        "own vertex stage not linked: {:?}",
        post.notes
    );
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    let globals = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack test globals"),
        layout: &stages[0].as_ref().unwrap().globals_layout,
        entries: &[],
    });
    let mut chunk = ModDrawChunk::quads(0, vbuf, ibuf, 0..1);
    chunk.mod_globals = Some(globals);
    post.set_mod_draws(vec![chunk]);
    // The pack's fragment stage wrote the mod's varying `(0.5, 0.25)` into colortex0.
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!((px[0] as i16 - 128).abs() <= 2, "red {px:?}");
    assert!((px[1] as i16 - 64).abs() <= 2, "green {px:?}");
}

#[test]
#[ignore = "daily"]
fn mod_vertex_pack_fragment_with_a_varying_the_mod_does_not_write_is_unbound() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let (requests, runtime) = own_stage_runtime();
    let stages = [Some(own_vertex_stage(&gpu.device))];
    let (post, _color, _depth) =
        load_with_mod_stages(&gpu, own_stage_pack(true), requests, &runtime, &stages);
    assert!(
        !post.is_mod_pipeline_bound(0),
        "a pack reading a varying the mod's vertex stage lacks must leave the slot unbound"
    );
    assert!(
        post.notes
            .iter()
            .any(|n| n.contains("mod's own vertex stage")),
        "no reason given: {:?}",
        post.notes
    );
}

/// Draws one full-screen quad through slot 0 with the stage's empty frame-uniform group and, when
/// given, its set-1 bind group.
fn draw_own_quad(
    gpu: &Gpu,
    post: &mut PackPost,
    stage: &ModVertexStage,
    set1: Option<wgpu::BindGroup>,
) {
    let (vbuf, ibuf) = mod_quad_buffers(&gpu.device, -0.5);
    let mut chunk = ModDrawChunk::quads(0, vbuf, ibuf, 0..1);
    chunk.mod_globals = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack test globals"),
        layout: &stage.globals_layout,
        entries: &[],
    }));
    chunk.mod_bind_group = set1.map(|bg| (bg, Vec::new()));
    post.set_mod_draws(vec![chunk]);
}

/// The pack declares `varying vec2 b; varying vec2 a;`, so its translator puts `b` at location 0
/// and `a` at location 1; the mod's vertex stage declares `a` at 0 and `b` at 1, the other way
/// round. Linked by name, the pack still reads `a = (0.5, 0.25)` and `b = (0.75, 0.125)`.
#[test]
#[ignore = "daily"]
fn mod_vertex_pack_fragment_links_varyings_by_name_not_location() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let decl = "varying vec2 b;\nvarying vec2 a;\n";
    let mut files = own_stage_pack(false);
    files[0].1 = format!(
        "#version 120\n{decl}void main(){{gl_Position=vec4(gl_Vertex.xyz,1.0);a=vec2(0.0);b=vec2(0.0);}}"
    );
    files[1].1 = format!(
        "#version 120\n/* DRAWBUFFERS:0 */\n{decl}void main(){{gl_FragData[0]=vec4(a,b.x,1.0);}}"
    );
    let (requests, runtime) = own_stage_runtime();
    let stages = [Some(vertex_stage_from(
        &gpu.device,
        "struct Out {
            @builtin(position) position: vec4<f32>,
            @location(0) a: vec2<f32>,
            @location(1) b: vec2<f32>,
        };
        @vertex fn main(@location(0) p: vec3<f32>) -> Out {
            var o: Out;
            o.position = vec4<f32>(p.xy, p.z + 0.75, 1.0);
            o.a = vec2<f32>(0.5, 0.25);
            o.b = vec2<f32>(0.75, 0.125);
            return o;
        }",
        None,
    ))];
    let (mut post, color, depth) = load_with_mod_stages(&gpu, files, requests, &runtime, &stages);
    assert!(
        post.is_mod_pipeline_bound(0),
        "varyings not linked by name: {:?}",
        post.notes
    );
    draw_own_quad(&gpu, &mut post, stages[0].as_ref().unwrap(), None);
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!((px[0] as i16 - 128).abs() <= 2, "a.x {px:?}");
    assert!((px[1] as i16 - 64).abs() <= 2, "a.y {px:?}");
    assert!((px[2] as i16 - 191).abs() <= 2, "b.x {px:?}");
}

/// A pack program with a geometry stage takes its vertices through the pack's own vertex stage,
/// which the mod's can't replace: the slot stays unbound, with a note and no wgpu validation
/// error.
#[test]
#[ignore = "daily"]
fn mod_vertex_pack_fragment_against_a_geometry_stage_program_is_unbound_with_a_note() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let files = vec![
        (
            "shadow.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(2.0,2.0,2.0,1.0);}".to_owned(),
        ),
        (
            "shadow.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0);}".to_owned(),
        ),
        (
            "mod_shadow.vsh".to_owned(),
            "#version 330\nout vec2 vUv;\nvoid main(){gl_Position=ftransform();vUv=vec2(0.0);}\n"
                .to_owned(),
        ),
        (
            "mod_shadow.gsh".to_owned(),
            "#version 330\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
             in vec2 vUv[];\nout vec2 gUv;\nvoid main() {\n    for (int i = 0; i < 3; i++) {\n        \
             gl_Position = gl_in[i].gl_Position;\n        gUv = vUv[i];\n        EmitVertex();\n    }\n    \
             EndPrimitive();\n}\n"
                .to_owned(),
        ),
        (
            "mod_shadow.fsh".to_owned(),
            "#version 330\nin vec2 gUv;\nlayout(location = 0) out vec4 c;\nvoid main(){c=vec4(gUv,0.0,1.0);}\n"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D shadowcolor0;varying vec2 uv;\n\
             void main(){gl_FragColor=vec4(texture2D(shadowcolor0,uv).rgb,1.0);}"
                .to_owned(),
        ),
    ];
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_shadow".into()],
        group: sunrise_shaders::Group::Shadow,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::Shadow,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: ModProgramMode::ModVertexPackFragment,
        }],
    };
    let stages = [Some(own_vertex_stage(&gpu.device))];
    let (post, _color, _depth) = load_with_mod_stages(&gpu, files, requests, &runtime, &stages);
    assert!(
        !post.is_mod_pipeline_bound(0),
        "a geometry-stage program must leave the slot unbound"
    );
    assert!(
        post.notes.iter().any(|n| n.contains("geometry stage")),
        "no reason given: {:?}",
        post.notes
    );
}

/// The mod's vertex stage reads its own set 1, a uniform buffer: the engine relocates it to
/// `MOD_SET_BASE + 1`, after the pack's sets, and the draw's bind group supplies it there. The
/// buffer's `(0.5, 0.25)` reaches the pack's fragment stage as `uv`.
#[test]
#[ignore = "daily"]
fn mod_vertex_pack_fragment_reads_its_own_set_one_uniform_after_the_packs_sets() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mod pack test set 1"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let stage = vertex_stage_from(
        &gpu.device,
        "struct Out {
            @builtin(position) position: vec4<f32>,
            @location(0) uv: vec2<f32>,
        };
        struct Own { value: vec4<f32> };
        @group(1) @binding(0) var<uniform> own: Own;
        @vertex fn main(@location(0) p: vec3<f32>) -> Out {
            var o: Out;
            o.position = vec4<f32>(p.xy, p.z + 0.75, 1.0);
            o.uv = own.value.xy;
            return o;
        }",
        Some(layout.clone()),
    );
    let buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mod pack test set 1 uniform"),
            contents: bytemuck::cast_slice(&[0.5f32, 0.25, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let set1 = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mod pack test set 1"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let (requests, runtime) = own_stage_runtime();
    let stages = [Some(stage)];
    let (mut post, color, depth) =
        load_with_mod_stages(&gpu, own_stage_pack(false), requests, &runtime, &stages);
    assert!(
        post.is_mod_pipeline_bound(0),
        "set 1 not relocated: {:?}",
        post.notes
    );
    draw_own_quad(&gpu, &mut post, stages[0].as_ref().unwrap(), Some(set1));
    let px = run_frame_and_sample_center(&gpu, &mut post, &color, &depth);
    assert!((px[0] as i16 - 128).abs() <= 2, "red {px:?}");
    assert!((px[1] as i16 - 64).abs() <= 2, "green {px:?}");
}

/// A pack-linked `ModDrawKind::Indirect` draws two indexed commands, the left and the right half
/// of the screen. Without a count buffer both draw; with one (where the adapter draws GPU-written
/// counts, #5908) holding 1, only command 0 does.
#[test]
#[ignore = "daily"]
fn mod_pack_indirect_draw_with_a_count_buffer_draws_only_the_counted_commands() {
    let _gpu_guard = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::with_features(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT) else {
        return;
    };
    let counts = crate::gfx::mod_render_targets::multi_draw_indirect_count(
        &gpu.adapter.get_info(),
        gpu.device.features(),
    );
    let files = vec![
        (
            "mod_terrain.vsh".to_owned(),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xyz,1.0);}".to_owned(),
        ),
        (
            "mod_terrain.fsh".to_owned(),
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0,0.0,0.0,1.0);}"
                .to_owned(),
        ),
        ("final.vsh".to_owned(), VERTEX.to_owned()),
        (
            "final.fsh".to_owned(),
            "#version 120\nuniform sampler2D colortex0;varying vec2 uv;\
             void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .to_owned(),
        ),
    ];
    let init = |contents: &[u8], usage| {
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mod pack indirect test"),
                contents,
                usage,
            })
    };
    // Two quads: x in -1..0 (vertices 0..4) and 0..1 (vertices 4..8).
    let color = u32::from_le_bytes([255, 255, 255, 255]);
    let vertex = |x: f32, y: f32| PipelineVertex {
        pos: [x, y, -0.5],
        uv: [0.5, 0.5],
        color,
    };
    let vertices = [
        vertex(-1.0, -1.0),
        vertex(-1.0, 1.0),
        vertex(0.0, 1.0),
        vertex(0.0, -1.0),
        vertex(0.0, -1.0),
        vertex(0.0, 1.0),
        vertex(1.0, 1.0),
        vertex(1.0, -1.0),
    ];
    let vbuf = init(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
    let vbuf_len = vbuf.size();
    let indices: [u32; 12] = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    let ibuf = init(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
    // `index_count, instance_count, first_index, base_vertex, first_instance`.
    let commands: [[u32; 5]; 2] = [[6, 1, 0, 0, 0], [6, 1, 6, 0, 0]];
    let command_buffer = init(
        bytemuck::cast_slice(&commands),
        wgpu::BufferUsages::INDIRECT,
    );
    let count_buffer = init(bytemuck::cast_slice(&[1u32]), wgpu::BufferUsages::INDIRECT);
    let requests = vec![sunrise_shaders::ModPackProgramRequest {
        slot: 0,
        programs: vec!["mod_terrain".into()],
        group: sunrise_shaders::Group::Gbuffers,
        inputs: mod_gbuffers_inputs(),
        default_blend: None,
    }];
    let runtime = OwnedModPackRuntime {
        depth_targets: Vec::new(),
        uniforms: Vec::new(),
        pipelines: vec![ModPipelinePackSpec {
            slot: 0,
            pass_slot: ModPassSlot::AfterOpaqueGbuffers,
            depth_target: None,
            depth_write: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            cull: false,
            program_mode: Default::default(),
        }],
    };
    // The final image at the middle of each half, drawn with `count_buffer`.
    let draw = |count_buffer: Option<(wgpu::Buffer, u64)>| {
        let (mut post, color, depth) =
            load_with_mods(&gpu, files.clone(), requests.clone(), &runtime);
        assert!(
            post.is_mod_pipeline_bound(0),
            "mod_terrain not bound: {:?}",
            post.notes
        );
        post.set_mod_draws(vec![ModDrawChunk {
            slot: 0,
            vertex_buffers: vec![(0, vbuf.clone(), 0..vbuf_len)],
            kind: ModDrawKind::Indirect {
                buffer: command_buffer.clone(),
                offset: 0,
                count: 2,
                count_buffer,
                index: Some((ibuf.clone(), wgpu::IndexFormat::Uint32)),
            },
            instances: 0..1,
            mod_bind_group: None,
            mod_globals: None,
        }]);
        run_frame_and_sample_at(
            &gpu,
            &mut post,
            &color,
            &depth,
            false,
            &[[SIZE[0] / 4, SIZE[1] / 2], [SIZE[0] * 3 / 4, SIZE[1] / 2]],
        )
    };
    let both = draw(None);
    assert!(both[0][0] >= 250, "command 0 without a count: {both:?}");
    assert!(both[1][0] >= 250, "command 1 without a count: {both:?}");
    if !counts {
        eprintln!(
            "no count draws on {:?}: the counted draw is not run",
            gpu.adapter.get_info()
        );
        return;
    }
    let counted = draw(Some((count_buffer, 0)));
    assert!(counted[0][0] >= 250, "command 0 is counted: {counted:?}");
    // Command 1 left the fixture's clear colour (`Gpu::clear`: 0.0625 per channel, 16 of 255).
    assert_eq!(
        counted[1],
        [16, 16, 16, 255],
        "command 1 is past the count of 1: {counted:?}"
    );
}
