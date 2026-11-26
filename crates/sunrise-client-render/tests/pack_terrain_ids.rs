//! A `shaders/` pack's block ids and block centres on terrain (`mc_Entity`, `at_midBlock`, spec
//! §4.1, §5.1, §5.3) through the real `Renderer`, in every way the game draws terrain for a pack:
//! flat, per section reading the arena's pages, and per section reading the normal stream where
//! vertex shaders read no storage (#5015). A pack whose terrain and water programs write the
//! attributes as their colour looks at five quads around the camera, each from a block of its own:
//! solid (`gbuffers_terrain`'s solid part), cutout (its cutout part), a fluid in the translucent
//! list (`gbuffers_water`), a block `block.properties` does not list, and a block past every
//! listed state; and its `shadow` program writes them into `shadowcolor0`, which its `final`
//! shows, for a solid, a cutout and a water quad. Asset-free; run with throwaway SUNRISE_HOME, SUNRISE_LAUNCHER_HOME,
//! XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md):
//! `cargo test -p sunrise-client-render --test pack_terrain_ids -- --ignored`
//! (under `python3 tools/ci/with_xvfb.py` without a display) gives each its own. On a Wayland
//! desktop, unset `WAYLAND_DISPLAY` as `with_xvfb.py` does: the child's own `XDG_RUNTIME_DIR`
//! has no Wayland socket.
use client::{
    gfx::{Frame, Renderer, ScreenshotRequest},
    graphics::GraphicsApi,
    hud::HudBatch,
    mesher::{OPAQUE_LAYER, SectionMesh, TerrainSource, TerrainVertex},
};
use std::{path::Path, sync::Arc};
use sunrise_client_render as client;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

/// Flat draws, then per-section draws that read the section arena's pages, on a device that
/// has flat draws.
#[test]
#[ignore = "daily"]
fn pack_terrain_ids() {
    run_daily("pack_terrain_ids", &[], fixture);
}

/// Per-section draws that read the pages on a device taken to have no flat draws
/// (`SUNRISE_FLAT_DRAWS=0`): the section arena keeps no sources for flat draws there.
#[test]
#[ignore = "daily"]
fn pack_terrain_ids_without_flat_draws() {
    run_daily(
        "pack_terrain_ids_without_flat_draws",
        &[("SUNRISE_FLAT_DRAWS", "0")],
        fixture_without_flat,
    );
}

/// Per-section draws that read the normal stream: no flat draws, and vertex shaders taken to
/// read no storage.
#[test]
#[ignore = "daily"]
fn pack_terrain_ids_from_the_stream() {
    run_daily(
        "pack_terrain_ids_from_the_stream",
        &[
            ("SUNRISE_FLAT_DRAWS", "0"),
            ("SUNRISE_SHADER_PACK_WITHOUT", "vertex-storage"),
        ],
        fixture_stream,
    );
}

/// The section the quads are in: keys count sections from the bottom of the world, y -64, so
/// 10 is y 96 to 112.
const SECTION: (i32, i32, i32) = (0, 10, 0);
/// The camera, at the middle of the section (section block 8, 8, 8).
const EYE: [f64; 3] = [8.0, 104.0, 8.0];

/// The block states the quads come from, and the two ids `block.properties` gives each state
/// (`Renderer::set_shader_pack`'s `block_ids`: the model's, then the fluid's).
const SOLID: u32 = 3;
const CUTOUT: u32 = 5;
const WATER: u32 = 6;
/// Listed in no `block.<id>` line: both its ids -1 (spec §5.3).
const UNLISTED: u32 = 1;
/// Past the table's last state.
const PAST: u32 = 9;
const STATES: usize = 8;

/// `block_ids` for `STATES` states: every one -1 but `SOLID`, `CUTOUT` and `WATER`, each with
/// ids of its own, `shift` added (a second pack's other ids).
fn block_ids(shift: i32) -> Vec<i32> {
    let mut ids = vec![-1; 2 * STATES];
    for (state, model, fluid) in [(SOLID, 7, 70), (CUTOUT, 23, 230), (WATER, 40, 41)] {
        ids[2 * state as usize] = model + shift;
        ids[2 * state as usize + 1] = fluid + shift;
    }
    ids
}

/// One quad: the state it comes from, its emitted light, whether it is a fluid, its sprite layer
/// (`OPAQUE_LAYER` makes it solid rather than cutout), whether it is in the translucent list, and
/// the view that looks at it: yaw, pitch and the axis and sign of the normal facing the camera.
struct Quad {
    name: &'static str,
    state: u32,
    emission: u8,
    fluid: bool,
    layer: u16,
    translucent: bool,
    yaw: f32,
    pitch: f32,
    axis: usize,
    sign: i32,
}

/// Yaw 0 looks south (+z) and 90 west (-x); pitch 90 looks down.
const QUADS: [Quad; 5] = [
    Quad {
        name: "solid",
        state: SOLID,
        emission: 9,
        fluid: false,
        layer: OPAQUE_LAYER,
        translucent: false,
        yaw: 0.0,
        pitch: 0.0,
        axis: 2,
        sign: -1,
    },
    Quad {
        name: "cutout",
        state: CUTOUT,
        emission: 4,
        fluid: false,
        layer: 0,
        translucent: false,
        yaw: 90.0,
        pitch: 0.0,
        axis: 0,
        sign: 1,
    },
    Quad {
        name: "water",
        state: WATER,
        emission: 2,
        fluid: true,
        layer: 0,
        translucent: true,
        yaw: 180.0,
        pitch: 0.0,
        axis: 2,
        sign: 1,
    },
    Quad {
        name: "unlisted",
        state: UNLISTED,
        emission: 12,
        fluid: false,
        layer: OPAQUE_LAYER,
        translucent: false,
        yaw: 270.0,
        pitch: 0.0,
        axis: 0,
        sign: -1,
    },
    Quad {
        name: "past",
        state: PAST,
        emission: 1,
        fluid: false,
        layer: 0,
        translucent: false,
        yaw: 0.0,
        pitch: 90.0,
        axis: 1,
        sign: 1,
    },
];

/// The quad's block (its cell in the section) and its corners: its centre 4.5 blocks from the
/// camera along its normal's axis, toward the camera; across it, the cell (7, 7) holding the
/// camera's line of sight. The quad lies a quarter block from its block's centre toward the
/// camera and reaches 0.75 of a block from it along the first axis across and 0.625 along the
/// second, so each corner's offset to the block's centre is, in 64ths of a block, 16 along the
/// normal and 48 and 40 across: whole numbers, the same size at every corner.
fn quad(q: &Quad) -> ([i32; 3], [[f32; 3]; 4]) {
    let (axis, sign) = (q.axis, q.sign);
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut cell = [7; 3];
    cell[axis] = if sign > 0 { 3 } else { 12 };
    let centre = cell.map(|c| c as f32 + 0.5);
    let corner = |du: f32, dv: f32| {
        let mut p = centre;
        p[axis] += 0.25 * sign as f32;
        p[u] += du * 0.75;
        p[v] += dv * 0.625;
        p
    };
    // `e_u x e_v = e_axis` for a cyclic order: this order faces +axis.
    let up = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let corners = if sign > 0 {
        up
    } else {
        [up[3], up[2], up[1], up[0]]
    };
    (cell, corners)
}

/// Every quad, each with its source, sorted as the meshing workers sort a mesh.
fn mesh() -> SectionMesh {
    mesh_of(&QUADS.each_ref())
}

/// `quads`, each with its source, sorted as the meshing workers sort a mesh.
fn mesh_of(quads: &[&Quad]) -> SectionMesh {
    let mut mesh = SectionMesh::default();
    let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    for q in quads {
        let (cell, corners) = quad(q);
        let vertices = corners
            .into_iter()
            .zip(uvs)
            .map(|(p, uv)| TerrainVertex::new(p, uv, q.layer, u32::MAX, 0xf0f0));
        let source = TerrainSource::new(q.state, cell, q.emission, q.fluid);
        if q.translucent {
            mesh.translucent.extend(vertices);
            mesh.translucent_sources.push(source);
        } else {
            mesh.solid.extend(vertices);
            mesh.solid_sources.push(source);
        }
    }
    mesh.sort_solid();
    mesh
}

/// What the pack writes, each as a colour of three whole 255ths.
#[derive(Clone, Copy, Debug)]
enum Probe {
    /// `mc_Entity.x + 1`, `mc_Entity.y + 2`, `at_midBlock.w + 1`.
    Entity,
    /// `abs(at_midBlock.xyz)`: each vertex's offset to its block's centre, the same size at
    /// every corner of these quads.
    Offset,
    /// `at_midBlock.xyz` along the normal (`gl_Normal`), plus 128: the side of the block's
    /// centre the quad is on, the same at every corner.
    Side,
    /// The shadow pass's: `mc_Entity.x + 1`, `Side`'s and `at_midBlock.w + 1`.
    Shadow,
}

impl Probe {
    fn glsl(self) -> &'static str {
        match self {
            Probe::Entity => {
                "vec3(mc_Entity.x + 1.0, mc_Entity.y + 2.0, at_midBlock.w + 1.0) / 255.0"
            }
            Probe::Offset => "abs(at_midBlock.xyz) / 255.0",
            Probe::Side => "vec3(dot(at_midBlock.xyz, gl_Normal) + 128.0, 0.0, 0.0) / 255.0",
            Probe::Shadow => {
                "vec3(mc_Entity.x + 1.0, dot(at_midBlock.xyz, gl_Normal) + 128.0, at_midBlock.w + 1.0) / 255.0"
            }
        }
    }

    /// The colour for quad `q` under `ids` (`block_ids`): spec §5.1 and §5.3, worked out from
    /// the quad as placed (`quad`), not from the renderer.
    fn want(self, q: &Quad, ids: &[i32]) -> [u8; 4] {
        // x: the state's id, its fluid id for a fluid, -1 past the table.
        let column = usize::from(q.fluid);
        let id = ids
            .get(2 * q.state as usize + column)
            .copied()
            .unwrap_or(-1);
        let rgb = match self {
            // y as flat draws give it: 1 for a fluid, 0 otherwise.
            Probe::Entity => [id + 1, i32::from(q.fluid) + 2, i32::from(q.emission) + 1],
            Probe::Offset => {
                let mut offset = [0; 3];
                offset[q.axis] = 16;
                offset[(q.axis + 1) % 3] = 48;
                offset[(q.axis + 2) % 3] = 40;
                offset
            }
            // The centre is a quarter block behind the quad, against its normal.
            Probe::Side => [128 - 16, 0, 0],
            // A quarter block above or below its block's centre.
            Probe::Shadow => [id + 1, 128 - 16, i32::from(q.emission) + 1],
        };
        let [r, g, b] = rgb.map(|c| u8::try_from(c).unwrap());
        [r, g, b, 255]
    }
}

/// Writes a pack whose terrain and water programs write `probe` into `colortex0`, and loads it.
/// For `Probe::Shadow`, they write `Probe::Entity`, and the pack's shadow program writes the
/// shadow probe into a 16 by 16 `shadowcolor0`, each quad drawn from its UVs (where its sprite
/// is in the block atlas), whichever way the light comes from; its `final` shows the last texel
/// the shadow program wrote, told from the clear (1, 1, 1, 1, spec §6.3) by its red, under 0.5
/// for every probe here (black for none).
fn post(renderer: &mut Renderer, folder: &Path, probe: Probe) -> sunrise_shaders::Post {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    let shadow = matches!(probe, Probe::Shadow);
    let terrain = if shadow { Probe::Entity } else { probe };
    let programs = [
        ("gbuffers_terrain", terrain, "ftransform()"),
        ("gbuffers_water", terrain, "ftransform()"),
        (
            "shadow",
            Probe::Shadow,
            "vec4(gl_MultiTexCoord0.xy * 2.0 - 1.0, 0.5, 1.0)",
        ),
    ];
    for (name, probe, position) in &programs[..if shadow { 3 } else { 2 }] {
        std::fs::write(
            shaders.join(format!("{name}.vsh")),
            format!(
                "#version 120\nattribute vec4 mc_Entity;\nattribute vec4 at_midBlock;\n\
                 varying vec3 probe;\n\
                 void main() {{ gl_Position = {position}; probe = {}; }}\n",
                probe.glsl()
            ),
        )
        .unwrap();
        std::fs::write(
            shaders.join(format!("{name}.fsh")),
            "#version 120\nvarying vec3 probe;\n\
             void main() { gl_FragData[0] = vec4(probe, 1.0); }\n",
        )
        .unwrap();
    }
    if shadow {
        std::fs::write(
            shaders.join("final.vsh"),
            "#version 120\nvoid main() { gl_Position = ftransform(); }\n",
        )
        .unwrap();
        std::fs::write(
            shaders.join("final.fsh"),
            "#version 330 compatibility\nconst int shadowMapResolution = 16;\n\
             uniform sampler2D shadowcolor0;\nlayout(location = 0) out vec4 color;\n\
             void main() {\n\
                 color = vec4(0.0, 0.0, 0.0, 1.0);\n\
                 for (int y = 0; y < 16; y++) for (int x = 0; x < 16; x++) {\n\
                     vec4 c = texelFetch(shadowcolor0, ivec2(x, y), 0);\n\
                     if (c.r < 0.5) color = vec4(c.rgb, 1.0);\n\
                 }\n\
             }\n",
        )
        .unwrap();
    } else {
        std::fs::write(shaders.join("shaders.properties"), "shadow.enabled=false\n").unwrap();
    }
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    assert!(post.terrain.is_some(), "gbuffers_terrain loads");
    assert!(post.water.is_some(), "gbuffers_water loads");
    assert_eq!(post.shadow.is_some(), shadow, "the shadow program loads");
    post
}

/// `post` run at once, with `ids`.
fn pack(renderer: &mut Renderer, folder: &Path, probe: Probe, ids: &[i32]) {
    let post = post(renderer, folder, probe);
    renderer
        .set_shader_pack(0, Some(post), ids)
        .unwrap()
        .expect("the pack runs");
}

/// `post` built on the renderer's loading thread (`load_shader_pack`), as the game loads a pack,
/// with `mesh()` uploaded while it builds, before it runs, and frames drawn meanwhile; run once
/// built (`poll_shader_pack`). Then the game meshes again whatever the renderer dropped
/// (`take_dropped_sections`): whether it did.
fn pack_async(
    renderer: &mut Renderer,
    hud: &HudBatch,
    folder: &Path,
    probe: Probe,
    ids: &[i32],
) -> bool {
    let post = post(renderer, folder, probe);
    let started = std::time::Instant::now();
    renderer.load_shader_pack(0, post, ids.to_vec(), "ids".into(), started);
    renderer.upload(SECTION, &mesh());
    let frame = folder.join("loading.png");
    let report = loop {
        if let Some((_, _, _, report)) = renderer.poll_shader_pack() {
            break report;
        }
        assert!(started.elapsed().as_secs() < 120, "the pack builds");
        shoot(renderer, hud, 0.0, 0.0, &frame);
    };
    report.unwrap().expect("the pack runs");
    shoot(renderer, hud, 0.0, 0.0, &folder.join("running.png"));
    let dropped = renderer.take_dropped_sections();
    if dropped {
        renderer.upload(SECTION, &mesh());
    }
    dropped
}

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

/// A renderer of 320 by 180 with Translucent Batching on; `pack` as `Renderer::new` takes the
/// picked pack (it draws flat, it is a `shaders/` pack).
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
    // The game's meshes carry sources while a `shaders/` pack is picked.
    renderer.keep_pack_sources(true);
    Ok(renderer)
}

type Results = serde_json::Map<String, serde_json::Value>;

/// Each quad's pixel against `probe` under `ids`, the sections as they are.
fn views(
    renderer: &mut Renderer,
    hud: &HudBatch,
    out: &Path,
    name: &str,
    probe: Probe,
    ids: &[i32],
    result: &mut Results,
) {
    for q in &QUADS {
        let name = format!("{name}-{}", q.name);
        let path = out.join(format!("{name}.png"));
        let pixel = shoot(renderer, hud, q.yaw, q.pitch, &path);
        assert_eq!(pixel, probe.want(q, ids), "{name}");
        result.insert(name, format!("{pixel:?}").into());
    }
}

/// A frame for the pack just loaded, which lays the sections out for it (and may drop them),
/// then `mesh` uploaded into that layout.
/// The bytes the upload copied (`Renderer::uploaded_bytes`, which counts toward the section
/// copies' submission threshold as it goes).
fn settle(renderer: &mut Renderer, hud: &HudBatch, path: &Path, mesh: &SectionMesh) -> u64 {
    shoot(renderer, hud, 0.0, 0.0, path);
    renderer.take_dropped_sections();
    let before = renderer.uploaded_bytes;
    renderer.upload(SECTION, mesh);
    renderer.uploaded_bytes - before
}

/// Bytes `mesh()`'s upload copies: its 20 vertices of 24 bytes (`TerrainVertex`), and for each
/// vertex slot its source (8 bytes, `TerrainSource`) where the arena keeps them (flat draws, or
/// per-section ones that read the pages), or its entry in the normal stream (16 bytes,
/// `pack_atlas::NORMAL_BYTES`) where per-section draws read that.
const WITH_SOURCES: u64 = 20 * (24 + 8);
const WITH_STREAM: u64 = 20 * (24 + 16);

/// Each quad's pixel for each probe, the pack loaded with `ids`, the mesh's upload copying
/// `bytes`; then the shadow pass's.
fn check(
    renderer: &mut Renderer,
    hud: &HudBatch,
    out: &Path,
    (draws, bytes): (&str, u64),
    ids: &[i32],
    result: &mut Results,
) {
    for probe in [Probe::Entity, Probe::Offset, Probe::Side] {
        let name = format!("{draws}-{probe:?}");
        pack(renderer, &out.join(format!("pack-{name}")), probe, ids);
        let path = out.join(format!("{name}-settle.png"));
        assert_eq!(
            settle(renderer, hud, &path, &mesh()),
            bytes,
            "{name}: bytes"
        );
        // The player's upload budget counts what the upload copies.
        assert_eq!(renderer.mesh_upload_bytes(&mesh()), bytes, "{name}: budget");
        views(renderer, hud, out, &name, probe, ids, result);
    }
    shadow(renderer, hud, out, draws, ids, result);
}

/// The shadow program's pixel (`post`'s `final`) for each of the solid, cutout and water quads
/// alone in the section.
fn shadow(
    renderer: &mut Renderer,
    hud: &HudBatch,
    out: &Path,
    draws: &str,
    ids: &[i32],
    result: &mut Results,
) {
    for q in &QUADS[..3] {
        let name = format!("{draws}-shadow-{}", q.name);
        pack(
            renderer,
            &out.join(format!("pack-{name}")),
            Probe::Shadow,
            ids,
        );
        settle(
            renderer,
            hud,
            &out.join(format!("{name}-settle.png")),
            &mesh_of(&[q]),
        );
        let pixel = shoot(renderer, hud, 0.0, 0.0, &out.join(format!("{name}.png")));
        assert_eq!(pixel, Probe::Shadow.want(q, ids), "{name}");
        result.insert(name, format!("{pixel:?}").into());
    }
}

/// Flat draws on, then off (per-section draws reading the pages), the pack loaded after each
/// switch.
fn fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    let hud = HudBatch::scratch([320, 180]);
    let mut result = Results::new();
    let ids = block_ids(0);
    for flat in [true, false] {
        renderer.set_flat_draws(flat)?;
        let draws = if flat { "flat" } else { "pages" };
        let mode = (draws, WITH_SOURCES);
        check(&mut renderer, &hud, out, mode, &ids, &mut result);
        assert!(
            renderer.section_arena_pages().0,
            "{draws}: the pages are storage"
        );
        assert!(
            !renderer.section_arena_normals().0,
            "{draws}: no normal stream"
        );
    }
    finish(out, result)
}

/// No flat draws: per-section draws read the pages, whose sources the section arena keeps
/// while the pack reads them: first a pack loaded as the game loads one (`load_shader_pack`,
/// the section uploaded while it builds), then the full check with packs run at once.
fn fixture_without_flat(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (false, false))?;
    assert!(!renderer.flat_draws(), "SUNRISE_FLAT_DRAWS=0");
    let hud = HudBatch::scratch([320, 180]);
    let mut result = Results::new();
    let ids = block_ids(0);
    pack_async(
        &mut renderer,
        &hud,
        &out.join("pack-async"),
        Probe::Entity,
        &ids,
    );
    let name = "async-pages";
    views(
        &mut renderer,
        &hud,
        out,
        name,
        Probe::Entity,
        &ids,
        &mut result,
    );
    check(
        &mut renderer,
        &hud,
        out,
        ("pages", WITH_SOURCES),
        &block_ids(0),
        &mut result,
    );
    assert!(renderer.section_arena_pages().0, "the pack reads the pages");
    assert!(renderer.keeps_pack_sources(), "the pages keep sources");
    finish(out, result)
}

/// No flat draws and no storage read in vertex shaders: per-section draws read the normal
/// stream, which carries each quad's ids as the running pack's `block.properties` gives them.
/// At start-up, a renderer made with a `shaders/` pack picked has the stream before the pack
/// is built: sections uploaded meanwhile show the pack's ids once it runs, from the loading
/// thread as the game loads it. A second pack with other ids, loaded the same way, shows its own
/// on the same sections.
fn fixture_stream(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut renderer = renderer(window, (true, true))?;
    assert!(!renderer.flat_draws(), "SUNRISE_FLAT_DRAWS=0");
    let hud = HudBatch::scratch([320, 180]);
    let mut result = Results::new();
    for (name, ids) in [("start", block_ids(0)), ("other", block_ids(100))] {
        let folder = out.join(format!("pack-{name}"));
        // The section uploaded while the pack builds holds the earlier ids (none at start-up):
        // running, the pack has it meshed again under its own.
        let dropped = pack_async(&mut renderer, &hud, &folder, Probe::Entity, &ids);
        assert!(
            dropped,
            "{name}: the section is meshed again for the pack's ids"
        );
        views(
            &mut renderer,
            &hud,
            out,
            name,
            Probe::Entity,
            &ids,
            &mut result,
        );
    }
    check(
        &mut renderer,
        &hud,
        out,
        ("stream", WITH_STREAM),
        &block_ids(0),
        &mut result,
    );
    let (section, translucent, _) = renderer.section_arena_normals();
    assert!(section && translucent, "the pack reads the stream");
    assert!(!renderer.section_arena_pages().0, "no pages read");
    finish(out, result)
}

fn finish(out: &Path, result: Results) -> Result<(), Box<dyn std::error::Error>> {
    // Vulkan drew these frames. A software adapter (lavapipe) is fine: the comparisons are exact.
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    eprintln!(
        "pack_terrain_ids adapter: {} ({}, {}), driver {}",
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
    const OUTPUT: &str = "SUNRISE_TERRAIN_IDS_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("pack_terrain_ids refused: no Vulkan adapter");
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
        "terrain id fixture {test} failed"
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
