//! Sign text reaches the buffers the pack's block program writes (#4691), over a board of the block
//! program or of the world; original fixture, no pack assets.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::pack_draw::EntitySpan;
use crate::gfx::{TEXT_NORMAL_SOLID_OVERLAY, TEXT_POLYGON_OFFSET_SOLID_OVERLAY, Vertex};
use crate::hud::HudBatch;
use sunrise_shaders::render_stage;
use wgpu::TextureFormat as T;

/// The board's colour and the two text modes', as the vertices carry them (RGBA bytes).
const BOARD: [u8; 4] = [200, 40, 0, 255];
const NORMAL_TEXT: [u8; 4] = [0, 160, 80, 255];
const OFFSET_TEXT: [u8; 4] = [60, 0, 220, 255];
/// The world target's clear colour (`Gpu::clear`: 0.0625 grey) as colortex0 holds it.
const WORLD_BOARD: [u8; 4] = [16, 16, 16, 255];

/// A quad over strip `strip` (4 pixels wide; `None`: the whole screen) at clip depth `z`.
fn quad(strip: Option<u32>, z: f32, color: [u8; 4], overlay: u32) -> [Vertex; 4] {
    let (x0, x1) = strip.map_or((-1.0, 1.0), |s| {
        let x = -1.0 + s as f32 * 0.5;
        (x, x + 0.5)
    });
    [[x0, -1.0], [x1, -1.0], [x1, 1.0], [x0, 1.0]].map(|[x, y]| Vertex {
        pos: [x, y, z],
        layer: overlay,
        color: u32::from_le_bytes(color),
        ..bytemuck::Zeroable::zeroed()
    })
}

#[test]
#[ignore = "daily"]
fn sign_text_shows_in_the_buffer_final_shows() {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("an executed adapter is required for this control");
    // The board at clip z 0.5, window depth 0.75 (the translated vertex stage's `(z + w) / 2`).
    // `behind` is 2^-21 further in clip z, 2^-22 in window depth: four ulps of 0.75, fewer than
    // the 10 units (here ulps) of vanilla's polygon offset. It brings POLYGON_OFFSET text in
    // front; NORMAL text stays behind.
    let behind = 0.5 + 4.0 * f32::EPSILON;
    let vertices: Vec<Vertex> = [
        quad(None, 0.5, BOARD, 0),
        // Strip 0: NORMAL text on the board's plane, drawn after it.
        quad(Some(0), 0.5, NORMAL_TEXT, TEXT_NORMAL_SOLID_OVERLAY),
        // Strip 1: NORMAL text just behind the board.
        quad(Some(1), behind, NORMAL_TEXT, TEXT_NORMAL_SOLID_OVERLAY),
        // Strip 2: POLYGON_OFFSET text just behind the board.
        quad(
            Some(2),
            behind,
            OFFSET_TEXT,
            TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
        ),
        // Strip 3: the board alone.
    ]
    .concat();
    let spans = [EntitySpan {
        stage: render_stage("BLOCK_ENTITIES"),
        id: 17,
        ..Default::default()
    }];
    // (the block program's draw-buffer comment, the buffer `final` shows, whether the board is
    // the world's, as a sign's board is a terrain block model drawn before block entities: its
    // colour in colortex0 and its depth in the depth buffer, from the world target).
    for (drawbuffers, shown, world_board) in [
        ("", "colortex0", false),
        ("/* DRAWBUFFERS:1 */\n", "colortex1", false),
        ("", "colortex0", true),
    ] {
        let files = vec![
            ("final.vsh".into(), VERTEX.into()),
            (
                "final.fsh".into(),
                format!(
                    "#version 120\nuniform sampler2D {shown}; varying vec2 uv; void main(){{gl_FragColor=vec4(texture2D({shown},uv).rgb,1.0);}}"
                ),
            ),
            (
                "gbuffers_block.vsh".into(),
                "#version 120\nvarying vec4 c; void main(){gl_Position=gl_Vertex; c=gl_Color;}"
                    .into(),
            ),
            (
                "gbuffers_block.fsh".into(),
                format!(
                    "#version 120\n{drawbuffers}varying vec4 c; void main(){{gl_FragData[0]=vec4(c.rgb,1.0);}}"
                ),
            ),
        ];
        let color = gpu.texture(T::Rgba8Unorm);
        let depth = gpu.texture(T::Depth32Float);
        let mut input = super::super::pack_entities::inputs();
        input.body = "hz_Vertex=vec4(hz_v_pos,1.0); hz_Color=hz_v_color;".into();
        let mut post = gpu.load(
            files,
            World {
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
            },
            sunrise_shaders::GameInputs {
                entities: Some(input),
                ..Default::default()
            },
        );
        let hud = HudBatch::scratch(SIZE);
        // Reversed depth: the board's window depth 0.75 is 0.25 in the world target.
        gpu.clear(&color, &depth, if world_board { 0.25 } else { 0.0 });
        let vertices = if world_board {
            &vertices[4..]
        } else {
            &vertices[..]
        };
        let mut rest = Vec::new();
        post.prepare_entities(
            &gpu.device,
            &gpu.queue,
            vertices,
            &spans,
            [0.; 4],
            [0.; 4],
            &mut rest,
            None,
            (&[], &[]),
        );
        assert!(
            rest.is_empty(),
            "{shown}: the pack draws the board and its text"
        );
        let pixels = gpu.render(
            &mut post,
            &color,
            &depth,
            &fixture_frame(&hud),
            &mut |_, _, _, _| {},
        );
        // The world's clear colour, 1/16 grey, stands for the terrain board.
        let board = if world_board { WORLD_BOARD } else { BOARD };
        let expected = [NORMAL_TEXT, board, OFFSET_TEXT, board];
        let actual: Vec<[u8; 4]> = (0..4)
            .map(|strip| pixels[(SIZE[1] / 2 * SIZE[0] + strip * 4 + 2) as usize])
            .collect();
        assert_eq!(
            actual, expected,
            "{shown}, board from the world {world_board}: strips 0 to 3"
        );
    }
}
