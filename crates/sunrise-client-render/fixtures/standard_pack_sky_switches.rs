//! A `shaders/` pack's `sky`, `stars`, `sun` and `moon` (spec 3.2) on the game's own sky: the pack
//! has only `final`, so the game draws the sky disc, the sunset fan, the sun and moon, the stars
//! and the void disc into `colortex0` itself. Each switch's frame must equal the frame with only
//! that part left out of the scene, and differ from the frame with no key; `sun` and `moon` also
//! on a scene of the sun alone and one of the moon alone. Asset-free; run with
//! throwaway SUNRISE_HOME, SUNRISE_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md):
//! `cargo test -p sunrise-client-render --test standard_pack_sky_switches -- --ignored`
//! (under `python3 tools/ci/with_xvfb.py` without a display) gives each its own. On a Wayland
//! desktop, unset `WAYLAND_DISPLAY` as `with_xvfb.py` does: the child's own `XDG_RUNTIME_DIR`
//! has no Wayland socket.
use client::{
    environment::Sky,
    gfx::{CelestialBody, Frame, Renderer, ScreenshotRequest},
    graphics::GraphicsApi,
    hud::HudBatch,
    render_geometry::Vertex,
};
use std::{path::Path, sync::Arc};
use sunrise_client_render as client;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

/// Loads a pack of only `final` (colortex0 as it is) with `properties` as its
/// `shaders.properties`.
fn pack(renderer: &mut Renderer, folder: &Path, properties: &str) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(
        shaders.join("final.vsh"),
        "#version 120\nvarying vec2 uv; void main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}\n",
    )
    .unwrap();
    std::fs::write(shaders.join("final.fsh"), "#version 120\nvarying vec2 uv; uniform sampler2D colortex0; void main(){gl_FragColor=texture2D(colortex0,uv);}\n").unwrap();
    std::fs::write(shaders.join("shaders.properties"), properties).unwrap();
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    assert!(post.sky.is_none() && post.sky_textured.is_none());
    renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap()
        .expect("the pack runs");
}

/// A celestial quad as `Game::celestial_geometry` builds it (`size` blocks across at 100, on
/// the frame turned -90 about Y then `angle` about X), turned `yaw` degrees more about Y so
/// that the sun and the moon both lie in view.
fn body(out: &mut Vec<Vertex>, yaw: f32, angle: f32, size: f32) {
    let m = glam::Mat4::from_rotation_y(yaw.to_radians())
        * glam::Mat4::from_rotation_y((-90.0f32).to_radians())
        * glam::Mat4::from_rotation_x(angle.to_radians());
    let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
    for ((x, z), uv) in corners
        .into_iter()
        .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
    {
        out.push(Vertex {
            pos: m.transform_point3(glam::Vec3::new(x, 100.0, z)).to_array(),
            uv,
            layer: 2,
            color: u32::MAX,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        });
    }
}

/// The scene's parts, each of which a test frame can leave out.
#[derive(Clone, Copy)]
struct Parts {
    /// The sky disc's colour differs from the fog's (the clear colour): with them equal, the
    /// disc can't be told from where it isn't drawn.
    disc: bool,
    sunrise: bool,
    dark_disc: bool,
    stars: bool,
    sun: bool,
    moon: bool,
}

const ALL: Parts = Parts {
    disc: true,
    sunrise: true,
    dark_disc: true,
    stars: true,
    sun: true,
    moon: true,
};

fn shoot(renderer: &mut Renderer, hud: &HudBatch, parts: Parts, path: &Path) -> Vec<u8> {
    // Looking west at dusk, a little up: the sunset fan on the western horizon, the void disc
    // below it, the stars above, the sun to the left and the moon to the right.
    let fog = [0.25, 0.5, 0.25];
    let sky = Sky {
        sky_color: if parts.disc { [0.2, 0.3, 0.9] } else { fog },
        fog_color: fog,
        sun_angle: 80.0,
        moon_angle: 260.0,
        sunrise_color: [1.0, 0.4, 0.1, if parts.sunrise { 0.9 } else { 0.0 }],
        ..Sky::default()
    };
    // Each body as the game gives it: its quad, and what it is.
    let (mut celestial, mut bodies) = (Vec::new(), Vec::new());
    if parts.sun {
        body(&mut celestial, 30.0, 60.0, 30.0);
        bodies.push(CelestialBody::Sun);
    }
    if parts.moon {
        body(&mut celestial, -30.0, 60.0, 20.0);
        bodies.push(CelestialBody::Moon);
    }
    let mut frame = fixture_frame(hud);
    frame.title = false;
    frame.skybox = true;
    frame.yaw = 90.0;
    frame.pitch = -15.0;
    // 32 chunks: the sky's fog leaves the disc's colour above the horizon.
    frame.distance = 512.0;
    frame.sky = sky;
    frame.dark_disc = parts.dark_disc;
    frame.stars = parts.stars.then_some((glam::Mat4::IDENTITY, 1.0));
    frame.celestial = &celestial;
    frame.celestial_bodies = &bodies;
    renderer.screenshot = Some(ScreenshotRequest {
        path: path.to_path_buf(),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame).unwrap();
    renderer
        .screenshot_result
        .take()
        .expect("readback")
        .unwrap()
        .save()
        .unwrap();
    read(path)
}

fn read(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    bytes.truncate(info.buffer_size());
    bytes
}

/// How many pixels differ between two pictures of the same size.
fn differing(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count()
}

fn fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
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
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        None,
        GraphicsApi::Vulkan,
        false,
        (true, true),
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    renderer.resize(320, 180);
    let hud = HudBatch::scratch([320, 180]);

    pack(&mut renderer, &out.join("no-key"), "shadow.enabled=false\n");
    let all = shoot(&mut renderer, &hud, ALL, &out.join("no-key.png"));
    // Each part leaves its own pixels: the scene shows all six.
    let mut result = serde_json::Map::new();
    let cases = [
        (
            "sky",
            Parts {
                disc: false,
                sunrise: false,
                dark_disc: false,
                ..ALL
            },
        ),
        (
            "stars",
            Parts {
                stars: false,
                ..ALL
            },
        ),
        ("sun", Parts { sun: false, ..ALL }),
        ("moon", Parts { moon: false, ..ALL }),
    ];
    let mut oracles = Vec::new();
    for (key, parts) in cases {
        let oracle = shoot(
            &mut renderer,
            &hud,
            parts,
            &out.join(format!("{key}-oracle.png")),
        );
        oracles.push(oracle);
    }
    for (part, parts) in [
        ("disc", Parts { disc: false, ..ALL }),
        (
            "sunrise",
            Parts {
                sunrise: false,
                ..ALL
            },
        ),
        (
            "dark_disc",
            Parts {
                dark_disc: false,
                ..ALL
            },
        ),
    ] {
        let without = shoot(
            &mut renderer,
            &hud,
            parts,
            &out.join(format!("{part}-oracle.png")),
        );
        let pixels = differing(&all, &without);
        assert!(pixels > 0, "the scene shows the {part}");
        result.insert(format!("{part}_pixels"), pixels.into());
    }
    // One body alone, as `Game::celestial_geometry` gives it when the other's sprite is
    // missing: `sun` and `moon` still leave out their own body, and only it.
    let [_, _, moon_alone, sun_alone] = &oracles[..] else {
        unreachable!()
    };
    let neither = shoot(
        &mut renderer,
        &hud,
        Parts {
            sun: false,
            moon: false,
            ..ALL
        },
        &out.join("neither-oracle.png"),
    );
    for (body, alone) in [("sun", sun_alone), ("moon", moon_alone)] {
        let pixels = differing(alone, &neither);
        assert!(pixels > 0, "the scene shows the {body} alone");
        result.insert(format!("{body}_alone_pixels"), pixels.into());
    }
    // Under each key, a scene of one body and the frame it must give: the other body's switch
    // keeps it, its own leaves it out.
    let sun_only = Parts { moon: false, ..ALL };
    let moon_only = Parts { sun: false, ..ALL };
    let singles = [
        ("sun", "moon-alone", moon_only, moon_alone),
        ("sun", "sun-alone", sun_only, &neither),
        ("moon", "sun-alone", sun_only, sun_alone),
        ("moon", "moon-alone", moon_only, &neither),
    ];
    for ((key, _), oracle) in cases.iter().zip(&oracles) {
        let pixels = differing(&all, oracle);
        assert!(pixels > 0, "the scene shows what `{key}` leaves out");
        pack(
            &mut renderer,
            &out.join(format!("{key}-false")),
            &format!("shadow.enabled=false\n{key}=false\n"),
        );
        let switched = shoot(
            &mut renderer,
            &hud,
            ALL,
            &out.join(format!("{key}-false.png")),
        );
        assert_eq!(
            differing(&switched, oracle),
            0,
            "`{key}=false` leaves out exactly its part"
        );
        result.insert(format!("{key}_pixels"), pixels.into());
        for (_, scene, parts, expected) in singles.iter().filter(|s| s.0 == *key) {
            let switched = shoot(
                &mut renderer,
                &hud,
                *parts,
                &out.join(format!("{key}-false-{scene}.png")),
            );
            assert_eq!(
                differing(&switched, expected),
                0,
                "`{key}=false` on the {scene} scene leaves out exactly its body"
            );
        }
    }
    // Vulkan drew these frames. A software adapter (lavapipe) is fine: the comparisons are exact.
    let adapter = client::gfx::adapter().expect("the renderer's adapter");
    assert_eq!(adapter.backend, "Vulkan");
    result.insert(
        "graphics_adapter".into(),
        format!(
            "{} ({}, {}), driver {}",
            adapter.name, adapter.backend, adapter.kind, adapter.driver
        )
        .into(),
    );
    result.insert("fixture_only_not_gameplay_or_fps_proof".into(), true.into());
    std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

struct App<'a>(&'a Path);
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = fixture(window, self.0) {
            eprintln!("fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn run(out: &Path) {
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
    builder.build().unwrap().run_app(&mut App(out)).unwrap();
}

pub(crate) fn run_daily() {
    const OUTPUT: &str = "SUNRISE_SKY_SWITCH_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("standard_pack_sky_switches refused: no Vulkan adapter");
        run(Path::new(&output));
        return;
    }

    // Keep the event loop and every writable home in a fresh process.
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "standard_pack_sky_switches",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(OUTPUT, scratch.path().join("output"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off");
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
        "sky switch fixture failed"
    );
}

fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        eye: glam::DVec3::new(0.5, 100.0, 0.5),
        eye_rotation: [0.0, 10.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        water_plane: None,
        cave_culling: false,
        title: true,
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
