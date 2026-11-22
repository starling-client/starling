//! `sky`, `stars`, `sun` and `moon` (spec 3.2) leave their own parts of the sky out of the pack's
//! `gbuffers_skybasic` and `gbuffers_skytextured`, and nothing else. Original fixture, no pack
//! files: both programs move every part to a band of its own, two pixels wide, by `renderStage`,
//! and paint it a red of its own.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::CelestialBody::{self, Moon, Sun};
use crate::hud::HudBatch;
use wgpu::TextureFormat as T;

/// The band (0..6) of this draw's `renderStage`; any other stage goes off the screen.
const BAND: &str = "uniform int renderStage;\n\
     int band() {\n\
     \x20   if (renderStage == MC_RENDER_STAGE_SKY) return 0;\n\
     \x20   if (renderStage == MC_RENDER_STAGE_SUNSET) return 1;\n\
     \x20   if (renderStage == MC_RENDER_STAGE_SUN) return 2;\n\
     \x20   if (renderStage == MC_RENDER_STAGE_MOON) return 3;\n\
     \x20   if (renderStage == MC_RENDER_STAGE_STARS) return 4;\n\
     \x20   if (renderStage == MC_RENDER_STAGE_VOID) return 5;\n\
     \x20   return 20;\n}\n";

/// Each part's triangles, six vertices at a time, cover its band from top to bottom: a quad two
/// pixels wide.
fn files(properties: &str) -> Vec<(String, String)> {
    let vertex = format!(
        "#version 130\n{BAND}void main() {{\n\
         \x20   int c = gl_VertexID % 6;\n\
         \x20   float x = (c == 1 || c == 2 || c == 4) ? 1.0 : 0.0;\n\
         \x20   float y = (c == 2 || c == 4 || c == 5) ? 1.0 : 0.0;\n\
         \x20   gl_Position = vec4(-1.0 + (float(band()) + x) * 0.25, y * 2.0 - 1.0, 0.0, 1.0);\n}}\n"
    );
    let fragment = format!(
        "#version 130\n{BAND}/* DRAWBUFFERS:0 */\nvoid main() {{\n\
         \x20   gl_FragData[0] = vec4(float(40 * (band() + 1)) / 255.0, 0.0, 0.0, 1.0);\n}}\n"
    );
    let mut files = vec![(
        "shaders.properties".into(),
        format!("blend.gbuffers_skybasic=off\nblend.gbuffers_skytextured=off\n{properties}"),
    )];
    for program in ["gbuffers_skybasic", "gbuffers_skytextured"] {
        files.push((format!("{program}.vsh"), vertex.clone()));
        files.push((format!("{program}.fsh"), fragment.clone()));
    }
    files.push(("final.vsh".into(), VERTEX.into()));
    files.push((
        "final.fsh".into(),
        "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
         void main() { gl_FragColor = texture2D(colortex0, uv); }\n"
            .into(),
    ));
    files
}

/// A quad across the whole view, in front of the camera.
fn quad() -> [crate::mesher::Vertex; 4] {
    [
        [-400.0, -400.0],
        [400.0, -400.0],
        [400.0, 400.0],
        [-400.0, 400.0],
    ]
    .map(|[x, y]| crate::mesher::Vertex {
        pos: [x, y, 100.0],
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    })
}

/// Each two-pixel band of the pack's picture with `properties`, every pixel of a band the same:
/// a part's red, or the world's picture (`CLEAR`) where no part drew. The frame is the
/// Overworld's with every part, and one quad for each of `bodies`, which says what it is.
fn bands(gpu: &Gpu, properties: &str, bodies: &[CelestialBody]) -> [[u8; 4]; 8] {
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    // The world drew nothing: the far plane (reverse-Z) everywhere.
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files(properties),
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: None,
            block_ids: &[],
            sprite_rects: &[],
            celestial: Some(gpu.texture(T::Rgba8Unorm)),
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        sunrise_shaders::GameInputs {
            sky: Some(super::super::pack_sky::inputs()),
            celestial: Some(super::super::pack_sky::celestial_inputs()),
            ..Default::default()
        },
    );
    assert!(
        post.sky
            .as_ref()
            .is_some_and(|s| s.basic.is_some() && s.textured.is_some()),
        "{:?}",
        post.notes
    );
    let hud = HudBatch::scratch(SIZE);
    let mut frame = fixture_frame(&hud);
    frame.title = false;
    let quads: Vec<_> = bodies.iter().flat_map(|_| quad()).collect();
    frame.skybox = true;
    // Every part this frame: the sunset fan, the stars, the void.
    frame.sky.sunrise_color = [1.0, 0.5, 0.0, 1.0];
    frame.stars = Some((glam::Mat4::IDENTITY, 1.0));
    frame.dark_disc = true;
    frame.celestial = &quads;
    frame.celestial_bodies = bodies;
    let pixels = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {});
    std::array::from_fn(|band| {
        let first = pixels[band * 2];
        for row in 0..SIZE[1] as usize {
            for x in band * 2..band * 2 + 2 {
                let p = pixels[row * SIZE[0] as usize + x];
                assert_eq!(p, first, "band {band}: pixel ({x}, {row}) differs");
            }
        }
        first
    })
}

/// Where no part draws: the world's picture, cleared to 0.0625 grey (`Gpu::clear`).
const CLEAR: [u8; 4] = [16, 16, 16, 255];

/// A part's band: `40 * (band + 1)` red.
const fn red(r: u8) -> [u8; 4] {
    [r, 0, 0, 255]
}

const SKY: [u8; 4] = red(40);
const SUNSET: [u8; 4] = red(80);
const SUN: [u8; 4] = red(120);
const MOON: [u8; 4] = red(160);
const STARS: [u8; 4] = red(200);
const VOID: [u8; 4] = red(240);

/// The bands of the sky disc, the sunset fan, the sun, the moon, the stars and the void, then
/// two the sky never reaches.
const ALL: [[u8; 4]; 8] = [SKY, SUNSET, SUN, MOON, STARS, VOID, CLEAR, CLEAR];

/// The Overworld's two quads: the sun's, then the moon's.
const BOTH: &[CelestialBody] = &[Sun, Moon];

#[test]
#[ignore = "daily"]
fn every_part_draws_without_the_keys() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    assert_eq!(bands(&gpu, "", BOTH), ALL);
    assert_eq!(
        bands(&gpu, "sky=true\nstars=true\nsun=true\nmoon=true\n", BOTH),
        ALL
    );
}

#[test]
#[ignore = "daily"]
fn sky_false_leaves_out_the_disc_the_sunset_and_the_void() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    assert_eq!(
        bands(&gpu, "sky=false\n", BOTH),
        [CLEAR, CLEAR, SUN, MOON, STARS, CLEAR, CLEAR, CLEAR]
    );
}

#[test]
#[ignore = "daily"]
fn stars_false_leaves_out_the_stars() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    assert_eq!(
        bands(&gpu, "stars=false\n", BOTH),
        [SKY, SUNSET, SUN, MOON, CLEAR, VOID, CLEAR, CLEAR]
    );
}

#[test]
#[ignore = "daily"]
fn sun_false_leaves_out_the_sun() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    assert_eq!(
        bands(&gpu, "sun=false\n", BOTH),
        [SKY, SUNSET, CLEAR, MOON, STARS, VOID, CLEAR, CLEAR]
    );
}

#[test]
#[ignore = "daily"]
fn moon_false_leaves_out_the_moon() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    assert_eq!(
        bands(&gpu, "moon=false\n", BOTH),
        [SKY, SUNSET, SUN, CLEAR, STARS, VOID, CLEAR, CLEAR]
    );
}

/// The moon alone (the sun's sprite missing) is the first quad, and still the moon: it draws as
/// stage MOON, `sun=false` keeps it and `moon=false` leaves it out.
#[test]
#[ignore = "daily"]
fn the_moon_alone_follows_only_moon() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    let moon = [SKY, SUNSET, CLEAR, MOON, STARS, VOID, CLEAR, CLEAR];
    let none = [SKY, SUNSET, CLEAR, CLEAR, STARS, VOID, CLEAR, CLEAR];
    assert_eq!(bands(&gpu, "", &[Moon]), moon);
    assert_eq!(bands(&gpu, "sun=false\n", &[Moon]), moon);
    assert_eq!(bands(&gpu, "moon=false\n", &[Moon]), none);
}

/// The sun alone (the moon's sprite missing): `moon=false` keeps it, `sun=false` leaves it out.
#[test]
#[ignore = "daily"]
fn the_sun_alone_follows_only_sun() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    let sun = [SKY, SUNSET, SUN, CLEAR, STARS, VOID, CLEAR, CLEAR];
    let none = [SKY, SUNSET, CLEAR, CLEAR, STARS, VOID, CLEAR, CLEAR];
    assert_eq!(bands(&gpu, "", &[Sun]), sun);
    assert_eq!(bands(&gpu, "moon=false\n", &[Sun]), sun);
    assert_eq!(bands(&gpu, "sun=false\n", &[Sun]), none);
}
