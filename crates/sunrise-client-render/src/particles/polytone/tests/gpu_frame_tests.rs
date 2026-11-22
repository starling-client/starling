//! A frame of the packs' GPU particles as the game builds it (`polytone_gpu_decide`, the quads,
//! `polytone_gpu_hand_over`) with a renderer that keeps what it is handed: the renderer decides
//! before the heightmap is read, only the readers that draw in the frame read it, and a shader
//! that samples it gets the heights its block advertises before its first frame.
use super::super::super::polytone_gpu::{
    FALLBACK_QUADS, GpuParticleRenderer, HeightmapSource, ShaderBatch, ShaderRequest,
};
use super::*;
use std::cell::Cell;
use sunrise_polytone::gpu_particle::{HeightmapArea, Surface};

/// A renderer that draws every shader it is asked about, or none (as on a frame of linear
/// light), and keeps what it is handed.
#[derive(Default)]
pub(super) struct Fake {
    pub(super) refuses: bool,
    /// Each request of the last verdict: the shader's id and its quads.
    pub(super) asked: Vec<(String, usize)>,
    /// Every heightmap handed over, in order.
    pub(super) uploads: Vec<Surface>,
    pub(super) batches: Vec<ShaderBatch>,
    /// The quads it has room for from the types drawn on the CPU (`FALLBACK_QUADS` if none).
    pub(super) room: Option<usize>,
}

impl GpuParticleRenderer for Fake {
    fn decide_polytone_shaders(&mut self, requests: &[ShaderRequest]) {
        self.asked = requests
            .iter()
            .map(|r| (r.shader.id.clone(), r.quads))
            .collect();
        for request in requests {
            request.shader.set_drawn(!self.refuses);
        }
    }

    fn polytone_fallback_quads(&self) -> usize {
        self.room.unwrap_or(FALLBACK_QUADS)
    }

    fn set_polytone_heightmap(&mut self, surface: &Surface) {
        self.uploads.push(surface.clone());
    }

    fn set_polytone_batches(&mut self, batches: Vec<ShaderBatch>) {
        self.batches = batches;
    }
}

/// One frame at `camera` with the heightmap of 4 columns a side above y 0 read from `top`:
/// the quads drawn on the CPU.
pub(super) fn frame(
    engine: &mut ParticleEngine,
    renderer: Option<&mut Fake>,
    camera: DVec3,
    top: &dyn Fn(i32, i32) -> i32,
) -> usize {
    let source = HeightmapSource {
        size: 4,
        min_y: 0,
        top,
    };
    frame_of(engine, renderer, camera, &source)
}

fn frame_of(
    engine: &mut ParticleEngine,
    mut renderer: Option<&mut Fake>,
    camera: DVec3,
    source: &HeightmapSource,
) -> usize {
    engine.polytone_gpu_decide(renderer.as_deref_mut().map(|r| r as _), camera, source);
    let mut out = Vec::new();
    engine.render(camera, glam::Quat::IDENTITY, 0.0, &|_| 0xF0, &mut out);
    engine.polytone_gpu_hand_over(renderer.map(|r| r as _), camera, 0.0, &|_| 0xF0);
    out.len() / 4
}

/// `top` for every column, counting the columns read in `reads`.
fn ground(top: &Cell<i32>, reads: &Cell<usize>) -> impl Fn(i32, i32) -> i32 {
    move |_, _| {
        reads.set(reads.get() + 1);
        top.get()
    }
}

/// `shader_pack`'s `test:rain` with `fsh`, dying under the heightmap, drawn as `render_type`,
/// with `area` copies when given.
fn kill_shader_pack(
    fsh: &str,
    uniforms: serde_json::Value,
    render_type: &str,
    area: Option<u32>,
) -> sunrise_assets::resource_source::ResourceSource {
    let mut rain = serde_json::json!({
        "type": "gpu", "texture": "test:textures/particle/rain.png",
        "render_type": render_type, "shader": "test:rain", "uniforms": uniforms,
        "kill_below_heightmap": true,
        "initializer": {"lifetime": "40", "size": "0.5", "custom": "4"}
    });
    if let Some(count) = area {
        rain["area"] = serde_json::json!({"size": [8, 8, 8], "count": count});
    }
    let path = "assets/test/polytone/custom_particles/rain.json";
    shader_pack(
        "rain",
        fsh,
        serde_json::json!({}),
        &[(path, rain.to_string())],
    )
}

/// A pack whose `test:rain` has no shader of its own and dies under the heightmap, drawn as
/// `render_type`, living 40 ticks; players emit it.
fn cpu_rain_pack(render_type: &str) -> sunrise_assets::resource_source::ResourceSource {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    let rain = serde_json::json!({
        "type": "gpu", "texture": "test:textures/particle/rain.png",
        "render_type": render_type, "kill_below_heightmap": true,
        "initializer": {"lifetime": "40", "size": "0.5"}
    });
    let emitter = serde_json::json!({"emitters": [{"particle": "test:rain"}]});
    ResourceSource::indexed(
        [
            ("assets/test/polytone/custom_particles/rain.json", rain),
            (
                "assets/minecraft/polytone/entity_modifiers/player.json",
                emitter,
            ),
        ]
        .into_iter()
        .map(|(path, json)| {
            (
                path.to_owned(),
                Resource::bytes(json.to_string().into_bytes()),
            )
        })
        .collect(),
    )
}

/// The rain's engine with one record from a player at (1, 64, 1).
fn raining(
    pack: &sunrise_assets::resource_source::ResourceSource,
    scene: &Scene,
) -> ParticleEngine {
    let mut engine = rain_engine(pack, blocks());
    engine.polytone_entity(&level(scene), "minecraft:player", [1.0, 64.0, 1.0]);
    assert_eq!(engine.polytone_gpu_records(), 1);
    engine
}

/// `HeightmapInfo` in a batch's block: the area's corner relative to the camera, its size and
/// bottom.
fn advertised(batch: &ShaderBatch) -> [f32; 4] {
    let float = |i: usize| f32::from_le_bytes(batch.block[i * 4..i * 4 + 4].try_into().unwrap());
    [float(24), float(25), float(26), float(27)]
}

const CAMERA: DVec3 = DVec3::new(1.0, 70.0, 1.0);

/// The switch to a frame of linear light: a shader that samples the heightmap stops drawing,
/// and its type, which doesn't die under the heightmap, draws its quads. Nothing reads the
/// heightmap then: it is neither read again nor handed to the renderer.
#[test]
fn a_shader_that_stops_drawing_gets_no_heightmap() {
    let scene = scene(blocks());
    let mut engine = raining(
        &shader_pack("rain", HEIGHT_FSH, serde_json::json!({}), &[]),
        &scene,
    );
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let mut renderer = Fake::default();
    let quads = frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(renderer.asked, [("test:rain".to_owned(), 1)]);
    assert_eq!((quads, renderer.batches.len()), (0, 1));
    assert_eq!((reads.get(), renderer.uploads.len()), (16, 1));
    for _ in 0..10 {
        engine.tick(&level(&scene));
    }
    renderer.refuses = true;
    let quads = frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!((quads, renderer.batches.len()), (1, 0));
    assert_eq!(reads.get(), 16, "not read again");
    assert_eq!(renderer.uploads.len(), 1, "not handed over");
}

/// A `kill_below_heightmap` type whose shader doesn't sample the heightmap reads none while the
/// shader draws; in the first frame the shader stops drawing, its quads die under heights read
/// in that frame.
#[test]
fn a_kill_type_whose_shader_stops_drawing_still_gets_heights() {
    let scene = scene(blocks());
    let uniforms = serde_json::json!({"Brightness": "1"});
    let pack = kill_shader_pack(BRIGHT_FSH, uniforms, "translucent", None);
    let mut engine = raining(&pack, &scene);
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let mut renderer = Fake::default();
    let quads = frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!((quads, renderer.batches.len()), (0, 1));
    assert_eq!((reads.get(), renderer.uploads.len()), (0, 0));
    renderer.refuses = true;
    let quads = frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 16, "read for its quads");
    assert_eq!(quads, 0, "under the ground");
    assert!(renderer.uploads.is_empty(), "no shader samples it");
}

/// Heights read for the quads alone (the shader refused) are not the texture's: when the
/// shader draws again three ticks later, inside the half second, the renderer gets those same
/// heights before the first frame that samples them, the area the batch's block advertises.
#[test]
fn a_sampler_reader_gets_the_heights_its_block_advertises_from_its_first_frame() {
    let scene = scene(blocks());
    let pack = kill_shader_pack(HEIGHT_FSH, serde_json::json!({}), "translucent", None);
    let mut engine = raining(&pack, &scene);
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let mut renderer = Fake {
        refuses: true,
        ..Fake::default()
    };
    assert_eq!(
        frame(
            &mut engine,
            Some(&mut renderer),
            CAMERA,
            &ground(&top, &reads)
        ),
        0
    );
    assert_eq!((reads.get(), renderer.uploads.len()), (16, 0));
    for _ in 0..3 {
        engine.tick(&level(&scene));
    }
    renderer.refuses = false;
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 16, "still fresh");
    let [upload] = &renderer.uploads[..] else {
        panic!("{} uploads", renderer.uploads.len());
    };
    let area = HeightmapArea {
        origin: [-1, -1],
        size: 4,
        min_y: 0,
    };
    assert_eq!(upload.area(), area);
    assert_eq!(upload.heights()[..], [100; 16]);
    assert_eq!(advertised(&renderer.batches[0]), [-2.0, -2.0, 4.0, 0.0]);
}

/// A renderer made after the heights were read (a new window, another output) has an unfilled
/// texture: it gets the heights before its first frame, though they are still fresh.
#[test]
fn every_new_renderer_gets_the_heights_before_its_first_frame() {
    let scene = scene(blocks());
    let pack = shader_pack("rain", HEIGHT_FSH, serde_json::json!({}), &[]);
    let mut engine = raining(&pack, &scene);
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let camera = DVec3::new(10.5, 70.0, -3.5);
    let mut first = Fake::default();
    frame(&mut engine, Some(&mut first), camera, &ground(&top, &reads));
    assert_eq!(first.uploads[0].area().origin, [8, -6]);
    let mut replaced = Fake::default();
    frame(
        &mut engine,
        Some(&mut replaced),
        camera,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 16, "read once");
    assert_eq!(replaced.uploads.len(), 1);
    assert_eq!(replaced.uploads[0].area(), first.uploads[0].area());
}

/// Rain with no shader of its own, steady: its heights are read every half second (10 ticks),
/// at once for another world's bottom, and never handed to the renderer; its quads die under
/// the last heights read. Once its records are over nothing reads them.
#[test]
fn rain_drawn_as_quads_reads_the_heightmap_every_half_second() {
    let scene = scene(blocks());
    let mut engine = raining(&cpu_rain_pack("translucent"), &scene);
    let (top, reads) = (Cell::new(0), Cell::new(0));
    let mut renderer = Fake::default();
    let mut drawn = Vec::new();
    for tick in 0..=20 {
        drawn.push(frame(
            &mut engine,
            Some(&mut renderer),
            CAMERA,
            &ground(&top, &reads),
        ));
        assert_eq!(reads.get(), 16 * (tick / 10 + 1), "tick {tick}");
        // The ground rises after the first frame's heights.
        top.set(100);
        engine.tick(&level(&scene));
    }
    assert_eq!(drawn[..10], [1; 10], "above the heights read on tick 0");
    assert_eq!(drawn[10..], [0; 11], "under the heights read on tick 10");
    assert!(renderer.asked.is_empty() && renderer.uploads.is_empty());
    let source = HeightmapSource {
        size: 4,
        min_y: -64,
        top: &ground(&top, &reads),
    };
    frame_of(&mut engine, Some(&mut renderer), CAMERA, &source);
    assert_eq!(reads.get(), 64, "another world's bottom");
    for _ in 0..20 {
        engine.tick(&level(&scene));
    }
    assert_eq!(engine.polytone_gpu_records(), 0);
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 64, "no live rain");
}

/// Invisible records draw nothing: they read no heightmap, and a type with a shader is not
/// put to the renderer.
#[test]
fn invisible_records_read_no_heightmap() {
    let scene = scene(blocks());
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let mut renderer = Fake::default();
    let mut quads = raining(&cpu_rain_pack("invisible"), &scene);
    frame(
        &mut quads,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    frame(&mut quads, None, CAMERA, &ground(&top, &reads));
    let pack = kill_shader_pack(HEIGHT_FSH, serde_json::json!({}), "invisible", None);
    let mut shaded = raining(&pack, &scene);
    frame(
        &mut shaded,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 0);
    assert!(renderer.asked.is_empty() && renderer.uploads.is_empty());
    // The same visible: read.
    let mut visible = raining(&cpu_rain_pack("translucent"), &scene);
    frame(&mut visible, None, CAMERA, &ground(&top, &reads));
    assert_eq!(reads.get(), 16);
}

/// A type asks the renderer for room for every quad it may hand over: each record times its
/// `area`'s copies.
#[test]
fn a_request_counts_each_copy_of_an_area() {
    let scene = scene(blocks());
    let pack = kill_shader_pack(HEIGHT_FSH, serde_json::json!({}), "translucent", Some(3));
    let mut engine = raining(&pack, &scene);
    let mut renderer = Fake::default();
    frame(&mut engine, Some(&mut renderer), CAMERA, &|_, _| 0);
    assert_eq!(renderer.asked, [("test:rain".to_owned(), 3)]);
    assert_eq!(renderer.batches[0].vertices.len(), 3 * 4);
}

/// Without a renderer every type draws its quads, its shader's verdict given in that frame.
#[test]
fn without_a_renderer_every_type_draws_its_quads() {
    let scene = scene(blocks());
    let pack = shader_pack("rain", HEIGHT_FSH, serde_json::json!({}), &[]);
    let mut engine = raining(&pack, &scene);
    let mut renderer = Fake::default();
    assert_eq!(
        frame(&mut engine, Some(&mut renderer), CAMERA, &|_, _| 0),
        0
    );
    assert_eq!(frame(&mut engine, None, CAMERA, &|_, _| 0), 1);
}

/// Clearing the particles (a mod's clear, a level left or switched) forgets the GPU records
/// and the heightmap too, even for a level with the same bottom: the next rain reads it at once.
#[test]
fn clearing_forgets_the_gpu_records_and_the_heightmap() {
    let scene = scene(blocks());
    let pack = shader_pack("rain", HEIGHT_FSH, serde_json::json!({}), &[]);
    let mut engine = raining(&pack, &scene);
    let (top, reads) = (Cell::new(100), Cell::new(0));
    let mut renderer = Fake::default();
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    engine.clear();
    assert_eq!(engine.polytone_gpu_records(), 0);
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert!(renderer.asked.is_empty(), "no live type");
    engine.polytone_entity(&level(&scene), "minecraft:player", [1.0, 64.0, 1.0]);
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 32, "read again at once for the new level");
}

/// A pack whose players emit each of `types` (name, texture, `area` copies): GPU types with no
/// shader of their own that die under the heightmap, living 40 ticks.
fn cpu_types_pack(types: &[(&str, &str, u32)]) -> sunrise_assets::resource_source::ResourceSource {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    let mut files: Vec<(String, serde_json::Value)> = types
        .iter()
        .map(|(name, texture, count)| {
            let json = serde_json::json!({
                "type": "gpu", "texture": format!("test:textures/particle/{texture}.png"),
                "render_type": "translucent", "kill_below_heightmap": true,
                "area": {"size": [8, 8, 8], "count": count},
                "initializer": {"lifetime": "40", "size": "0.5"}
            });
            (
                format!("assets/test/polytone/custom_particles/{name}.json"),
                json,
            )
        })
        .collect();
    let emitters: Vec<serde_json::Value> = types
        .iter()
        .map(|(name, _, _)| serde_json::json!({"particle": format!("test:{name}")}))
        .collect();
    files.push((
        "assets/minecraft/polytone/entity_modifiers/player.json".to_owned(),
        serde_json::json!({ "emitters": emitters }),
    ));
    ResourceSource::indexed(
        files
            .into_iter()
            .map(|(path, json)| (path, Resource::bytes(json.to_string().into_bytes())))
            .collect(),
    )
}

/// The quads drawn on the CPU have a room of their own, shared in the types' order: a type
/// whose quads (each record times its copies) don't fit what the types before it left draws
/// none and reads no heightmap, and the next frame with room draws it again.
#[test]
fn the_quads_drawn_on_the_cpu_fit_the_room_the_renderer_has() {
    let scene = scene(blocks());
    let pack = cpu_types_pack(&[("rain", "rain", 2), ("snow", "rain", 2)]);
    let mut engine = rain_engine(&pack, blocks());
    engine.polytone_entity(&level(&scene), "minecraft:player", [1.0, 64.0, 1.0]);
    assert_eq!(engine.polytone_gpu_records(), 2);
    let mut renderer = Fake {
        room: Some(3),
        ..Fake::default()
    };
    assert_eq!(
        frame(&mut engine, Some(&mut renderer), CAMERA, &|_, _| 0),
        2,
        "the first type"
    );
    renderer.room = Some(4);
    assert_eq!(
        frame(&mut engine, Some(&mut renderer), CAMERA, &|_, _| 0),
        4,
        "both"
    );
    let (top, reads) = (Cell::new(0), Cell::new(0));
    renderer.room = Some(1);
    assert_eq!(
        frame(
            &mut engine,
            Some(&mut renderer),
            CAMERA,
            &ground(&top, &reads)
        ),
        0
    );
    assert_eq!(reads.get(), 0, "no quads drawn: no heights read for them");
}

/// Without a renderer the quads drawn on the CPU are still bounded: a type over
/// `FALLBACK_QUADS` (257 records of 1024 copies) draws none.
#[test]
fn without_a_renderer_the_quads_are_bounded_too() {
    let scene = scene(blocks());
    let pack = cpu_types_pack(&[("rain", "rain", 1024)]);
    let mut engine = rain_engine(&pack, blocks());
    for _ in 0..257 {
        engine.polytone_entity(&level(&scene), "minecraft:player", [1.0, 64.0, 1.0]);
    }
    assert_eq!(engine.polytone_gpu_records(), 257);
    const { assert!(257 * 1024 > FALLBACK_QUADS) };
    assert_eq!(frame(&mut engine, None, CAMERA, &|_, _| 0), 0);
}

/// A type drawn on the CPU whose sprite isn't in the atlas draws nothing: it reads no heightmap
/// and takes no room.
#[test]
fn a_type_without_its_sprite_reads_no_heightmap() {
    let scene = scene(blocks());
    let pack = cpu_types_pack(&[("ghost", "missing", 1), ("rain", "rain", 1)]);
    let mut engine = rain_engine(&pack, blocks());
    engine.polytone_entity(&level(&scene), "minecraft:player", [1.0, 64.0, 1.0]);
    let (top, reads) = (Cell::new(0), Cell::new(0));
    let mut renderer = Fake {
        room: Some(1),
        ..Fake::default()
    };
    assert_eq!(
        frame(
            &mut engine,
            Some(&mut renderer),
            CAMERA,
            &ground(&top, &reads)
        ),
        1
    );
    assert_eq!(reads.get(), 16, "for the rain");
    let ghosts = cpu_types_pack(&[("ghost", "missing", 1)]);
    let mut engine = rain_engine(&ghosts, blocks());
    engine.polytone_entity(&level(&scene), "minecraft:player", [1.0, 64.0, 1.0]);
    frame(
        &mut engine,
        Some(&mut renderer),
        CAMERA,
        &ground(&top, &reads),
    );
    assert_eq!(reads.get(), 16, "none for the ghost");
}

/// A camera at the far end of the floats reads a heightmap whose every column is a block
/// coordinate: the area's far side stops at the largest one, its near side at the smallest.
#[test]
fn the_heightmap_stays_in_block_coordinates_at_any_camera() {
    let scene = scene(blocks());
    let mut engine = raining(&cpu_rain_pack("translucent"), &scene);
    let columns = std::cell::RefCell::new(Vec::new());
    let top = |x: i32, z: i32| {
        columns.borrow_mut().push((x, z));
        0
    };
    let source = HeightmapSource {
        size: 4,
        min_y: 0,
        top: &top,
    };
    frame_of(
        &mut engine,
        None,
        DVec3::new(f64::MAX, 70.0, -1.0e300),
        &source,
    );
    let columns = columns.borrow();
    assert_eq!(columns.len(), 16);
    assert_eq!(columns[0], (i32::MAX - 3, i32::MIN));
    assert_eq!(columns[15], (i32::MAX, i32::MIN + 3));
}
