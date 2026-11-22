//! A pack's `core/clouds` through a game's frames: the menu bundle's atlas loads the shader,
//! the renderer the game builds takes it as a reload hands it over, and the frame draws it in
//! place of the native clouds, outside Improved Transparency in the world pass and under it in
//! `OIT_CLOUDS`' stages. Each case runs in a process of its own (one event loop, its own
//! resources).
use super::native_entity_refs::tests::{fixture_root, run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::gfx::ScreenshotRequest;

const NAME: &str =
    "game::pack_clouds_frame_tests::a_packs_clouds_draw_with_and_without_improved_transparency";
const CASE: &str = "SUNRISE_PACK_CLOUDS_FRAME_CASE";

/// Vanilla's includes `oit.glsl` pulls in, which the pack's fragment stage includes.
const OIT_INCLUDES: [&str; 7] = [
    "oit",
    "oit_common",
    "projection",
    "oit_depth_sample",
    "oit_depth_bounds",
    "oit_add_transmittance",
    "oit_sample",
];

/// A pack of vanilla's `oit.glsl` includes and `own` files.
fn pack(own: &[(&str, &str)]) -> ResourceSource {
    let root = ResourceSource::directory(fixture_root());
    let mut files: std::collections::BTreeMap<String, Resource> = OIT_INCLUDES
        .iter()
        .map(|name| {
            let path = format!("assets/minecraft/shaders/include/{name}.glsl");
            let text = root.read_to_string(&path).unwrap();
            (path, Resource::bytes(text.into_bytes()))
        })
        .collect();
    for (path, text) in own {
        files.insert(
            (*path).to_owned(),
            Resource::bytes(text.as_bytes().to_vec()),
        );
    }
    ResourceSource::indexed(files)
}

/// Clouds that cover the whole frame, nearest of all (depth 1, reversed Z), each face's quad
/// placed by `gl_VertexIndex` alone (its vertex stage reads no face). Outside Improved
/// Transparency they are 0.2, 0.4, 0.6 (51, 102, 153 in a unorm byte); under it they go through
/// vanilla's `oit.glsl` as `clouds.fsh` does, opaque, and accumulate magenta (255, 0, 255), so
/// a frame shows which program drew them.
const CLOUDS: [(&str, &str); 2] = [
    (
        "assets/minecraft/shaders/core/clouds.vsh",
        "#version 330\nvoid main() {\n    int corner = gl_VertexIndex % 4;\n    \
         vec2 xy = vec2(corner == 1 || corner == 2 ? 1.0 : -1.0, corner >= 2 ? 1.0 : -1.0);\n    \
         gl_Position = vec4(xy, 1.0, 1.0);\n}\n",
    ),
    (
        "assets/minecraft/shaders/core/clouds.fsh",
        "#version 330\n#include <minecraft:oit.glsl>\n#ifndef OIT_ALPHA_ONLY\n\
         layout(location = 0) out vec4 fragColor;\n#endif\nvoid main() {\n\
         #ifdef OIT_ALPHA_ONLY\n    executeAlphaOnlyPhase(gl_FragCoord.z, 1.0);\n\
         #elif defined(OIT_ACCUMULATE)\n    \
         fragColor = sampleColorForAccumulation(vec4(1.0, 0.0, 1.0, 1.0));\n\
         #else\n    fragColor = vec4(0.2, 0.4, 0.6, 1.0);\n#endif\n}\n",
    ),
];

/// The frame's pixels, RGBA, and its width.
fn drawn_frame(game: &mut super::Game<'_>) -> (Vec<u8>, usize) {
    game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame))
        .unwrap();
    let result = game.renderer.as_mut().unwrap().screenshot_result.take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    (std::mem::take(&mut image.rgba), image.width as usize)
}

/// The pixel at `x`, `y`.
fn pixel(frame: &(Vec<u8>, usize), x: usize, y: usize) -> [u8; 4] {
    frame.0[(y * frame.1 + x) * 4..][..4].try_into().unwrap()
}

/// Pixels left of the hotbar and the crosshair.
fn samples(frame: &(Vec<u8>, usize)) -> [[u8; 4]; 3] {
    let (pixels, width) = frame;
    let at =
        |x: usize, y: usize| -> [u8; 4] { pixels[(y * width + x) * 4..][..4].try_into().unwrap() };
    [at(20, 40), at(20, 120), at(300, 40)]
}

/// With Improved Transparency off and on, one session:
/// - a pack without `core/clouds` loads no pack clouds, and the native clouds change a known
///   cloud pixel from the same scene with clouds off;
/// - once a reload hands the renderer the pack's clouds, they cover the frame in the program's
///   colour for that path, fancy and fast; with the clouds off nothing of them is drawn (the
///   frame is the clouds-off one, to the byte);
/// - once a reload removes them, the native clouds draw again: the frame is the one drawn before
///   the pack, to the byte.
#[test]
#[ignore = "daily"]
fn a_packs_clouds_draw_with_and_without_improved_transparency() {
    if run_fresh_cases(NAME, CASE, &["classic", "improved_transparency"]) {
        return;
    }
    let improved = std::env::var(CASE).unwrap() == "improved_transparency";
    with_client_game(|game| {
        game.gui = crate::game::UiResources::load(&pack(&[]), std::path::Path::new("")).unwrap();
        assert!(game.gui.atlas.clouds_shader.is_none(), "no core/clouds");
        let _event_loop = frame_scene(game);
        let renderer = game.renderer.as_mut().unwrap();
        renderer.improved_transparency = improved;
        if improved
            && !renderer
                .device
                .features()
                .contains(wgpu::Features::FLOAT32_BLENDABLE)
        {
            // `oit::supported`: Improved Transparency needs blendable float targets.
            eprintln!("skipped: no Improved Transparency on this adapter");
            return;
        }
        game.screen = super::Screen::Playing;
        game.screen_fx.vignette_brightness = 0.0;
        // Straight up at the layer, with the field of view a tick would have settled
        // (`Camera.tick` eases the modifier from 0; this scene runs no tick).
        game.pitch = -90.0;
        game.view.fov_modifier = 1.0;
        game.view.old_fov_modifier = 1.0;
        game.settings.clouds = "true".into();
        let before = drawn_frame(game);
        game.settings.clouds = "false".into();
        let off = drawn_frame(game);
        // This pixel is under the north-west cell's bottom face. Compare it with the same
        // scene with clouds off: the fogged blend's byte from another adapter is not a reference.
        assert_ne!(
            pixel(&before, 30, 85),
            pixel(&off, 30, 85),
            "the native clouds cover this pixel"
        );

        let loaded = crate::game::UiResources::load(&pack(&CLOUDS), std::path::Path::new(""))
            .unwrap()
            .atlas
            .clouds_shader;
        let programs = loaded.as_deref().expect("the pack's clouds");
        game.renderer
            .as_mut()
            .unwrap()
            .set_clouds_shader(Some(programs))
            .unwrap();
        // Each stage translates, and the varying's location is present in both. Only pipeline
        // linking checks the vertex's float against the fragment's integer at this location.
        let mismatched = [
            (
                CLOUDS[0].0,
                "#version 330\nlayout(location = 0) flat out float tag;\nvoid main() { gl_Position = vec4(1.0); tag = 1.0; }",
            ),
            (
                CLOUDS[1].0,
                "#version 330\nlayout(location = 0) flat in int tag;\nlayout(location = 0) out vec4 pixel;\nvoid main() { pixel = vec4(float(tag)); }",
            ),
        ];
        let refused = crate::game::UiResources::load(&pack(&mismatched), std::path::Path::new(""))
            .unwrap()
            .atlas
            .clouds_shader
            .unwrap();
        let error = game
            .renderer
            .as_mut()
            .unwrap()
            .set_clouds_shader(Some(&refused))
            .unwrap_err();
        assert_eq!(
            error.split_once(": ").map(|(name, _)| name),
            Some("minecraft:pipeline/clouds")
        );
        // Refusing its replacement keeps the previously installed program for both paths.
        let expected = if improved {
            [255, 0, 255, 255]
        } else {
            [51, 102, 153, 255]
        };
        // `renderClouds`: true is fancy.
        for style in ["true", "fast"] {
            game.settings.clouds = style.into();
            assert_eq!(samples(&drawn_frame(game)), [expected; 3], "{style}");
        }
        game.settings.clouds = "false".into();
        assert!(drawn_frame(game).0 == off.0, "no clouds drawn while off");

        game.renderer
            .as_mut()
            .unwrap()
            .set_clouds_shader(None)
            .unwrap();
        game.settings.clouds = "true".into();
        assert!(
            drawn_frame(game).0 == before.0,
            "the native clouds again, to the byte"
        );
    });
}
