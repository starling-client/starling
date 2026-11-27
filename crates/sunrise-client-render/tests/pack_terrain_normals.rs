//! A `shaders/` pack's terrain normal and tangent (`gl_Normal`, `at_tangent`, spec §5.1)
//! through the real `Renderer`, with flat section draws on and off (`set_flat_draws`): a pack
//! that writes one of them as its colour, inside a box of six quads, one facing the camera from
//! each direction, solid (`gbuffers_terrain`) and translucent (`gbuffers_water`). Each face must
//! read its own normal and tangent in both draw paths (#432), per-section draws reading their
//! page or, where they read none (no storage in vertex shaders, or the pack's buffers leave no
//! room), the normal stream. Asset-free; run with throwaway SUNRISE_HOME,
//! SUNRISE_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md):
//! `cargo test -p sunrise-client-render --test pack_terrain_normals -- --ignored`
//! (under `python3 tools/ci/with_xvfb.py` without a display) gives each its own. On a Wayland
//! desktop, unset `WAYLAND_DISPLAY` as `with_xvfb.py` does: the child's own `XDG_RUNTIME_DIR`
//! has no Wayland socket.
use client::{
    gfx::{Frame, Renderer, ScreenshotRequest},
    graphics::GraphicsApi,
    hud::HudBatch,
    mesher::{SectionMesh, TerrainVertex},
};
use std::{path::Path, sync::Arc};
use sunrise_client_render as client;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

#[test]
#[ignore = "daily"]
fn pack_terrain_normals() {
    run_daily("pack_terrain_normals", &[], fixture);
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_without_flat_draws() {
    run_daily(
        "pack_terrain_normals_without_flat_draws",
        &[("SUNRISE_FLAT_DRAWS", "0")],
        fixture_without_flat,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_without_vertex_storage() {
    run_daily(
        "pack_terrain_normals_without_vertex_storage",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_without_vertex_storage,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_stream_from_start() {
    run_daily(
        "pack_terrain_normals_stream_from_start",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_pending,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_flat_from_start() {
    run_daily("pack_terrain_normals_flat_from_start", &[], fixture_pending);
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_pending_packed() {
    run_daily(
        "pack_terrain_normals_pending_packed",
        &[],
        fixture_pending_packed,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_pending_post_only() {
    run_daily(
        "pack_terrain_normals_pending_post_only",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_pending_post_only,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_water_only() {
    run_daily(
        "pack_terrain_normals_water_only",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_water_only,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_pending_refused() {
    run_daily(
        "pack_terrain_normals_pending_refused",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_pending_refused,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_pages_from_start() {
    run_daily(
        "pack_terrain_normals_pages_from_start",
        &[("SUNRISE_FLAT_DRAWS", "0")],
        fixture_pending,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_stream_in_the_shadow_pass() {
    run_daily(
        "pack_terrain_normals_stream_in_the_shadow_pass",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_shadow,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_stream_after_flat_draws() {
    run_daily(
        "pack_terrain_normals_stream_after_flat_draws",
        &[],
        fixture_stream_after_flat,
    );
}

#[test]
#[ignore = "daily"]
fn pack_terrain_normals_stream_where_pages_dont_fit() {
    run_daily(
        "pack_terrain_normals_stream_where_pages_dont_fit",
        &[("SUNRISE_FLAT_DRAWS", "0")],
        fixture_pages_dont_fit,
    );
}

/// The section the box is in: keys count sections from the bottom of the world, y -64, so 10
/// is y 96 to 112.
const SECTION: (i32, i32, i32) = (0, 10, 0);
/// The camera, at the middle of the section.
const EYE: [f64; 3] = [8.0, 104.0, 8.0];

/// The pack's colour for a direction's component: -1, 0 and 1 as 51, 153 and 255 of 255 (0.2,
/// 0.6 and 1.0, each a whole number of 255ths).
fn channel(n: i32) -> u8 {
    match n {
        -1 => 51,
        0 => 153,
        1 => 255,
        _ => unreachable!(),
    }
}

/// Writes a pack whose solid and translucent terrain write `input` (a direction), as
/// `n * 0.4 + 0.6`, into `colortex0`, and loads it.
fn pack(renderer: &mut Renderer, folder: &Path, input: &str) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for name in ["gbuffers_terrain", "gbuffers_water"] {
        std::fs::write(
            shaders.join(format!("{name}.vsh")),
            format!(
                "#version 120\nattribute vec4 at_tangent;\nvarying vec3 n;\n\
                 void main() {{ gl_Position = ftransform(); n = {input}; }}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            shaders.join(format!("{name}.fsh")),
            "#version 120\nvarying vec3 n;\n\
             void main() { gl_FragData[0] = vec4(n * 0.4 + 0.6, 1.0); }\n",
        )
        .unwrap();
    }
    std::fs::write(shaders.join("shaders.properties"), "shadow.enabled=false\n").unwrap();
    let post = read_pack(renderer, folder);
    assert!(post.terrain.is_some(), "gbuffers_terrain loads");
    assert!(post.water.is_some(), "gbuffers_water loads");
    renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap()
        .expect("the pack runs");
}

/// The pack in `folder`, read as the game reads it for this renderer (`pack_inputs`).
fn read_pack(renderer: &mut Renderer, folder: &Path) -> sunrise_shaders::Post {
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs())
}

/// The quad facing `normal` (an axis, `axis`, and its sign) from the side of the box opposite
/// to it, 4 blocks from the camera and 12 across: its corners in the order that makes
/// `cross(p1 - p0, p2 - p0)` point along `normal`.
fn face(axis: usize, sign: i32) -> [[f32; 3]; 4] {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let corner = |du: f32, dv: f32| {
        let mut p = [0.0; 3];
        p[axis] = 8.0 - 4.0 * sign as f32;
        p[u] = du;
        p[v] = dv;
        p
    };
    // Along u then v: the axis after them, as `e_u x e_v = e_axis` for a cyclic order.
    let up = [
        corner(2.0, 2.0),
        corner(14.0, 2.0),
        corner(14.0, 14.0),
        corner(2.0, 14.0),
    ];
    if sign > 0 {
        up
    } else {
        [up[3], up[2], up[1], up[0]]
    }
}

/// The box's six quads, all in the solid list or all in the translucent one.
fn mesh(translucent: bool) -> SectionMesh {
    let mut mesh = SectionMesh::default();
    let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    for axis in 0..3 {
        for sign in [1, -1] {
            let quad = face(axis, sign)
                .into_iter()
                .zip(uvs)
                .map(|(p, uv)| TerrainVertex::new(p, uv, 0, u32::MAX, 0xf0f0));
            if translucent {
                mesh.translucent.extend(quad);
            } else {
                mesh.solid.extend(quad);
            }
        }
    }
    mesh.sort_solid();
    mesh
}

/// Each view: its yaw and pitch, and the normal and tangent of the face it looks at. The face
/// across the box faces back toward the camera; its tangent is where its U grows (`mesh`'s
/// UVs): along the axis after its normal's (`face`'s `u`), whichever way the quad faces. Yaw 0
/// looks south (+z) and 90 west (-x); pitch 90 looks down.
const VIEWS: [(&str, f32, f32, [i32; 3], [i32; 3]); 6] = [
    ("south", 0.0, 0.0, [0, 0, -1], [1, 0, 0]),
    ("west", 90.0, 0.0, [1, 0, 0], [0, 1, 0]),
    ("north", 180.0, 0.0, [0, 0, 1], [1, 0, 0]),
    ("east", 270.0, 0.0, [-1, 0, 0], [0, 1, 0]),
    ("down", 0.0, 90.0, [0, 1, 0], [0, 0, 1]),
    ("up", 0.0, -90.0, [0, -1, 0], [0, 0, 1]),
];

fn shoot(renderer: &mut Renderer, hud: &HudBatch, yaw: f32, pitch: f32, path: &Path) -> [u8; 4] {
    let mut frame = fixture_frame(hud);
    frame.yaw = yaw;
    frame.pitch = pitch;
    frame.eye_rotation = [yaw, pitch];
    renderer.screenshot = Some(ScreenshotRequest {
        path: path.to_path_buf(),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, None).unwrap();
    renderer
        .screenshot_result
        .take()
        .expect("readback")
        .unwrap()
        .save()
        .unwrap();
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let (width, height) = (info.width as usize, info.height as usize);
    let at = 4 * ((height / 2) * width + width / 2);
    bytes[at..at + 4].try_into().unwrap()
}

/// A renderer of 320 by 180 with Translucent Batching on (translucent quads get an arena of their
/// own unless the pack's draws read the section arena's pages); `pack` as `Renderer::new`
/// takes the picked pack (it draws flat, it is a `shaders/` pack).
fn renderer(
    window: Arc<Window>,
    pack: (bool, bool),
) -> Result<Renderer, Box<dyn std::error::Error>> {
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        16,
        16,
        1,
        &[],
        &[],
        (&white, 16, 16),
        None,
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        client::gfx::RendererDevice::New,
        GraphicsApi::Vulkan,
        true,
        pack,
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    renderer.resize(320, 180);
    Ok(renderer)
}

/// The box's six views with the pack writing `written` (`normal` or `tangent`), its quads in
/// the `translucent` list or the solid one: each view's pixel against `want` of the view's normal
/// and tangent. A first frame lays the sections out for the pack (which may drop them), then the
/// box is uploaded into that layout.
fn check_views(
    renderer: &mut Renderer,
    hud: &HudBatch,
    out: &Path,
    name: &str,
    translucent: bool,
    want: fn([i32; 3], [i32; 3]) -> [i32; 3],
    result: &mut serde_json::Map<String, serde_json::Value>,
) {
    let list = if translucent { "translucent" } else { "solid" };
    shoot(renderer, hud, 0.0, 0.0, &out.join("settle.png"));
    renderer.take_dropped_sections();
    renderer.upload(SECTION, &mesh(translucent));
    for (view, yaw, pitch, normal, tangent) in VIEWS {
        let name = format!("{name}-{list}-{view}");
        let pixel = shoot(renderer, hud, yaw, pitch, &out.join(format!("{name}.png")));
        let want = want(normal, tangent);
        let expected = [channel(want[0]), channel(want[1]), channel(want[2]), 255];
        assert_eq!(pixel, expected, "{name}: the face's {want:?}");
        result.insert(name, format!("{pixel:?}").into());
    }
}

fn normal(normal: [i32; 3], _: [i32; 3]) -> [i32; 3] {
    normal
}
fn tangent(_: [i32; 3], tangent: [i32; 3]) -> [i32; 3] {
    tangent
}
/// The constant normal per-section draws give where they don't read the pages.
fn constant(_: [i32; 3], _: [i32; 3]) -> [i32; 3] {
    [0, 1, 0]
}

/// Flat draws on and off on a device that has them, the pack loaded after each switch.
fn fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    for flat in [true, false] {
        renderer.set_flat_draws(flat)?;
        let draws = if flat { "flat" } else { "per-section" };
        for (written, input, want) in [
            ("normal", "gl_Normal", normal as fn(_, _) -> _),
            ("tangent", "at_tangent.xyz", tangent),
        ] {
            pack(
                &mut renderer,
                &out.join(format!("pack-{draws}-{written}")),
                input,
            );
            for translucent in [false, true] {
                let name = format!("{draws}-{written}");
                check_views(
                    &mut renderer,
                    &hud,
                    out,
                    &name,
                    translucent,
                    want,
                    &mut result,
                );
            }
            assert!(
                !renderer.section_arena_normals().0,
                "{draws}: no normal stream where the pages are read"
            );
        }
    }
    finish(out, result)
}

/// Without flat draws (`SUNRISE_FLAT_DRAWS=0`) and with no pack picked: the section arena's
/// pages start as vertex buffers alone. A pack loaded after the renderer makes them storage for
/// its per-section terrain programs (`follow_arena_layout`), which then read the real normals;
/// unloading it makes them vertex buffers again, as large as before. A pack whose buffer
/// objects leave no room in the vertex stage for the pages still loads its terrain programs, and
/// they get the constant normal, the pages staying vertex buffers.
fn fixture_without_flat(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    assert!(
        !renderer.flat_draws(),
        "SUNRISE_FLAT_DRAWS=0 leaves out flat draws"
    );
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("start.png"));
    let (storage, vertices) = renderer.section_arena_pages();
    assert!(!storage, "no pack: the pages are vertex buffers alone");
    pack(&mut renderer, &out.join("pack-normal"), "gl_Normal");
    for translucent in [false, true] {
        check_views(
            &mut renderer,
            &hud,
            out,
            "loaded",
            translucent,
            normal,
            &mut result,
        );
    }
    assert!(
        renderer.section_arena_pages().0,
        "the pack reads the pages: storage"
    );
    renderer.set_shader_pack(0, None, &[]).unwrap();
    shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("unloaded.png"));
    assert_eq!(
        renderer.section_arena_pages(),
        (false, vertices),
        "unloaded: vertex buffers alone again, of the pages' first size"
    );
    // Past any device's storage buffers: the terrain programs read no pages.
    renderer.set_pack_terrain_bindings(client::gfx::pack_atlas::TerrainBindings {
        buffer_objects: u32::MAX,
    });
    pack(&mut renderer, &out.join("pack-crowded"), "gl_Normal");
    for translucent in [false, true] {
        check_views(
            &mut renderer,
            &hud,
            out,
            "crowded",
            translucent,
            constant,
            &mut result,
        );
    }
    assert_eq!(renderer.section_arena_pages(), (false, vertices));
    finish(out, result)
}

/// Bytes of a section arena page as the renderer first makes it: 1 << 20 vertices of 24 bytes
/// (`TerrainVertex`), with 16 bytes more a vertex (`pack_atlas::NORMAL_BYTES`) where it has a
/// normal stream.
const FIRST_PAGE: u64 = 25_165_824;
const FIRST_PAGE_WITH_NORMALS: u64 = 41_943_040;

/// Uploads the solid box on a settled frame, for the arenas' bytes.
/// The bytes the upload copied (`Renderer::uploaded_bytes`, which counts toward the section
/// copies' submission threshold as it goes): the box's 24 vertices of 24 bytes, and 16 bytes of
/// normal stream each where the arena keeps one.
fn upload_box(renderer: &mut Renderer, hud: &HudBatch, out: &Path, name: &str) -> u64 {
    shoot(renderer, hud, 0.0, 0.0, &out.join(format!("{name}.png")));
    renderer.take_dropped_sections();
    let before = renderer.uploaded_bytes;
    renderer.upload(SECTION, &mesh(false));
    renderer.uploaded_bytes - before
}

/// Without flat draws (`SUNRISE_FLAT_DRAWS=0`), on a device taken to read no storage in vertex
/// shaders (`SUNRISE_SHADER_PACK_WITHOUT=vertex-storage`): per-section draws read no pages, and
/// a pack's terrain programs read their quad's normal and tangent from the normal stream, which
/// the arenas (the translucent one too, with Translucent Batching) have exactly while such a
/// pack runs. With no pack the section arena is as it was before the stream: vertex buffers
/// alone, of the same page size and bytes.
fn fixture_without_vertex_storage(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    assert!(!renderer.flat_draws(), "SUNRISE_FLAT_DRAWS=0");
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    // No pack: vertex buffers alone, pages of 256 MiB of 24-byte vertices in whole blocks of
    // 256 or the device's buffers, as before the stream.
    assert_eq!(upload_box(&mut renderer, &hud, out, "start"), 576);
    let limits = renderer.device.limits();
    let page_vertices = (limits.max_buffer_size.min(256 << 20) / 24) / 256 * 256;
    assert_eq!(renderer.section_arena_pages(), (false, page_vertices));
    assert_eq!(
        renderer.section_arena_normals(),
        (false, false, FIRST_PAGE),
        "no pack: no normal stream"
    );
    for (written, input, want) in [
        ("normal", "gl_Normal", normal as fn(_, _) -> _),
        ("tangent", "at_tangent.xyz", tangent),
    ] {
        pack(&mut renderer, &out.join(format!("pack-{written}")), input);
        for translucent in [false, true] {
            check_views(
                &mut renderer,
                &hud,
                out,
                &format!("stream-{written}"),
                translucent,
                want,
                &mut result,
            );
        }
        let (section, translucent, _) = renderer.section_arena_normals();
        assert!(section && translucent, "the pack reads the stream");
        assert_eq!(renderer.section_arena_pages(), (false, page_vertices));
    }
    assert_eq!(upload_box(&mut renderer, &hud, out, "loaded"), 960);
    assert_eq!(
        renderer.section_arena_normals(),
        (true, true, FIRST_PAGE_WITH_NORMALS)
    );
    renderer.set_shader_pack(0, None, &[]).unwrap();
    assert_eq!(upload_box(&mut renderer, &hud, out, "unloaded"), 576);
    assert_eq!(
        renderer.section_arena_normals(),
        (false, false, FIRST_PAGE),
        "unloaded: the stream is dropped"
    );
    assert_eq!(renderer.section_arena_pages(), (false, page_vertices));
    finish(out, result)
}

/// Without flat draws, on a device whose vertex shaders read and write storage: a pack whose
/// buffer objects leave the vertex stage room for one buffer more than its per-section draws
/// take without normals (2 uniform and 2 vertex buffers), and not for set 3's three storage
/// buffers, reads the normal stream; one buffer object more and it reads neither, keeping the
/// constant normal.
fn fixture_pages_dont_fit(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    assert!(!renderer.flat_draws(), "SUNRISE_FLAT_DRAWS=0");
    let room = room_for_the_stream(&renderer);
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    for (name, buffer_objects, want, stream) in [
        ("room", room, normal as fn(_, _) -> _, true),
        ("full", room + 1, constant, false),
    ] {
        renderer
            .set_pack_terrain_bindings(client::gfx::pack_atlas::TerrainBindings { buffer_objects });
        pack(
            &mut renderer,
            &out.join(format!("pack-{name}")),
            "gl_Normal",
        );
        for translucent in [false, true] {
            check_views(
                &mut renderer,
                &hud,
                out,
                name,
                translucent,
                want,
                &mut result,
            );
        }
        assert_eq!(renderer.section_arena_normals().0, stream, "{name}");
        assert!(!renderer.section_arena_pages().0, "{name}: no pages read");
    }
    finish(out, result)
}

/// Buffer objects that leave the vertex stage room for the normal stream and none for set 3:
/// the device's own limit of buffers in a stage, less the 2 uniform buffers, the 3 vertex
/// buffers of a draw with the stream, and 0 to spare (`fixture_pages_dont_fit`).
fn room_for_the_stream(renderer: &Renderer) -> u32 {
    assert!(
        renderer
            .device
            .features()
            .contains(wgpu::Features::VERTEX_WRITABLE_STORAGE),
        "the pack's buffer objects are in the vertex stage"
    );
    let buffers = renderer
        .device
        .limits()
        .max_buffers_and_acceleration_structures_per_shader_stage;
    buffers - 5
}

/// The layouts the sections are in: the section arena's pages (storage, page size), its
/// normal stream, the translucent arena's stream, whether translucent quads have that arena
/// (`translucent_split`), and whether solid quads take the compact layout (`packed_layout`).
type Layout = ((bool, u64), bool, bool, bool, bool);

fn layout(renderer: &Renderer) -> Layout {
    let (normals, translucent, _) = renderer.section_arena_normals();
    (
        renderer.section_arena_pages(),
        normals,
        translucent,
        renderer.translucent_split(),
        renderer.packed_layout()[0],
    )
}

/// Writes a pack in `folder` whose solid and translucent terrain write `gl_Normal` as
/// `pack` writes it, and `extra` (name, source) files beside them.
fn write_normal_pack(folder: &Path, extra: &[(&str, &str)]) -> std::io::Result<()> {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders)?;
    for name in ["gbuffers_terrain", "gbuffers_water"] {
        std::fs::write(
            shaders.join(format!("{name}.vsh")),
            "#version 120\nvarying vec3 n;\nvoid main() { gl_Position = ftransform(); n = gl_Normal; }\n",
        )?;
        std::fs::write(
            shaders.join(format!("{name}.fsh")),
            "#version 120\nvarying vec3 n;\nvoid main() { gl_FragData[0] = vec4(n * 0.4 + 0.6, 1.0); }\n",
        )?;
    }
    std::fs::write(shaders.join("shaders.properties"), "shadow.enabled=false\n")?;
    for (name, source) in extra {
        std::fs::write(shaders.join(name), source)?;
    }
    Ok(())
}

/// Polls the pack `load_shader_pack` builds until it runs (whether it does), or is refused (why).
fn poll(renderer: &mut Renderer) -> Result<bool, String> {
    loop {
        if let Some((_, _, _, report)) = renderer.poll_shader_pack() {
            return report.map(|r| r.is_some()).map_err(|e| e.reason);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Draws `frames` frames, each leaving every section where it was (`take_dropped_sections`)
/// and the layouts as `want`.
fn frames_keep(
    renderer: &mut Renderer,
    hud: &HudBatch,
    out: &Path,
    name: &str,
    frames: usize,
    want: Layout,
) {
    for frame in 0..frames {
        shoot(
            renderer,
            hud,
            0.0,
            0.0,
            &out.join(format!("{name}-{frame}.png")),
        );
        assert!(
            !renderer.take_dropped_sections(),
            "{name} frame {frame}: the sections stay"
        );
        assert_eq!(
            layout(renderer),
            want,
            "{name} frame {frame}: the layout stays"
        );
    }
}

/// A renderer made with a `shaders/` pack picked (`Renderer::new`'s `pack_starts_wide`)
/// starts in the layout that pack's terrain draws in: flat (pages storage, no translucent
/// arena) where flat draws are on; per section, the stream in both arenas on a device taken to
/// read no storage in vertex shaders (`SUNRISE_SHADER_PACK_WITHOUT=vertex-storage`), else the
/// pages storage. The game loads the pack on a thread of its own (`load_shader_pack`) and draws
/// without it until `poll_shader_pack` runs it: the frames drawn meanwhile keep that layout, so
/// no section is dropped, neither then nor once the pack runs, and both lists draw with the pack
/// at once.
fn fixture_pending(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    let flat = renderer.flat_draws();
    let stream = std::env::var("SUNRISE_SHADER_PACK_WITHOUT").as_deref() == Ok("vertex-storage");
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    let start = layout(&renderer);
    // Storage pages flat or per section, translucent quads in the section arena that pages
    // are read from; else the stream in both arenas, translucent quads in an arena of their own.
    assert_eq!(
        (start.0.0, start.1, start.2, start.3),
        (!stream, stream, stream, stream)
    );
    renderer.upload(SECTION, &mesh(false));
    let folder = out.join("pack-normal");
    write_normal_pack(&folder, &[])?;
    let post = read_pack(&mut renderer, &folder);
    renderer.load_shader_pack(
        0,
        post,
        Vec::new(),
        "normal".into(),
        std::time::Instant::now(),
    );
    // A frame runs the compact layout as the pack will (`follow_packed_layout`): flat draws
    // take it, per-section ones don't. Then frames while it builds, before the game polls it.
    let want = (start.0, start.1, start.2, start.3, flat);
    frames_keep(&mut renderer, &hud, out, "pending", 3, want);
    assert!(poll(&mut renderer).unwrap(), "the pack runs");
    let south = [channel(0), channel(0), channel(-1), 255];
    for translucent in [false, true] {
        let name = if translucent { "translucent" } else { "solid" };
        if translucent {
            renderer.upload(SECTION, &mesh(true));
        }
        let pixel = shoot(
            &mut renderer,
            &hud,
            0.0,
            0.0,
            &out.join(format!("{name}.png")),
        );
        assert_eq!(
            pixel, south,
            "{name}: the south view's face reads [0, 0, -1]"
        );
        assert!(
            !renderer.take_dropped_sections(),
            "{name}: running the pack kept the sections"
        );
        assert_eq!(layout(&renderer), want, "{name}: the layout stays");
        result.insert(name.into(), format!("{pixel:?}").into());
    }
    finish(out, result)
}

/// On a device with flat draws, turned off (`set_flat_draws(false)`), and no pack picked:
/// solid quads start in the compact layout, translucent ones in an arena of their own. A pack
/// loaded meanwhile draws terrain per section from the pages, which reads no compact records
/// and holds translucent quads in the section arena: the first frame drawn while it builds lays
/// the sections out so (none uploaded yet), the sections uploaded then are laid out as the pack
/// reads them, and none is dropped once it runs.
fn fixture_pending_packed(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    renderer.set_flat_draws(false)?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("start.png"));
    renderer.take_dropped_sections();
    let start = layout(&renderer);
    assert!(start.4, "no pack: solid quads compact");
    let folder = out.join("pack-normal");
    write_normal_pack(&folder, &[])?;
    let post = read_pack(&mut renderer, &folder);
    renderer.load_shader_pack(
        0,
        post,
        Vec::new(),
        "normal".into(),
        std::time::Instant::now(),
    );
    // Per section from the pages, which the section arena holds the translucent quads beside.
    let want = (start.0, false, false, false, false);
    frames_keep(&mut renderer, &hud, out, "pending", 1, want);
    // Uploaded while the pack builds.
    renderer.upload(SECTION, &mesh(false));
    frames_keep(&mut renderer, &hud, out, "pending-uploaded", 2, want);
    assert!(poll(&mut renderer).unwrap(), "the pack runs");
    frames_keep(&mut renderer, &hud, out, "running", 1, want);
    let pixel = shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("solid.png"));
    assert_eq!(pixel, [channel(0), channel(0), channel(-1), 255]);
    result.insert("solid".into(), format!("{pixel:?}").into());
    finish(out, result)
}

/// A pack whose programs draw no terrain and no shadow pass (a `composite` alone), loaded
/// mid-session where per-section terrain would read the normal stream: the frames while it
/// builds, and once it runs, keep the layout without a pack's terrain; no section is dropped.
fn fixture_pending_post_only(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    let hud = HudBatch::scratch([320, 180]);
    shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("start.png"));
    renderer.upload(SECTION, &mesh(false));
    renderer.take_dropped_sections();
    let start = layout(&renderer);
    assert!(!start.1, "no pack: no stream");
    let folder = out.join("pack-composite");
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders)?;
    std::fs::write(
        shaders.join("composite.vsh"),
        "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
    )?;
    std::fs::write(
        shaders.join("composite.fsh"),
        "#version 120\nvoid main() { gl_FragData[0] = vec4(0.2, 0.6, 1.0, 1.0); }\n",
    )?;
    let post = read_pack(&mut renderer, &folder);
    assert!(post.terrain.is_none() && post.shadow.is_none());
    renderer.load_shader_pack(
        0,
        post,
        Vec::new(),
        "composite".into(),
        std::time::Instant::now(),
    );
    frames_keep(&mut renderer, &hud, out, "pending", 3, start);
    assert!(poll(&mut renderer).unwrap(), "the pack runs");
    frames_keep(&mut renderer, &hud, out, "running", 2, start);
    finish(out, serde_json::Map::new())
}

/// A pack whose only program drawing the game's sections is `gbuffers_water` (and a `final`
/// that shows `colortex0`), no terrain program: per-section translucent quads read the
/// normal stream, which both arenas keep from the start (a pack picked, vertex storage taken
/// away), through the frames while the pack builds and once it runs, and each translucent view
/// reads its face's own normal.
fn fixture_water_only(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    let start = layout(&renderer);
    assert!(
        start.1 && start.2,
        "a pack picked: the stream from the start"
    );
    renderer.upload(SECTION, &mesh(true));
    let folder = out.join("pack-water");
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders)?;
    std::fs::write(
        shaders.join("gbuffers_water.vsh"),
        "#version 120\nvarying vec3 n;\nvoid main() { gl_Position = ftransform(); n = gl_Normal; }\n",
    )?;
    std::fs::write(
        shaders.join("gbuffers_water.fsh"),
        "#version 120\nvarying vec3 n;\nvoid main() { gl_FragData[0] = vec4(n * 0.4 + 0.6, 1.0); }\n",
    )?;
    std::fs::write(
        shaders.join("final.vsh"),
        "#version 120\nvarying vec2 uv;\n\
         void main() { uv = gl_MultiTexCoord0.xy; gl_Position = ftransform(); }\n",
    )?;
    std::fs::write(
        shaders.join("final.fsh"),
        "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
         void main() { gl_FragColor = texture2D(colortex0, uv); }\n",
    )?;
    std::fs::write(shaders.join("shaders.properties"), "shadow.enabled=false\n")?;
    let post = read_pack(&mut renderer, &folder);
    assert!(post.water.is_some(), "gbuffers_water loads");
    assert!(
        post.terrain.is_none() && post.terrain_cutout.is_none(),
        "no terrain program"
    );
    renderer.load_shader_pack(
        0,
        post,
        Vec::new(),
        "water".into(),
        std::time::Instant::now(),
    );
    frames_keep(&mut renderer, &hud, out, "pending", 3, start);
    assert!(poll(&mut renderer).unwrap(), "the pack runs");
    frames_keep(&mut renderer, &hud, out, "running", 1, start);
    check_views(&mut renderer, &hud, out, "water", true, normal, &mut result);
    let (section, translucent, _) = renderer.section_arena_normals();
    assert!(section && translucent, "both arenas keep the stream");
    finish(out, result)
}

/// A pack the device refuses once built (`colortex0` in an integer format and no `final` to
/// draw it, `PackPost::new`'s refusal), on a renderer made with a `shaders/` pack picked where
/// per-section terrain reads the normal stream: the frames while it builds keep the stream; the
/// refusal leaves the dimension without a pack, and the next frame lays the sections out as
/// without one, the stream dropped.
fn fixture_pending_refused(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    let hud = HudBatch::scratch([320, 180]);
    let start = layout(&renderer);
    assert!(
        start.1 && start.2,
        "a pack picked: the stream from the start"
    );
    renderer.upload(SECTION, &mesh(false));
    let folder = out.join("pack-refused");
    write_normal_pack(
        &folder,
        &[
            (
                "composite.vsh",
                "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
            ),
            (
                "composite.fsh",
                "#version 330 compatibility\n/* RENDERTARGETS: 0 */\n/*\nconst int colortex0Format = RGBA32UI;\n*/\n\
                 layout(location = 0) out uvec4 o;\nvoid main() { o = uvec4(0u, 200u, 0u, 255u); }\n",
            ),
        ],
    )?;
    let post = read_pack(&mut renderer, &folder);
    renderer.load_shader_pack(
        0,
        post,
        Vec::new(),
        "refused".into(),
        std::time::Instant::now(),
    );
    frames_keep(&mut renderer, &hud, out, "pending", 2, start);
    assert!(poll(&mut renderer).is_err(), "the device refuses the pack");
    shoot(&mut renderer, &hud, 0.0, 0.0, &out.join("refused.png"));
    assert!(
        renderer.take_dropped_sections(),
        "laid out again without a pack"
    );
    let (normals, translucent, _) = renderer.section_arena_normals();
    assert!(!normals && !translucent, "no pack runs: no stream");
    finish(out, serde_json::Map::new())
}

/// The shadow pass of a pack whose per-section terrain reads the normal stream (vertex storage
/// taken away, flat draws off) reads it too: a shadow program that writes the normal into
/// `shadowcolor0` over the whole shadow map (each quad drawn from its UVs, 0 to 1 across the
/// block atlas's one sprite), and a `final` program that shows the map's middle. A single quad
/// facing east (+x) writes (255, 153, 153); the constant normal would write (153, 255, 153).
/// Solid, then translucent (the shadow pass's two lists).
fn fixture_shadow(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    assert!(!renderer.flat_draws(), "SUNRISE_FLAT_DRAWS=0");
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    let folder = out.join("pack-shadow");
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders)?;
    std::fs::write(
        shaders.join("shadow.vsh"),
        "#version 120\nvarying vec3 n;\n\
         void main() { n = gl_Normal; gl_Position = vec4(gl_MultiTexCoord0.xy * 2.0 - 1.0, 0.5, 1.0); }\n",
    )?;
    std::fs::write(
        shaders.join("shadow.fsh"),
        "#version 120\nvarying vec3 n;\nvoid main() { gl_FragData[0] = vec4(n * 0.4 + 0.6, 1.0); }\n",
    )?;
    std::fs::write(
        shaders.join("final.vsh"),
        "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
    )?;
    std::fs::write(
        shaders.join("final.fsh"),
        "#version 120\nconst int shadowMapResolution = 16;\nuniform sampler2D shadowcolor0;\n\
         void main() { gl_FragColor = texture2D(shadowcolor0, vec2(0.5)); }\n",
    )?;
    let post = read_pack(&mut renderer, &folder);
    assert!(post.shadow.is_some(), "the shadow program loads");
    renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap()
        .expect("the pack runs");
    let east = [channel(1), channel(0), channel(0), 255];
    for translucent in [false, true] {
        let name = if translucent { "translucent" } else { "solid" };
        shoot(
            &mut renderer,
            &hud,
            0.0,
            0.0,
            &out.join(format!("settle-{name}.png")),
        );
        renderer.take_dropped_sections();
        let mut quad = SectionMesh::default();
        let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
        let vertices = face(0, 1)
            .into_iter()
            .zip(uvs)
            .map(|(p, uv)| TerrainVertex::new(p, uv, 0, u32::MAX, 0xf0f0));
        if translucent {
            quad.translucent.extend(vertices);
        } else {
            quad.solid.extend(vertices);
        }
        quad.sort_solid();
        renderer.upload(SECTION, &quad);
        let pixel = shoot(
            &mut renderer,
            &hud,
            0.0,
            0.0,
            &out.join(format!("{name}.png")),
        );
        assert_eq!(
            pixel, east,
            "{name}: the shadow map holds the east face's normal"
        );
        result.insert(name.into(), format!("{pixel:?}").into());
    }
    let (section, translucent, _) = renderer.section_arena_normals();
    assert!(section && translucent, "both arenas keep the stream");
    finish(out, result)
}

/// On a device with flat draws and storage written in vertex shaders: a pack whose buffer
/// objects leave room for the normal stream and not for set 3 draws flat with flat draws on,
/// with no translucent arena and no stream; with flat draws off it draws per section from the
/// stream, and the translucent arena, which Translucent Batching then splits off, has the stream
/// too.
fn fixture_stream_after_flat(
    window: Arc<Window>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = serde_json::Map::new();
    let room = room_for_the_stream(&renderer);
    for flat in [true, false] {
        renderer.set_flat_draws(flat)?;
        renderer.set_pack_terrain_bindings(client::gfx::pack_atlas::TerrainBindings {
            buffer_objects: room,
        });
        let draws = if flat { "flat" } else { "stream" };
        pack(
            &mut renderer,
            &out.join(format!("pack-{draws}")),
            "gl_Normal",
        );
        for translucent in [false, true] {
            check_views(
                &mut renderer,
                &hud,
                out,
                draws,
                translucent,
                normal,
                &mut result,
            );
        }
        let (section, translucent, _) = renderer.section_arena_normals();
        assert_eq!(section, !flat, "{draws}: the section arena's stream");
        assert_eq!(
            translucent, !flat,
            "{draws}: the translucent arena's stream"
        );
    }
    finish(out, result)
}

fn finish(
    out: &Path,
    result: serde_json::Map<String, serde_json::Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Vulkan drew these frames. A software adapter (lavapipe) is fine: the comparisons are exact.
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    eprintln!(
        "pack_terrain_normals adapter: {} ({}, {}), driver {}",
        adapter.name, adapter.backend, adapter.kind, adapter.driver
    );
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

type Fixture = fn(Arc<Window>, &Path) -> Result<(), Box<dyn std::error::Error>>;

struct App<'a>(&'a Path, Fixture);
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = (self.1)(window, self.0) {
            eprintln!("fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn run(out: &Path, fixture: Fixture) {
    for key in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        assert!(
            std::env::var_os(key).is_some(),
            "set throwaway {key} (AGENTS.md)"
        );
    }
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_any_thread(true);
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    builder
        .build()
        .unwrap()
        .run_app(&mut App(out, fixture))
        .unwrap();
}

/// Runs `fixture` in a child process of this test binary that runs only `test`, with `env` set
/// and throwaway homes.
fn run_daily(test: &str, env: &[(&str, &str)], fixture: Fixture) {
    const OUTPUT: &str = "SUNRISE_TERRAIN_NORMALS_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("pack_terrain_normals refused: no Vulkan adapter");
        run(Path::new(&output), fixture);
        return;
    }

    // Keep the event loop and every writable home in a fresh process.
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--ignored", "--nocapture"])
        .current_dir(scratch.path())
        .env(OUTPUT, scratch.path().join("output"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off")
        .envs(env.iter().copied());
    for name in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_RUNTIME_DIR",
        "SUNRISE_LAUNCHER_TRASH",
        "SUNRISE_LAUNCHER_CLIPBOARD",
        "SUNRISE_LAUNCHER_DESKTOP",
    ] {
        let home = scratch.path().join(name);
        std::fs::create_dir(&home).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        command.env(name, home);
    }
    assert!(
        command.status().unwrap().success(),
        "terrain normal fixture {test} failed"
    );
}

fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    let eye = glam::DVec3::from_array(EYE);
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: eye,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_degrees: 85.0,
        cull_fov_degrees: 85.0,
        eye,
        eye_rotation: [0.0, 0.0],
        view_transform: glam::Mat4::IDENTITY,
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 64.0,
        detail_distance: 64.0,
        shadow_distance: None,
        depth_far: 1000.0,
        underwater: false,
        water_plane: None,
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
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        shadow_only: &[],
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
        chunk_fade: 0.0,
        post_effects: client::post_chain::Request::none(),
        menu_blur: None,
    }
}
