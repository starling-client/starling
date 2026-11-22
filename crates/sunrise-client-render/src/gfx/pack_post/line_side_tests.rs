//! A pack's `gbuffers_line` drawing the block outline. One widens its line itself, as vanilla's
//! `rendertype_lines.vsh` does: it reads `vaPosition` and `vaNormal` without declaring them (as
//! Complementary Reimagined r5.9.3's `#version 130` line program does) and offsets each vertex
//! to one side of the line's direction on screen by `gl_VertexID % 2`. The others only place
//! the line, and the game widens it. The programs here are original and cut down.
//!
//! The fixture's camera looks along +z (yaw 0) with a 90° field of view, so in the square
//! 64×64 frame a point (x, y, z) ahead of it is at column 32 − 32x/z, row 32 − 32y/z. Each case
//! draws its edges with ends chosen so that no pixel centre lies on a stroke's border, and
//! checks every pixel: the colour of the last stroke over its centre, the clear value of
//! colortex2 (transparent black, spec 6.1.5) outside them all.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};
use wgpu::util::DeviceExt;

const SIZE: u32 = 64;
const OUTSIDE: [u8; 4] = [0; 4];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255; 4];
/// Storage a vertex stage writes: only `a_line_program_that_writes_an_image_is_widened_by_the_game`
/// needs it.
const VERTEX_STORAGE: wgpu::Features = wgpu::Features::VERTEX_WRITABLE_STORAGE;

/// A pack whose line program is `line_vsh` (setting `color`) and draws `color` into
/// colortex2; its final copies colortex2 to the screen.
fn pack(line_vsh: &str) -> sunrise_shaders::Pack {
    let files = [
        ("shaders/gbuffers_line.vsh", line_vsh),
        (
            "shaders/gbuffers_line.fsh",
            "#version 130\nvarying vec4 color;\n/* DRAWBUFFERS:2 */\n\
             void main() { gl_FragData[0] = color; }\n",
        ),
        (
            "shaders/final.vsh",
            "#version 120\nvarying vec2 uv;\n\
             void main() { gl_Position = ftransform(); uv = gl_MultiTexCoord0.xy; }\n",
        ),
        (
            "shaders/final.fsh",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = texture2D(colortex2, uv); }\n",
        ),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, source) in files {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    sunrise_shaders::Pack::from_zip_bytes("line-side", zip.finish().unwrap().into_inner()).unwrap()
}

/// A line program that widens its line as vanilla's `rendertype_lines.vsh` does, to `width`
/// pixels, after `VIEW_SCALE` scales the view (vanilla's shrinks it by 1/256), which leaves the
/// line where it was on screen; then moves the whole line `jitter` pixels (right, up), as a
/// temporal anti-aliasing jitter does. `extra` goes first in `main`.
fn widening_with(width: f32, jitter: [f32; 2], extra: &str) -> String {
    format!(
        "#version 130\nuniform float viewWidth, viewHeight;\nvarying vec4 color;\n\
         #define VIEW_SCALE 0.75\nvec4 place(vec3 p) {{\n    \
         vec4 view = gl_ModelViewMatrix * vec4(p, 1.0);\n    \
         view.xyz *= VIEW_SCALE;\n    return gl_ProjectionMatrix * view;\n}}\n\
         void main() {{\n{extra}    color = vaColor;\n    vec2 screen = vec2(viewWidth, viewHeight);\n    \
         vec4 a = place(vaPosition);\n    vec4 b = place(vaPosition + vaNormal);\n    \
         vec2 d = normalize((b.xy / b.w - a.xy / a.w) * screen);\n    \
         vec2 offset = vec2(-d.y, d.x) * {width:?} / screen;\n    \
         if (offset.x < 0.0) offset = -offset;\n    \
         gl_Position = a + vec4((gl_VertexID % 2 == 0 ? offset : -offset) * a.w, 0.0, 0.0);\n    \
         gl_Position.xy += vec2({:?}, {:?}) * 2.0 / screen * gl_Position.w;\n}}\n",
        jitter[0], jitter[1]
    )
}

fn widening(width: f32) -> String {
    widening_with(width, [0.0; 2], "")
}

/// A line program that only transforms its line, then moves it `up` pixels up the screen (a
/// temporal anti-aliasing jitter's kind of move).
fn transforming(up: f32) -> String {
    format!(
        "#version 130\nuniform float viewHeight;\nvarying vec4 color;\nvoid main() {{\n    \
         color = vaColor;\n    gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n    \
         gl_Position.y += {up:?} * 2.0 / viewHeight * gl_Position.w;\n}}\n"
    )
}

/// A line program that only transforms its line, then maps the frame into its top left
/// quarter, as a pack that renders the world into part of its buffers does.
const INTO_A_QUARTER: &str = "#version 130\nvarying vec4 color;\nvoid main() {\n    \
     color = vaColor;\n    gl_Position = ftransform();\n    \
     gl_Position.xy = gl_Position.xy * 0.5 + vec2(-0.5, 0.5) * gl_Position.w;\n}\n";

/// A line program that only transforms its line, through the model-view and the projection
/// apart: its rounding isn't that of the game's product of the two.
const TRANSFORMING_IN_TWO_STEPS: &str = "#version 130\nvarying vec4 color;\nvoid main() {\n    \
     color = vaColor;\n    \
     gl_Position = gl_ProjectionMatrix * (gl_ModelViewMatrix * vec4(vaPosition, 1.0));\n}\n";

/// A line program that reads its vertex index and the line's direction into a varying, as a
/// widening one does, and still only transforms its line.
const READS_INDEX_AND_NORMAL: &str = "#version 130\nvarying vec4 color;\nvarying float side;\n\
     void main() {\n    color = vaColor;\n    side = float(gl_VertexID % 2) * vaNormal.x;\n    \
     gl_Position = ftransform();\n}\n";

/// An edge's ends, camera-relative.
type Edge = ([f32; 3], [f32; 3]);

/// An edge drawn in a case: its name, its ends, and the ends of the part of it on screen, in
/// pixels (column, row), before the pack moves it.
type Case = (&'static str, Edge, [[f32; 2]; 2]);

/// `edge` at z = 2, its ends on screen where the camera puts them.
const fn ahead(name: &'static str, [x0, y0]: [f32; 2], [x1, y1]: [f32; 2]) -> Case {
    (
        name,
        ([x0, y0, 2.0], [x1, y1, 2.0]),
        [
            [32.0 - 16.0 * x0, 32.0 - 16.0 * y0],
            [32.0 - 16.0 * x1, 32.0 - 16.0 * y1],
        ],
    )
}

/// From (-0.5, 0.25) to (0.5, 0.25): columns 24..40 at row 28; from (-0.25, -0.5) to
/// (-0.25, 0.5): rows 24..40 at column 36; from (-0.4, -0.4) to (0.4, 0.4): from (38.4, 38.4)
/// to (25.6, 25.6), across the diagonal.
const AHEAD: [Case; 3] = [
    ahead("horizontal", [-0.5, 0.25], [0.5, 0.25]),
    ahead("vertical", [-0.25, -0.5], [-0.25, 0.5]),
    ahead("diagonal", [-0.4, -0.4], [0.4, 0.4]),
];

/// Edges from a start at clip w 1e-6, just ahead of the camera, and from one just behind it
/// (w -1e-3), both to (-0.4, -0.4, 2) at (38.4, 38.4), far off the frame (the one behind
/// mirrored): every point of each has x = y, so on screen each runs along the diagonal from
/// (38.4, 38.4) up and left past the frame's corner, reached at z 0.91, ahead of the near plane.
const NEAR_ZERO: [Case; 2] = [
    (
        "start at w ~ 0",
        ([2.0, 2.0, 1e-6], [-0.4, -0.4, 2.0]),
        [[-1000.0, -1000.0], [38.4, 38.4]],
    ),
    (
        "start behind the camera",
        ([2.0, 2.0, -1e-3], [-0.4, -0.4, 2.0]),
        [[-1000.0, -1000.0], [38.4, 38.4]],
    ),
];

/// One stroke of the outline: its ARGB colour and its width in pixels by the game's measure.
type Stroke = (u32, f32);

/// The outline buffer: each stroke over `edge`, in order, as the game writes it.
fn outline(gpu: &Fixture, edge: Edge, strokes: &[Stroke]) -> wgpu::Buffer {
    let mut bytes = Vec::new();
    for &(argb, width) in strokes {
        for v in edge.0.into_iter().chain(edge.1) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&argb.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
    }
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("line side outline"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        })
}

/// A stroke of the expected picture: how far it reaches to each side of its edge, in pixels,
/// and its colour.
type Band = (f32, [u8; 4]);

/// Where the pack puts a point of the screen (column c, row r), in pixels: column
/// `p[0] c + p[1] r + p[2]`, row `p[3] c + p[4] r + p[5]`.
type Place = [f32; 6];

/// The pack leaves the line where the camera puts it.
const IN_PLACE: Place = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// The pack turns the frame a quarter turn anticlockwise about its centre (`turned`): (c, r)
/// goes to (r, 64 - c), a pixel centre to a pixel centre.
const TURNED: Place = [0.0, 1.0, 0.0, -1.0, 0.0, SIZE as f32];

/// `line_vsh` with the frame turned a quarter turn anticlockwise last (clip x, y to -y, x),
/// after the pack placed and, for a widening one, widened its line: a horizontal edge drawn
/// vertical, a vertical one horizontal.
fn turned(line_vsh: &str) -> String {
    let end = line_vsh.rfind('}').expect("main's end");
    format!(
        "{}    gl_Position.xy = vec2(-gl_Position.y, gl_Position.x);\n{}",
        &line_vsh[..end],
        &line_vsh[end..]
    )
}

/// The colour expected at pixel (x, y): that of the last of `bands` whose rectangle holds the
/// pixel's centre, `OUTSIDE` for none. Each band is the edge on screen `ends`, placed by
/// `place`, reaching its distance to each side.
fn expected(x: u32, y: u32, ends: [[f32; 2]; 2], place: Place, bands: &[Band]) -> [u8; 4] {
    let at = |[c, r]: [f32; 2]| {
        [
            place[0] * c + place[1] * r + place[2],
            place[3] * c + place[4] * r + place[5],
        ]
    };
    let (a, b) = (at(ends[0]), at(ends[1]));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = (dx * dx + dy * dy).sqrt();
    let (px, py) = (x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]);
    let along = (px * dx + py * dy) / length;
    let across = (px * dy - py * dx).abs() / length;
    bands
        .iter()
        .rev()
        .find(|(half, _)| (0.0..length).contains(&along) && across < *half)
        .map_or(OUTSIDE, |&(_, colour)| colour)
}

/// A pixel that isn't as expected: the edge's name, x, y, actual and expected colours.
type Wrong = (&'static str, u32, u32, [u8; 4], [u8; 4]);

/// The pixels that differ from `expected` when the pack whose line program is `line_vsh` draws
/// `strokes` over each edge of `cases`; `None` without a GPU.
fn wrong_pixels_of(
    line_vsh: &str,
    strokes: &[Stroke],
    cases: &[Case],
    place: Place,
    bands: &[Band],
) -> Option<Vec<Wrong>> {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Fixture::new()?;
    // A line program writing an image needs storage written by a vertex stage, as the game's
    // device has it where the adapter does (`surface_setup.rs`); only that case needs it.
    let storage = gpu.adapter.features() & VERTEX_STORAGE;
    let (device, queue) = pollster::block_on(gpu.adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: storage,
        required_limits: gpu.adapter.limits(),
        ..Default::default()
    }))
    .expect("GPU device for the line side cases");
    let gpu = Fixture {
        adapter: gpu.adapter,
        device,
        queue,
    };
    let pack = pack(line_vsh);
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let inputs = sunrise_shaders::GameInputs {
        lines: Some(crate::gfx::pack_lines::inputs()),
        ..Default::default()
    };
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    // The line program, not a fallback, draws the outline.
    let lines = post.lines.as_ref().map(|p| p.name.as_str());
    assert_eq!(lines, Some("gbuffers_line"), "{:?}", post.notes);
    let mut post = gpu.try_build(post).expect("the pack runs");
    assert!(post.has_lines(), "{:?}", post.notes);
    let mut wrong = Vec::new();
    for &(name, edge, ends) in cases {
        post.set_lines(Some(super::Lines {
            edges: outline(&gpu, edge, strokes),
            count: 1,
            strokes: strokes.len() as u32,
        }));
        let pixels = gpu.render(&mut post, [SIZE, SIZE]);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let actual = pixels[(y * SIZE + x) as usize];
                let expected = expected(x, y, ends, place, bands);
                if actual != expected {
                    wrong.push((name, x, y, actual, expected));
                }
            }
        }
    }
    Some(wrong)
}

/// `wrong_pixels_of` the three edges ahead, the line moved `up` pixels up the screen.
fn wrong_pixels(line_vsh: &str, strokes: &[Stroke], up: f32, bands: &[Band]) -> Option<Vec<Wrong>> {
    wrong_pixels_of(
        line_vsh,
        strokes,
        &AHEAD,
        [1.0, 0.0, 0.0, 0.0, 1.0, -up],
        bands,
    )
}

fn assert_none_wrong(wrong: Option<Vec<Wrong>>) {
    let Some(wrong) = wrong else {
        return;
    };
    assert!(
        wrong.is_empty(),
        "{} pixels wrong, first {:?}",
        wrong.len(),
        &wrong[..wrong.len().min(8)]
    );
}

/// A line program that widens its line by vertex index, 4 pixels wide (2 to each side), under
/// a view scale: its band is exactly the pack's, so the game added no width of its own, and
/// both triangles of each strip have their sides (a vertex's `gl_VertexID % 2` is its
/// corner's side).
#[test]
#[ignore = "daily"]
fn a_line_program_that_widens_its_line_draws_its_own_band() {
    let wrong = wrong_pixels(&widening(4.0), &[(0xFFFF_FFFF, 6.0)], 0.0, &[(2.0, WHITE)]);
    assert_none_wrong(wrong);
}

/// A line program that only places the line (as BSL's `gbuffers_basic` does) is widened by the
/// game to its 6 pixels, 3 to each side.
#[test]
#[ignore = "daily"]
fn a_line_program_that_only_places_its_line_is_widened_by_the_game() {
    let wrong = wrong_pixels(
        &transforming(0.0),
        &[(0xFFFF_FFFF, 6.0)],
        0.0,
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line program that reads its vertex index and `vaNormal` into a varying but only
/// transforms its line is widened by the game: reading them doesn't widen the line.
#[test]
#[ignore = "daily"]
fn a_line_program_reading_its_index_and_normal_but_only_placing_its_line_is_widened() {
    let strokes = [(0xFFFF_FFFF, 6.0)];
    let wrong = wrong_pixels(READS_INDEX_AND_NORMAL, &strokes, 0.0, &[(3.0, WHITE)]);
    assert_none_wrong(wrong);
}

/// A line the pack moves 2.3 pixels up the screen, its two sides alike, is still the game's to
/// widen: 3 pixels to each side of where the pack put it.
#[test]
#[ignore = "daily"]
fn a_line_the_pack_moves_is_widened_by_the_game() {
    let wrong = wrong_pixels(
        &transforming(2.3),
        &[(0xFFFF_FFFF, 6.0)],
        2.3,
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line program that maps the frame into its top left quarter (each point at half its
/// column and row) is widened by the game, at the game's 6 pixels: every vertex moved, both
/// sides of each end alike.
#[test]
#[ignore = "daily"]
fn a_line_program_that_maps_the_frame_into_a_quarter_is_widened_by_the_game() {
    let strokes = [(0xFFFF_FFFF, 6.0)];
    let wrong = wrong_pixels_of(
        INTO_A_QUARTER,
        &strokes,
        &AHEAD,
        [0.5, 0.0, 0.0, 0.0, 0.5, 0.0],
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line the pack widens to 0.6 pixels keeps the pack's width (0.3 to each side, which no
/// pixel centre of the horizontal and vertical edges is within, and only those on the
/// diagonal's own line).
#[test]
#[ignore = "daily"]
fn a_line_widened_by_a_little_keeps_the_packs_width() {
    let wrong = wrong_pixels(&widening(0.6), &[(0xFFFF_FFFF, 6.0)], 0.0, &[(0.3, WHITE)]);
    assert_none_wrong(wrong);
}

/// A line the pack widens to 0.4 pixels, less than half a pixel, keeps the pack's width too:
/// 0.2 to each side, within which only the pixel centres on the diagonal's own line are.
/// Any width the pack gives its line is the pack's.
#[test]
#[ignore = "daily"]
fn a_line_widened_by_less_than_half_a_pixel_keeps_the_packs_width() {
    let wrong = wrong_pixels(&widening(0.4), &[(0xFFFF_FFFF, 6.0)], 0.0, &[(0.2, WHITE)]);
    assert_none_wrong(wrong);
}

/// A line program whose two corners of an end land 1/4096 pixels apart, a quarter of the
/// 1/1024 px floor (the rounding a driver may leave between runs of a pack that only places its
/// line), is widened by the game: 3 pixels to each side, give or take the pack's 1/8192, which
/// no pixel centre is near.
#[test]
#[ignore = "daily"]
fn a_line_spread_below_the_rounding_floor_is_widened_by_the_game() {
    let wrong = wrong_pixels(
        &widening(1.0 / 4096.0),
        &[(0xFFFF_FFFF, 6.0)],
        0.0,
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line program that widens its line to 4 pixels, then turns the frame a quarter turn (each
/// edge drawn across its own direction before): its band is exactly the pack's, 2 to each side
/// of the edge where the pack drew it. The game measures the pack's width across the edge as
/// drawn, not as the camera puts it.
#[test]
#[ignore = "daily"]
fn a_line_program_that_widens_and_turns_its_line_draws_its_own_band() {
    let wrong = wrong_pixels_of(
        &turned(&widening(4.0)),
        &[(0xFFFF_FFFF, 6.0)],
        &AHEAD,
        TURNED,
        &[(2.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line program that only places its line and turns the frame a quarter turn is widened by
/// the game to its 6 pixels, 3 to each side, across the edge where the pack drew it: across
/// the camera's direction, it would be along the drawn edge, with no area.
#[test]
#[ignore = "daily"]
fn a_line_program_that_turns_its_line_is_widened_across_it_by_the_game() {
    let wrong = wrong_pixels_of(
        &turned(&transforming(0.0)),
        &[(0xFFFF_FFFF, 6.0)],
        &AHEAD,
        TURNED,
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A widening line program whose globals it reads before writing them (one with an
/// initializer, one without, which a function of its adds to, and one whose initializer reads
/// that one) draws the line its single run draws, 4 pixels wide, though the game runs it three
/// times for each vertex: each run starts from the globals' first values.
#[test]
#[ignore = "daily"]
fn a_line_program_reading_its_globals_draws_its_single_runs_line() {
    let vsh = widening_with(4.0, [0.0; 2], "    t *= 2.0;\n    count();\n")
        .replacen(
            "varying vec4 color;\n",
            "varying vec4 color;\nfloat t = 0.5;\nfloat acc;\nfloat first = acc + 1.0;\n\
             void count() { acc += 1.0; }\n",
            1,
        )
        .replacen("* 4.0 / screen", "* 4.0 * t * acc * first / screen", 1);
    let wrong = wrong_pixels(&vsh, &[(0xFFFF_FFFF, 6.0)], 0.0, &[(2.0, WHITE)]);
    assert_none_wrong(wrong);
}

/// A line the pack widens to 2 pixels (1 to each side, as Complementary Reimagined's do) and
/// then jitters 0.4 pixels right and up, across each edge, keeps the pack's width on both
/// sides of where the jitter put it: the jitter moves both of an end's sides alike.
#[test]
#[ignore = "daily"]
fn a_widened_line_with_a_jitter_keeps_the_packs_width_on_both_sides() {
    let jitter = [0.4, 0.4];
    let wrong = wrong_pixels_of(
        &widening_with(2.0, jitter, ""),
        &[(0xFFFF_FFFF, 6.0)],
        &AHEAD,
        [1.0, 0.0, 0.4, 0.0, 1.0, -0.4],
        &[(1.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line whose start is at clip w near 0 or behind the camera, under a line program that
/// only transforms it, in two steps: the game widens all of it to its 6 pixels, the part on
/// screen too (`NEAR_ZERO`). Each end's corners have the same `w`, however near 0.
#[test]
#[ignore = "daily"]
fn a_line_with_an_end_near_or_behind_the_camera_keeps_the_games_width() {
    let strokes = [(0xFFFF_FFFF, 6.0)];
    let wrong = wrong_pixels_of(
        TRANSFORMING_IN_TWO_STEPS,
        &strokes,
        &NEAR_ZERO,
        IN_PLACE,
        &[(3.0, WHITE)],
    );
    assert_none_wrong(wrong);
}

/// A line program that writes an image is run once per vertex, so its
/// writes aren't repeated, and the game can't tell that it widens its line: it widens it too,
/// by its 6 pixels on top of the pack's 4. The pack puts each corner on the side the game
/// widens it to (an even corner to the right of the line's direction on screen), so the two
/// add up: 3 + 2 to each side. Skipped on an adapter without vertex writable storage.
#[test]
#[ignore = "daily"]
fn a_line_program_that_writes_an_image_is_widened_by_the_game() {
    {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some(gpu) = Fixture::new() else { return };
        if !gpu.adapter.features().contains(VERTEX_STORAGE) {
            eprintln!(
                "a_line_program_that_writes_an_image_is_widened_by_the_game skipped: \
                 no vertex writable storage"
            );
            return;
        }
    }
    let extra = "    imageStore(colorimg5, ivec2(gl_VertexID, 0), vec4(1.0));\n";
    let vsh = widening_with(4.0, [0.0; 2], extra)
        .replacen(
            "varying vec4 color;\n",
            "varying vec4 color;\nlayout(rgba8) writeonly uniform image2D colorimg5;\n",
            1,
        )
        .replacen(
            "if (offset.x < 0.0) offset = -offset;",
            "offset = -offset;",
            1,
        );
    let wrong = wrong_pixels(&vsh, &[(0xFFFF_FFFF, 6.0)], 0.0, &[(5.0, WHITE)]);
    assert_none_wrong(wrong);
}

/// The high contrast outline (a black stroke 6 pixels wide, then the bright one, 2 wide, over
/// it) under a pack that widens its lines to 4 pixels: the bright stroke keeps the pack's
/// width, 2 to each side, and the black one reaches 2 more to each side, (6 - 2) / 2, as the
/// game draws it 4 pixels wider than the bright one.
#[test]
#[ignore = "daily"]
fn a_high_contrast_outline_keeps_its_black_border_under_a_pack_that_widens_its_lines() {
    let strokes = [(0xFF00_0000, 6.0), (0xFFFF_FFFF, 2.0)];
    let bands = [(4.0, BLACK), (2.0, WHITE)];
    assert_none_wrong(wrong_pixels(&widening(4.0), &strokes, 0.0, &bands));
}

/// The same high contrast outline under a pack that widens its lines to 4 pixels and turns the
/// frame a quarter turn: the black stroke's extra 2 pixels to each side are across the edge as
/// drawn, so it still shows around the bright one, as without the turn.
#[test]
#[ignore = "daily"]
fn a_high_contrast_outline_keeps_its_black_border_under_a_pack_that_turns_its_lines() {
    let strokes = [(0xFF00_0000, 6.0), (0xFFFF_FFFF, 2.0)];
    let bands = [(4.0, BLACK), (2.0, WHITE)];
    let wrong = wrong_pixels_of(&turned(&widening(4.0)), &strokes, &AHEAD, TURNED, &bands);
    assert_none_wrong(wrong);
}

/// The same high contrast outline under a pack that only places its lines: each stroke at the
/// game's width, 3 and 1 pixels to each side.
#[test]
#[ignore = "daily"]
fn a_high_contrast_outline_under_a_pack_that_places_its_lines_has_the_games_widths() {
    let strokes = [(0xFF00_0000, 6.0), (0xFFFF_FFFF, 2.0)];
    let bands = [(3.0, BLACK), (1.0, WHITE)];
    assert_none_wrong(wrong_pixels(&transforming(0.0), &strokes, 0.0, &bands));
}
