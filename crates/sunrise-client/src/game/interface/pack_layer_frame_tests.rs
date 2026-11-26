//! A `shaders/` pack's `layer.*` rules (spec 4.2) through the game's frames: the pack loads
//! through `shader_pack_inputs` (its pipelines on their own thread), the sections mesh and
//! upload while it is pending, and once it runs the moved state is drawn by `gbuffers_water`
//! and the other states of the same block by `gbuffers_terrain`, in the shadow pass by
//! `shadow_water` and `shadow`. A reload with other rules, another pack, the pack dropped and
//! the world's tags changing each mesh the terrain again. Each draw mode runs in a process of
//! its own (one event loop, its own resources).
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use std::sync::Arc;
use sunrise_client::gfx::ScreenshotRequest;
use sunrise_shaders::RenderLayer;
use sunrise_world::registry::BlockStateId;

const NAME: &str =
    "game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_terrain_programs";
const SHADOW_NAME: &str =
    "game::pack_layer_frame_tests::layer_rules_move_states_between_the_packs_shadow_programs";
const CASE: &str = "SUNRISE_PACK_LAYER_FRAME_CASE";

const VERTEX: &str = "#version 120\nvoid main() {\n    gl_Position = ftransform();\n}\n";
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// A fragment stage writing `rgb`, opaque: a pixel tells which program drew it.
fn fragment(rgb: &str) -> String {
    format!("#version 120\nvoid main() {{\n    gl_FragData[0] = vec4({rgb}, 1.0);\n}}\n")
}

/// A pack whose `block.properties` is `rules`: `gbuffers_terrain` draws red, `gbuffers_water`
/// green.
fn write_pack(root: &std::path::Path, rules: &str) {
    write_files(
        root,
        &[
            ("block.properties", rules.into()),
            ("gbuffers_terrain.vsh", VERTEX.into()),
            ("gbuffers_terrain.fsh", fragment("1.0, 0.0, 0.0")),
            ("gbuffers_water.vsh", VERTEX.into()),
            ("gbuffers_water.fsh", fragment("0.0, 1.0, 0.0")),
        ],
    );
}

fn write_files(root: &std::path::Path, files: &[(&str, String)]) {
    let shaders = root.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in files {
        std::fs::write(shaders.join(name), text).unwrap();
    }
}

/// The frame's pixels, RGBA, and its width.
fn drawn_frame(game: &mut super::Game<'_>) -> (Vec<u8>, usize) {
    game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame, None))
        .unwrap();
    let result = game.renderer.as_mut().unwrap().screenshot_result.take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    (std::mem::take(&mut image.rgba), image.width as usize)
}

/// The pixels over the north slab's top face and over the south one's (looking straight down,
/// facing south: south is up on the screen).
fn slabs_drawn(game: &mut super::Game<'_>) -> [[u8; 4]; 2] {
    let (pixels, width) = drawn_frame(game);
    let height = pixels.len() / 4 / width;
    let at =
        |x: usize, y: usize| -> [u8; 4] { pixels[(y * width + x) * 4..][..4].try_into().unwrap() };
    [at(width / 2, height * 5 / 6), at(width / 2, height / 6)]
}

/// Meshes and uploads every section waiting, without drawing a frame (no pack is polled).
fn settle(game: &mut super::Game<'_>) {
    let started = std::time::Instant::now();
    loop {
        game.update_world();
        if !game.streaming.busy() && game.pending_uploads.is_empty() {
            return;
        }
        assert!(started.elapsed().as_secs() < 30, "sections still meshing");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The state of `name` with every `properties` pair.
fn state(game: &super::Game<'_>, name: &str, properties: &[(&str, &str)]) -> u32 {
    let registry = game.pack.assets.registry;
    (0..registry.len() as u32)
        .find(|&i| {
            registry.state(BlockStateId(i)).is_some_and(|s| {
                s.name == name
                    && properties
                        .iter()
                        .all(|(k, v)| s.properties.get(*k).map(String::as_str) == Some(*v))
            })
        })
        .unwrap()
}

fn slab(game: &super::Game<'_>, name: &str, kind: &str) -> u32 {
    state(game, name, &[("type", kind), ("waterlogged", "false")])
}

/// Frames until the pack runs and every section meshed since it loaded is drawn.
fn pack_frames<T>(game: &mut super::Game<'_>, read: fn(&mut super::Game<'_>) -> T) -> T {
    for _ in 0..200 {
        drawn_frame(game);
        settle(game);
        if game.renderer.as_ref().unwrap().shader_pack_kind() == Some("iris") {
            return read(game);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the pack never ran");
}

/// The scene both tests draw, in this process's draw mode (`flat`): looking straight down from
/// (8.5, 65.62, 8.5) facing south, at noon, a bottom oak slab north of the player and a top one
/// south of it, at y 61, in a lit column with lit neighbours as a server sends them. `None`
/// where this adapter has no flat draws.
fn scene(game: &mut super::Game<'_>, flat: bool) -> Option<winit::event_loop::EventLoop<()>> {
    let event_loop = frame_scene(game);
    if game
        .renderer
        .as_mut()
        .unwrap()
        .set_flat_draws(flat)
        .is_err()
    {
        return None;
    }
    // A pack's first frame turns the material maps on and reloads the resource packs, under
    // the reload's overlay: on from the start, nothing reloads and no overlay covers the
    // frames compared.
    sunrise_client::sprites::set_labpbr_maps(true);
    game.screen = super::Screen::Playing;
    game.ui.hide_gui = true;
    game.screen_fx.vignette_brightness = 0.0;
    game.world.day_time = 6000;
    game.pitch = 90.0;
    game.yaw = 0.0;
    game.view.fov_modifier = 1.0;
    game.view.old_fov_modifier = 1.0;
    let player = game.player.as_mut().unwrap();
    player.position = sunrise_core::geometry::Vec3::new(8.5, 64.0, 8.5);
    game.previous_position = player.position;
    let (bottom, top) = (
        slab(game, "minecraft:oak_slab", "bottom"),
        slab(game, "minecraft:oak_slab", "top"),
    );
    // The title screen's streamer is paused; a server's columns are never generated here.
    game.streaming.paused = false;
    game.streaming.remote = true;
    game.streaming.radius = 1;
    for cx in -1..=1 {
        for cz in -1..=1 {
            let mut chunk = sunrise_client::chunk::Chunk::empty(cx, cz);
            chunk.lit = true;
            if (cx, cz) == (0, 0) {
                chunk.set(8, 61, 7, bottom);
                chunk.set(8, 61, 9, top);
            }
            game.streaming.insert_remote(&mut game.world, chunk);
        }
    }
    settle(game);
    Some(event_loop)
}

/// The issue's control, on a block that may move (4.2 keeps full opaque cubes such as the
/// issue's oak log on their layer: a pack naming one moves nothing):
/// - before any pack, both slabs draw natively (neither pixel is a program's colour);
/// - `layer.translucent=oak_slab:type=bottom` loads; the sections mesh and upload while its
///   pipelines are still building; once it runs, the bottom slab is green and the top one red;
/// - reloaded (Shader Options saved) with `type=top` named, they swap;
/// - another pack, with `layer.cutout=oak_slab:type=bottom oak_log` (the alpha test keeps both
///   in `gbuffers_terrain`), draws both red, and leaves the oak log, a full opaque cube, alone;
/// - a rule naming a tag follows the world's tags: when they stop listing oak slabs, the bottom
///   slab is red again, while another slab of the tag still moves;
/// - dropped, the terrain meshes without any rule and draws as before any pack.
#[test]
#[ignore = "daily"]
fn layer_rules_move_states_between_the_packs_terrain_programs() {
    if run_fresh_cases(NAME, CASE, &["flat", "sections"]) {
        return;
    }
    let flat = std::env::var(CASE).unwrap() == "flat";
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("layers");
    write_pack(&pack, "layer.translucent=oak_slab:type=bottom\n");
    let other = scratch.path().join("cutout");
    write_pack(&other, "layer.cutout=oak_slab:type=bottom oak_log\n");
    let tagged = scratch.path().join("tagged");
    write_pack(&tagged, "layer.translucent=%slabs:type=bottom\n");
    with_client_game(|game| {
        let Some(_event_loop) = scene(game, flat) else {
            eprintln!("skipped: {NAME}: this adapter has no flat section draws");
            return;
        };
        let bottom = slab(game, "minecraft:oak_slab", "bottom");
        let top = slab(game, "minecraft:oak_slab", "top");
        let native = slabs_drawn(game);
        for pixel in native {
            assert!(pixel != RED && pixel != GREEN, "{native:?}");
        }

        game.settings.sunrise.shader_pack = pack.to_str().unwrap().to_owned();
        // This frame loads the pack; its pipelines build on their own thread, and nothing
        // installs it before the next frame.
        drawn_frame(game);
        assert!(game.pack_ids.is_some(), "the pack loads");
        settle(game);
        assert!(
            game.renderer.as_ref().unwrap().shader_pack_kind().is_none(),
            "the sections were uploaded while the pack was pending"
        );
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [GREEN, RED],
            "the bottom slab blended by gbuffers_water"
        );
        let overrides = &game.streaming.layer_overrides;
        assert_eq!(overrides[bottom as usize], Some(RenderLayer::Translucent));
        assert_eq!(overrides[top as usize], None);

        // Reloaded with the other state named.
        write_pack(&pack, "layer.translucent=oak_slab:type=top\n");
        game.open_shader_options();
        game.save_shader_options();
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, GREEN],
            "reloaded: the rules swapped"
        );

        // Another pack: cut out, both drawn by gbuffers_terrain; the oak log keeps its layer.
        game.settings.sunrise.shader_pack = other.to_str().unwrap().to_owned();
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, RED],
            "cutout stays in gbuffers_terrain"
        );
        let log = state(game, "minecraft:oak_log", &[("axis", "y")]);
        let overrides = &game.streaming.layer_overrides;
        assert_eq!(overrides[bottom as usize], Some(RenderLayer::Cutout));
        assert_eq!(overrides[log as usize], None, "a full opaque cube");

        // A tag's rule, then the world's tags without oak slabs in `minecraft:slabs`.
        game.settings.sunrise.shader_pack = tagged.to_str().unwrap().to_owned();
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [GREEN, RED],
            "through %slabs"
        );
        let mut items = game.items().clone();
        items
            .block_tags
            .get_mut("minecraft:slabs")
            .unwrap()
            .remove("minecraft:oak_slab");
        items.rebuild_block_tag_flags(items.block_state_names());
        game.server_items = Some(Arc::new(items));
        assert_eq!(
            pack_frames(game, slabs_drawn),
            [RED, RED],
            "the tags changed"
        );
        let birch = slab(game, "minecraft:birch_slab", "bottom");
        let overrides = &game.streaming.layer_overrides;
        assert_eq!(overrides[bottom as usize], None);
        assert_eq!(
            overrides[birch as usize],
            Some(RenderLayer::Translucent),
            "the tag's other slabs still move"
        );

        // Dropped: the next frame takes the rules away with the pack, and the sections mesh
        // again as before any pack.
        game.settings.sunrise.shader_pack = String::new();
        drawn_frame(game);
        settle(game);
        drawn_frame(game);
        assert!(game.streaming.layer_overrides.is_empty());
        settle(game);
        assert_eq!(slabs_drawn(game), native);
    });
}

/// What `final` reports of the shadow map, from the frame's left column (it scans the whole
/// 64×64 `shadowcolor0` there): R is 255 when some texel is `shadow`'s red, G is 255 when some
/// texel is `shadow_water`'s green, B is 0 and A 255.
fn shadow_drawn(game: &mut super::Game<'_>) -> [u8; 4] {
    let (pixels, width) = drawn_frame(game);
    let height = pixels.len() / 4 / width;
    pixels[(height / 2 * width) * 4..][..4].try_into().unwrap()
}

/// A pack with `shadow` writing red and `shadow_water` green into `shadowcolor0` (entities and
/// block entities write blue), a 64-texel map 16 blocks around the camera, and `final` writing
/// in its left column which of the two colours the map holds:
/// - with no `layer.*` rule, only `shadow` draws the slabs;
/// - with `layer.translucent=oak_slab:type=bottom`, `shadow` draws the top slab and
///   `shadow_water` the bottom one: both colours;
/// - with every oak slab moved, only `shadow_water` draws them.
#[test]
#[ignore = "daily"]
fn layer_rules_move_states_between_the_packs_shadow_programs() {
    if run_fresh_cases(SHADOW_NAME, CASE, &["flat", "sections"]) {
        return;
    }
    let flat = std::env::var(CASE).unwrap() == "flat";
    let scratch = tempfile::tempdir().unwrap();
    let final_fsh = "#version 120\n\
        const int shadowMapResolution = 64;\nconst float shadowDistance = 16.0;\n\
        uniform sampler2D shadowcolor0;\nvoid main() {\n    vec2 found = vec2(0.0);\n\
        if (gl_FragCoord.x < 1.0) {\n        for (int y = 0; y < 64; y++) {\n\
        for (int x = 0; x < 64; x++) {\n\
        vec4 c = texture2D(shadowcolor0, (vec2(x, y) + 0.5) / 64.0);\n\
        if (c == vec4(1.0, 0.0, 0.0, 1.0)) found.x = 1.0;\n\
        if (c == vec4(0.0, 1.0, 0.0, 1.0)) found.y = 1.0;\n        }\n        }\n    }\n\
        gl_FragData[0] = vec4(found, 0.0, 1.0);\n}\n";
    let packs = [
        ("none", "block.5=stone\n"),
        ("bottom", "layer.translucent=oak_slab:type=bottom\n"),
        ("both", "layer.translucent=oak_slab\n"),
    ]
    .map(|(name, rules)| {
        let root = scratch.path().join(name);
        write_files(
            &root,
            &[
                ("block.properties", rules.into()),
                ("shadow.vsh", VERTEX.into()),
                ("shadow.fsh", fragment("1.0, 0.0, 0.0")),
                ("shadow_water.vsh", VERTEX.into()),
                ("shadow_water.fsh", fragment("0.0, 1.0, 0.0")),
                ("shadow_entities.vsh", VERTEX.into()),
                ("shadow_entities.fsh", fragment("0.0, 0.0, 1.0")),
                ("shadow_block.vsh", VERTEX.into()),
                ("shadow_block.fsh", fragment("0.0, 0.0, 1.0")),
                (
                    "final.vsh",
                    "#version 120\nvoid main() { gl_Position = ftransform(); }\n".into(),
                ),
                ("final.fsh", final_fsh.into()),
            ],
        );
        root
    });
    with_client_game(|game| {
        let Some(_event_loop) = scene(game, flat) else {
            eprintln!("skipped: {SHADOW_NAME}: this adapter has no flat section draws");
            return;
        };
        let expected = [[255, 0, 0, 255], [255, 255, 0, 255], [0, 255, 0, 255]];
        for (root, expected) in packs.iter().zip(expected) {
            game.settings.sunrise.shader_pack = root.to_str().unwrap().to_owned();
            assert_eq!(
                pack_frames(game, shadow_drawn),
                expected,
                "{}",
                root.display()
            );
        }
    });
}
