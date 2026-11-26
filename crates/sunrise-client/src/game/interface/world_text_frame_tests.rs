//! A pack's `core/text` on a game's world text, through its frames: a text display's
//! translucent background and a name tag's copies, outside and under Improved Transparency.
//! Vanilla 26.3 draws a name tag's depth-tested copy with `RenderTypes.text`
//! (`RenderPipelines.TEXT`, `OIT_TEXT`; `SubmitNodeCollection.submitNameTag`), a text display's
//! background with `RenderTypes.textPolygonOffset` (`TEXT_POLYGON_OFFSET`,
//! `OIT_TEXT_POLYGON_OFFSET`), both from `core/text`. Each case runs in a process of its own (one
//! event loop, its own resources).
use super::lightmap_expression_tests::{corner_of as corner, frame};
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_core::nbt::Tag;
use sunrise_protocol::entity_data::Value;

const NAME: &str = "game::world_text_frame_tests::a_packs_text_draws_world_text_with_and_without_improved_transparency";
const CASE: &str = "SUNRISE_WORLD_TEXT_FRAME_CASE";

/// Text that covers the whole frame with one triangle of each glyph's quad (a vertex's index
/// picks its corner), nearer than the empty world: translucent text nearer (depth 0.75, reversed)
/// than opaque text (0.25), so a frame shows which program drew the nearest. The
/// world text paints opaque text green (0, 255, 0) and translucent text, backgrounds included,
/// red with the vertex's green and blue (a black background red, white glyphs white), except a
/// pure blue background: half-transparent blue, and nearer still (depth 0.9). Under Improved
/// Transparency it goes through vanilla's `oit.glsl`, opaque, and accumulates magenta
/// (255, 0, 255). The GUI's and the see-through text draw nothing.
const TEXT: [(&str, &str); 2] = [
    (
        "assets/minecraft/shaders/core/text.vsh",
        "#version 330\nlayout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\n\
         layout(location = 0) flat out vec4 vertexColor;\nvoid main() {\n    \
         int corner = gl_VertexID % 3;\n    vec2 uv = vec2((corner << 1) & 2, corner & 2);\n    \
         bool blue = Color.b > 0.99 && Color.g < 0.01;\n    \
         float depth = Color.a > 0.99 ? 0.25 : blue ? 0.9 : 0.75;\n    \
         gl_Position = vec4(uv * 2.0 - 1.0 + 0.0 * Position.xy, depth, 1.0);\n    \
         vertexColor = Color;\n}\n",
    ),
    (
        "assets/minecraft/shaders/core/text.fsh",
        "#version 330\n#include <minecraft:oit.glsl>\n\
         layout(location = 0) flat in vec4 vertexColor;\n\
         #ifndef OIT_ALPHA_ONLY\nlayout(location = 0) out vec4 fragColor;\n#endif\nvoid main() {\n\
         #if defined(IS_GUI) || defined(IS_SEE_THROUGH)\n    discard;\n\
         #elif defined(OIT_ALPHA_ONLY)\n    executeAlphaOnlyPhase(gl_FragCoord.z, 1.0);\n\
         #elif defined(OIT_ACCUMULATE)\n    \
         fragColor = sampleColorForAccumulation(vec4(1.0, 0.0, 1.0, 1.0));\n\
         #else\n    fragColor = vertexColor.a > 0.99 ? vec4(0.0, 1.0, 0.0, 1.0) \
         : vertexColor.b > 0.99 && vertexColor.g < 0.01 ? vec4(0.0, 0.0, 1.0, 0.5) \
         : vec4(1.0, vertexColor.gb, 1.0);\n#endif\n}\n",
    ),
];

const GREEN: [u8; 3] = [0, 255, 0];
const RED: [u8; 3] = [255, 0, 0];
const MAGENTA: [u8; 3] = [255, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

/// With Improved Transparency off and on, one session with the pack's text:
/// - a text display's text (opaque, the solid phase) and its default background (translucent):
///   the background's program draws last, red outside Improved Transparency, its `OIT_*` stages'
///   magenta under it;
/// - an empty display with no background draws nothing (the frame is the empty scene's);
/// - named, its name tag's depth-tested copy is opaque text: green in either mode (the solid
///   phase, as `canRenderAsSolid` puts it);
/// - crouching (`isDiscrete`), the tag is drawn once, faded on its background: translucent text,
///   its white glyphs last outside Improved Transparency, magenta under it;
/// - that tag behind the display's own blue background (its text back), outside Improved
///   Transparency: the background blends over the tag, half blue over white
///   (`FeatureRenderDispatcher.PreparedFrame.executeTranslucent` runs `nameTags` before `texts`);
/// - and, in a case of its own, a mod's world view draws the name tag
///   ([`a_world_view_draws_the_name_tag`]).
#[test]
#[ignore = "daily"]
fn a_packs_text_draws_world_text_with_and_without_improved_transparency() {
    let cases = ["classic", "improved_transparency", "world_view"];
    if run_fresh_cases(NAME, CASE, &cases) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    if case == "world_view" {
        return a_world_view_draws_the_name_tag();
    }
    let improved = case == "improved_transparency";
    with_client_game(|game| {
        let pack = super::pack_clouds_frame_tests::pack(&TEXT);
        game.gui = crate::game::UiResources::load(&pack, std::path::Path::new("")).unwrap();
        let text = &game.gui.atlas.text_shaders;
        assert!(
            text.world.is_some() && text.oit.is_some(),
            "the pack's text"
        );
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
        let sy = game.world.shape.column().section_index(65).unwrap() as i32;
        game.settings.chunk_fade = 0.0;
        game.renderer
            .as_mut()
            .unwrap()
            .upload((0, sy, 0), &sunrise_client::mesher::SectionMesh::default());
        // The world alone (F1): the GUI's text draws nothing anyway.
        game.ui.hide_gui = true;
        let empty = corner(game, frame);
        let translucent = if improved { MAGENTA } else { RED };

        // A text display in front of the camera, as a server's would arrive.
        let mut display = super::text_displays::RemoteTextDisplay::new(
            super::text_displays::Shows::Text,
            [0.5, 65.5, 3.5],
            0.0,
        );
        display.metadata(&[(23, 0, Value::Text(Tag::String("W".into())))]);
        game.text_displays.insert(1, display);
        game.tick_text_displays(game.clock.runs_normally());
        assert_eq!(corner(game, frame), translucent, "a display's background");

        // No text and no background: nothing drawn.
        let display = game.text_displays.get_mut(&1).unwrap();
        display.metadata(&[
            (23, 0, Value::Text(Tag::String("".into()))),
            (25, 1, Value::Int(0)),
        ]);
        game.tick_text_displays(game.clock.runs_normally());
        assert_eq!(corner(game, frame), empty, "an empty display");

        let display = game.text_displays.get_mut(&1).unwrap();
        display.metadata(&[
            (2, 0, Value::OptText(Some(Tag::String("Tag".into())))),
            (3, 0, Value::Bool(true)),
        ]);
        game.tick_text_displays(game.clock.runs_normally());
        assert_eq!(corner(game, frame), GREEN, "a name tag");

        let display = game.text_displays.get_mut(&1).unwrap();
        display.metadata(&[(0, 0, Value::Byte(0x02))]);
        game.tick_text_displays(game.clock.runs_normally());
        let discrete = if improved { MAGENTA } else { WHITE };
        assert_eq!(corner(game, frame), discrete, "a discrete name tag");

        if !improved {
            let display = game.text_displays.get_mut(&1).unwrap();
            display.metadata(&[
                (23, 0, Value::Text(Tag::String("W".into()))),
                (25, 1, Value::Int(0x8000_00FFu32 as i32)),
            ]);
            game.tick_text_displays(game.clock.runs_normally());
            // Half blue over the tag's white glyphs: (0.5, 0.5, 1); a unorm byte rounds 127.5
            // either way, so one step of tolerance. Drawn the other way round, the background's
            // nearer depth hides the tag and the frame is half blue over the green text: (0, 128,
            // 128).
            let over_the_tag = corner(game, frame);
            let expected = [128u8, 128, 255];
            assert!(
                over_the_tag
                    .iter()
                    .zip(expected)
                    .all(|(got, want)| got.abs_diff(want) <= 1),
                "name tags before texts: {over_the_tag:?}, expected {expected:?} within 1"
            );
        }
    });
}

/// `SubmitNodeCollection.canRenderAsSolid`: with a pack's text, a name tag's `NORMAL` copy is
/// solid only when its text is opaque and its background has no alpha, whatever the background's
/// colour; without one, it stays the native entity layer.
#[test]
fn a_name_tag_copy_is_solid_only_when_opaque_on_no_background() {
    use super::names::name_tag_layer;
    use sunrise_client::gfx::{NAME_TAG_OVERLAY, NAME_TAG_SOLID_OVERLAY};
    assert_eq!(name_tag_layer(true, 0xFF, 0), NAME_TAG_SOLID_OVERLAY);
    assert_eq!(
        name_tag_layer(true, 0xFF, 0x00FF_FFFF),
        NAME_TAG_SOLID_OVERLAY
    );
    assert_eq!(name_tag_layer(true, 0xFF, 0x4000_0000), NAME_TAG_OVERLAY);
    assert_eq!(name_tag_layer(true, 0xFE, 0), NAME_TAG_OVERLAY);
    assert_eq!(name_tag_layer(true, 0x20, 0x4000_0000), NAME_TAG_OVERLAY);
    // Without a pack's text, both copies stay the native entity layer.
    assert_eq!(name_tag_layer(false, 0xFF, 0), 0);
    assert_eq!(name_tag_layer(false, 0x20, 0x4000_0000), 0);
}

/// The picture of the one world view [`a_world_view_draws_the_name_tag`]'s mod asks for, copied
/// out as the renderer hands it back: the buffer, its device and the row length.
static VIEW_PICTURE: std::sync::Mutex<Option<(wgpu::Buffer, wgpu::Device, u32)>> =
    std::sync::Mutex::new(None);

/// A mod's world view (`render_slots::world_views`) from the player's eye toward the display: with
/// a pack's text, a name tag's copies are the pack's materials, which the view draws as the native
/// entity layer: its picture changes when the display is named, as on main.
fn a_world_view_draws_the_name_tag() {
    use sunrise_client_render::render_slots::{self, Slots, WorldView, WorldViewDrawn, WorldViews};
    fn ask(views: &mut WorldViews) {
        let eye = glam::DVec3::new(0.5, 65.5, 0.5);
        views
            .push(WorldView::new(1, eye, 0.0, 0.0, 70.0, [64, 64]))
            .unwrap();
    }
    fn drawn(view: &mut WorldViewDrawn<'_>) {
        let row = (view.size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = view.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * view.size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        view.encoder.copy_texture_to_buffer(
            view.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            view.color.size(),
        );
        *VIEW_PICTURE.lock().unwrap() = Some((buffer, view.device.clone(), row));
    }
    render_slots::install(Slots {
        world_views: ask,
        world_views_hooked: || true,
        world_view_drawn: drawn,
        world_drawn: |_| {},
        hand_drawn: |_| {},
        render_needs: |_| {},
        world_drawn_hooked: || false,
    });
    with_client_game(|game| {
        let pack = super::pack_clouds_frame_tests::pack(&TEXT);
        game.gui = crate::game::UiResources::load(&pack, std::path::Path::new("")).unwrap();
        let _event_loop = frame_scene(game);
        assert!(game.renderer.as_ref().unwrap().draws_pack_world_text());
        let sy = game.world.shape.column().section_index(65).unwrap() as i32;
        game.settings.chunk_fade = 0.0;
        game.renderer
            .as_mut()
            .unwrap()
            .upload((0, sy, 0), &sunrise_client::mesher::SectionMesh::default());
        game.ui.hide_gui = true;
        // The view's picture after a frame.
        let picture = |game: &mut super::Game<'_>| {
            *VIEW_PICTURE.lock().unwrap() = None;
            frame(game).unwrap();
            game.renderer.as_mut().unwrap().wait_submissions();
            let (buffer, device, _) = VIEW_PICTURE.lock().unwrap().take().expect("the view drew");
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            buffer.slice(..).get_mapped_range().unwrap().to_vec()
        };
        // An empty display, with no background: then named.
        let mut display = super::text_displays::RemoteTextDisplay::new(
            super::text_displays::Shows::Text,
            [0.5, 65.5, 3.5],
            0.0,
        );
        display.metadata(&[
            (23, 0, Value::Text(Tag::String("".into()))),
            (25, 1, Value::Int(0)),
        ]);
        game.text_displays.insert(1, display);
        game.tick_text_displays(game.clock.runs_normally());
        let unnamed = picture(game);
        let display = game.text_displays.get_mut(&1).unwrap();
        display.metadata(&[
            (2, 0, Value::OptText(Some(Tag::String("Tag".into())))),
            (3, 0, Value::Bool(true)),
        ]);
        game.tick_text_displays(game.clock.runs_normally());
        let named = picture(game);
        assert!(named != unnamed, "the view shows the name tag");
    });
}
