//! What a pack's `core/clouds` shader reads, against 26.3's `CloudRenderer`: the expected faces
//! are worked out by hand from `buildMesh`, `buildExtrudedCell` and `encodeFace` (the first two
//! cases), or by an independent transcription of them in Python written for these tests (the
//! wrapping case), never by `pack::faces` itself.
use super::pack::{faces, faces_key, info};
use super::*;

/// The input over a `texture`, the camera at the corner of cell 0, 0 (at time 0) and
/// `above` the clouds' bottom.
fn input(texture: &(u32, u32, Vec<bool>), above: f64, range: i32, fast: bool) -> Input<'_> {
    Input {
        texture,
        uv: [0.0; 2],
        // Vanilla's height is a float, and it adds 3.9600000381469727 to z: the camera at x 0,
        // z -3.9600000381469727 sits at the corner of cell 0, 0, `above` the layer's bottom.
        camera: glam::DVec3::new(
            0.0,
            f64::from(CLOUD_HEIGHT) + above,
            -3.960_000_038_146_972_7,
        ),
        time: 0,
        partial_tick: 0.0,
        range,
        fast,
        color: [1.0, 1.0, 1.0, 0.8],
        height: CLOUD_HEIGHT,
    }
}

/// One filled cell east of the camera's, the camera inside the layer: up and down (inside, so
/// both), west (its west neighbour empty, x > 0), then its six inward faces, x 1 halved to 0
/// with bit 7 set. `CloudInfo`: the colour through 8 bits (0.8 is 204 of 255), the layer's
/// bottom 2 below the camera, cells 12 by 4 by 12.
#[test]
fn fancy_faces_follow_vanillas_build_mesh() {
    let mut filled = vec![false; 16];
    filled[1] = true;
    let texture = (4, 4, filled);
    let input = input(&texture, 2.0, 1, false);
    assert_eq!(
        faces(&input),
        [
            0, 0, -127, 0, 0, -128, 0, 0, -124, // up, down, west
            0, 0, -112, 0, 0, -111, 0, 0, -110, 0, 0, -109, 0, 0, -108, 0, 0, -107, // inward
        ]
    );
    let info = info(&input);
    let f = |at: usize| f32::from_le_bytes(info[at..at + 4].try_into().unwrap());
    assert_eq!([f(0), f(4), f(8), f(12)], [1.0, 1.0, 1.0, 204.0 / 255.0]);
    assert_eq!([f(16), f(20), f(24)], [0.0, -2.0, 0.0]);
    assert_eq!([f(32), f(36), f(40)], [12.0, 4.0, 12.0]);
}

/// The same cell with the camera above the layer (its bottom 5 below the camera, so its top 1
/// below): no down face, the rest as inside it.
#[test]
fn above_the_layer_a_cell_has_no_down_face() {
    let mut filled = vec![false; 16];
    filled[1] = true;
    let texture = (4, 4, filled);
    assert_eq!(
        faces(&input(&texture, 5.0, 1, false)),
        [
            0, 0, -127, 0, 0, -124, // up, west
            0, 0, -112, 0, 0, -111, 0, 0, -110, 0, 0, -109, 0, 0, -108, 0, 0, -107, // inward
        ]
    );
    // Level with the layer's top (`relativeTopY` 0 is not above it): the down face is there.
    assert_eq!(faces(&input(&texture, 4.0, 1, false))[3..6], [0, 0, -128]);
}

/// Fast clouds: one face a cell, down with the top colour (32). The cell west of the camera's,
/// x -1: halved to -1, bit 7 set, so its third byte is 32 | 128, -96 as a signed byte.
#[test]
fn flat_faces_are_one_down_face_with_the_top_colour() {
    let mut filled = vec![false; 16];
    filled[3] = true;
    let texture = (4, 4, filled);
    assert_eq!(faces(&input(&texture, 2.0, 1, true)), [-1, 0, -96]);
}

/// A 4 by 2 texture with cells 1 and 2 of its first row filled, the camera in cell 0 below the
/// layer, range 3 (4 cells): every face in buildMesh's ring order. Vanilla's neighbours wrap
/// east and west by the texture's height: cell 1's east neighbour is cell 0 (empty), so the
/// copy of cell 1 three cells west gets an east face (-2, 0, -123) that the texture's width
/// would not give it.
#[test]
fn faces_wrap_the_texture_as_vanilla_does() {
    let mut filled = vec![false; 8];
    filled[1] = true;
    filled[2] = true;
    let texture = (4, 2, filled);
    #[rustfmt::skip]
    let expected = [
        0, 0, -128, 0, 0, -124, 0, 0, -112, 0, 0, -111, 0, 0, -110, 0, 0, -109, 0, 0, -108,
        0, 0, -107, -1, 0, 0, 1, 0, 0, -2, 0, -128, -2, 0, -123, 0, -1, -128, 0, -1, -125, 0, -1,
        -124, 0, 1, -128, 0, 1, -126, 0, 1, -124, -1, -1, 0, -1, -1, 3, -1, 1, 0, -1, 1, 2, 1, -1,
        0, 1, -1, 3, 1, 1, 0, 1, 1, 2, -2, -1, -128, -2, -1, -125, -2, -1, -123, -2, 1, -128, -2,
        1, -126, -2, 1, -123,
    ];
    assert_eq!(faces(&input(&texture, -5.0, 3, false)), expected);
}

/// The textures vanilla 26.3 reads past, as its own load reported them (the bench mod's hook
/// loading each through `CloudRenderer.prepare`, `.local/clouddump`): `NativeImage.getPixel`'s
/// message for the first pixel it asks for outside the image. A tall one with filled cells inside
/// only, a square one with them at both edges, and a wide one load.
#[test]
fn a_texture_vanilla_reads_past_is_named_by_the_pixel_it_asks_for() {
    let texture = |w: u32, h: u32, cells: &[[u32; 2]]| {
        let mut filled = vec![false; (w * h) as usize];
        for [x, y] in cells {
            filled[(y * w + x) as usize] = true;
        }
        (w, h, filled)
    };
    let refused: [(u32, u32, &[[u32; 2]], &str); 4] = [
        // A cell in the first column: its west neighbour wraps to column 5.
        (
            4,
            6,
            &[[0, 2], [2, 1]],
            "(5, 2) outside of image bounds (4, 6)",
        ),
        // A cell in the last column: its east neighbour is column 4.
        (4, 6, &[[3, 4]], "(4, 4) outside of image bounds (4, 6)"),
        // Both neighbours of a one-column texture's cell are outside: the east one is read first.
        (1, 6, &[[0, 0]], "(1, 0) outside of image bounds (1, 6)"),
        // Two cells read outside: the first row's first.
        (
            4,
            6,
            &[[3, 0], [0, 1]],
            "(4, 0) outside of image bounds (4, 6)",
        ),
    ];
    for (w, h, cells, java) in refused {
        let pixel = outside_image(&texture(w, h, cells));
        assert_eq!(pixel.map(|p| p.to_string()).as_deref(), Some(java));
    }
    for (name, w, h, cells) in JAVA_TEXTURES {
        assert_eq!(outside_image(&texture(*w, *h, cells)), None, "{name}");
    }
}

/// A tall cloud texture vanilla reads past fails the load with vanilla's message, for the
/// reload to fail as vanilla's does; a tall one read inside, a wide one and a square one load,
/// their cells filled from alpha 10 (`CloudRenderer.isCellEmpty`).
#[test]
fn a_cloud_texture_vanilla_reads_past_fails_the_load() {
    use crate::resource_source::{Resource, ResourceSource};
    let source_none = ResourceSource::default;
    // Pixels as x, y and alpha; the rest are transparent.
    let png = |w: u32, h: u32, cells: &[[u32; 3]]| {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for [x, y, alpha] in cells {
            rgba[((y * w + x) * 4 + 3) as usize] = *alpha as u8;
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
        out
    };
    let source = |bytes: Vec<u8>| {
        ResourceSource::indexed(
            [(
                "assets/minecraft/textures/environment/clouds.png".to_owned(),
                Resource::bytes(bytes),
            )]
            .into_iter()
            .collect(),
        )
    };
    let refused = source(png(4, 6, &[[0, 2, 255]]));
    assert_eq!(
        load(&refused).map_err(|e| e.to_string()),
        Err(
            "assets/minecraft/textures/environment/clouds.png: (5, 2) outside of image bounds (4, 6)"
                .to_owned()
        )
    );
    let tall_inner = source(png(4, 6, &[[1, 0, 255]]));
    let (w, h, filled) = load(&tall_inner).unwrap();
    assert_eq!(
        (w, h, filled.iter().filter(|f| **f).count(), filled[1]),
        (4, 6, 1, true)
    );
    let wide = source(png(6, 4, &[[0, 2, 255], [5, 1, 255]]));
    assert_eq!(load(&wide).unwrap().2.iter().filter(|f| **f).count(), 2);
    // A cell with alpha under 10 is empty.
    let square = source(png(2, 2, &[[0, 0, 10], [1, 0, 9]]));
    assert_eq!(load(&square), Ok((2, 2, vec![true, false, false, false])));
    // No texture: no cloud.
    assert_eq!(load(&source_none()), Ok((1, 1, vec![false])));
}

/// Faces kept while `faces_key` holds are the ones built afresh: along a path through cells and
/// in and out of the layer, an unchanged key never hides changed faces, and the path does
/// change cells and sides.
#[test]
fn the_faces_key_holds_only_while_the_faces_do() {
    let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
    let mut kept: Option<(pack::FacesKey, Vec<i32>)> = None;
    let (mut rebuilt, mut kept_frames) = (0, 0);
    for step in 0..400i32 {
        let t = f64::from(step);
        let input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(
                t * 0.37,
                f64::from(CLOUD_HEIGHT) - 6.0 + (t * 0.05) % 14.0,
                t * 0.37,
            ),
            time: i64::from(step),
            partial_tick: 0.0,
            range: 3,
            fast: false,
            color: [1.0; 4],
            height: CLOUD_HEIGHT,
        };
        let key = faces_key(&input);
        let faces = faces(&input);
        match &kept {
            Some((k, f)) if *k == key => {
                assert_eq!(*f, faces, "step {step}: the key held but the faces changed");
                kept_frames += 1;
            }
            _ => rebuilt += 1,
        }
        kept = Some((key, faces));
    }
    assert!(
        kept_frames > 300 && rebuilt > 10,
        "{kept_frames} kept, {rebuilt} rebuilt"
    );
}

// ---- Java 26.3's own output -----------------------------------------------------------------

/// Cloud textures `CloudRenderer.prepare` loaded in vanilla 26.3 (`name`, width, height, filled
/// cells; the rest clear).
const JAVA_TEXTURES: &[(&str, u32, u32, &[[u32; 2]])] = &[
    (
        "wide",
        6,
        4,
        &[[1, 0], [2, 0], [5, 1], [0, 2], [3, 3], [4, 3]],
    ),
    ("tall_inner", 4, 6, &[[1, 0], [2, 1], [1, 3], [2, 5]]),
    ("square_edge", 4, 4, &[[0, 1], [3, 2]]),
];

/// Frames `CloudRenderer.prepare` took in vanilla 26.3: the ARGB colour, fancy, the camera, the
/// game time and the partial tick, at the overworld's cloud height (192.33) and range.
///
/// The partial tick is Sunrise's double; vanilla's is a float. The last frame's is a hair above
/// the float vanilla took (1.2207031e-4f, 2^-13 exactly): narrowed first and added to the game
/// time's remainder in float, as vanilla adds them, it ties to the remainder itself; summed in
/// double it would round up a float step.
const JAVA_FRAMES: &[JavaFrame] = &[
    (0xFFFF_FFFF, true, 1, [37.25, 200.5, -5.5], 12345, 0.37),
    (0x00FF_0001, false, 1, [-3.75, 194.0, 30.125], 0, 0.0),
    (0xFE7F_80FF, true, 2, [0.5, 150.25, 0.5], 999_999, 0.99),
    (0x0180_7F00, true, 1, [11.999, 196.33, 23.5], 400, 0.5),
    (
        0xFFFF_FFFF,
        true,
        1,
        [0.5, 150.25, 0.5],
        2048,
        1.220_703_135e-4,
    ),
];

struct Case {
    texture: &'static str,
    frame: usize,
    /// `CloudInfo`'s 44 bytes as prepare wrote them (none: it built no face and wrote none).
    info: Option<&'static [u8]>,
    /// The bytes buildMesh put in the faces buffer.
    faces: &'static [u8],
}

/// What a temporary hook in the bench mod read of vanilla 26.3 (`.local/clouddump`): for each
/// texture and frame, the faces buildMesh encoded into the buffer prepare gave it, and the
/// CloudInfo bytes prepare wrote with Std140Builder.
const JAVA: &[Case] = &[
    Case {
        texture: "square_edge",
        frame: 0,
        info: None,
        faces: &[],
    },
    Case {
        texture: "square_edge",
        frame: 1,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 0, 0, 129, 128, 128, 59, 0, 0, 0, 0, 0, 0, 4, 193, 128, 194, 213,
            191, 41, 92, 33, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[0, 0, 32, 0, 255, 224],
    },
    Case {
        texture: "square_edge",
        frame: 2,
        info: Some(&[
            255, 254, 254, 62, 129, 128, 0, 63, 0, 0, 128, 63, 255, 254, 126, 63, 0, 217, 255, 190,
            236, 81, 40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64,
            65,
        ]),
        faces: &[
            0, 0, 64, 0, 0, 66, 0, 0, 80, 0, 0, 81, 0, 0, 82, 0, 0, 83, 0, 0, 84, 0, 0, 85, 255,
            255, 128, 255, 255, 131, 255, 255, 133, 255, 1, 128, 255, 1, 130, 255, 1, 133, 0, 254,
            64, 0, 254, 67,
        ],
    },
    Case {
        texture: "square_edge",
        frame: 3,
        info: Some(&[
            129, 128, 0, 63, 255, 254, 254, 62, 0, 0, 0, 0, 129, 128, 128, 59, 178, 97, 101, 188,
            248, 255, 127, 192, 164, 112, 93, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0,
            64, 65,
        ]),
        faces: &[
            0, 0, 129, 0, 0, 128, 0, 0, 132, 0, 0, 144, 0, 0, 145, 0, 0, 146, 0, 0, 147, 0, 0, 148,
            0, 0, 149,
        ],
    },
    Case {
        texture: "tall_inner",
        frame: 0,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 20, 199, 57, 193, 80, 184,
            2, 193, 41, 92, 39, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            0, 0, 65, 0, 0, 66, 0, 0, 80, 0, 0, 81, 0, 0, 82, 0, 0, 83, 0, 0, 84, 0, 0, 85, 0, 0,
            129, 0, 0, 132, 0, 0, 144, 0, 0, 145, 0, 0, 146, 0, 0, 147, 0, 0, 148, 0, 0, 149, 0,
            255, 1, 0, 255, 3,
        ],
    },
    Case {
        texture: "tall_inner",
        frame: 1,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 0, 0, 129, 128, 128, 59, 0, 0, 0, 0, 0, 0, 4, 193, 128, 194, 213,
            191, 41, 92, 33, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[255, 255, 224],
    },
    Case {
        texture: "tall_inner",
        frame: 2,
        info: Some(&[
            255, 254, 254, 62, 129, 128, 0, 63, 0, 0, 128, 63, 255, 254, 126, 63, 0, 217, 255, 190,
            236, 81, 40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64,
            65,
        ]),
        faces: &[
            0, 0, 128, 0, 0, 132, 0, 0, 144, 0, 0, 145, 0, 0, 146, 0, 0, 147, 0, 0, 148, 0, 0, 149,
            254, 0, 128, 254, 0, 133, 255, 255, 64, 255, 255, 67, 255, 255, 69, 255, 0, 64, 255, 0,
            66, 255, 0, 69, 1, 255, 64, 1, 255, 67, 1, 255, 68, 1, 0, 64, 1, 0, 66, 1, 0, 68,
        ],
    },
    Case {
        texture: "tall_inner",
        frame: 3,
        info: Some(&[
            129, 128, 0, 63, 255, 254, 254, 62, 0, 0, 0, 0, 129, 128, 128, 59, 178, 97, 101, 188,
            248, 255, 127, 192, 164, 112, 93, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0,
            64, 65,
        ]),
        faces: &[
            0, 255, 65, 0, 255, 64, 0, 255, 67, 0, 255, 80, 0, 255, 81, 0, 255, 82, 0, 255, 83, 0,
            255, 84, 0, 255, 85, 255, 0, 193, 255, 0, 192, 255, 0, 194, 255, 0, 197, 255, 0, 208,
            255, 0, 209, 255, 0, 210, 255, 0, 211, 255, 0, 212, 255, 0, 213,
        ],
    },
    Case {
        texture: "wide",
        frame: 0,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 17, 199, 57, 193, 80, 184,
            2, 193, 41, 92, 39, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            0, 0, 1, 0, 0, 16, 0, 0, 17, 0, 0, 18, 0, 0, 19, 0, 0, 20, 0, 0, 21, 0, 0, 129, 0, 0,
            144, 0, 0, 145, 0, 0, 146, 0, 0, 147, 0, 0, 148, 0, 0, 149, 255, 0, 193, 255, 0, 194,
            255, 0, 197, 255, 0, 208, 255, 0, 209, 255, 0, 210, 255, 0, 211, 255, 0, 212, 255, 0,
            213,
        ],
    },
    Case {
        texture: "wide",
        frame: 1,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 0, 0, 129, 128, 128, 59, 0, 0, 0, 0, 0, 0, 4, 193, 128, 194, 213,
            191, 41, 92, 33, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[0, 255, 96, 0, 0, 160, 255, 0, 224],
    },
    Case {
        texture: "wide",
        frame: 2,
        info: Some(&[
            255, 254, 254, 62, 129, 128, 0, 63, 0, 0, 128, 63, 255, 254, 126, 63, 0, 217, 255, 190,
            236, 81, 40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64,
            65,
        ]),
        faces: &[
            0, 255, 64, 0, 255, 67, 0, 255, 80, 0, 255, 81, 0, 255, 82, 0, 255, 83, 0, 255, 84, 0,
            255, 85, 255, 0, 0, 255, 0, 5, 255, 255, 192, 255, 255, 195, 255, 255, 197, 255, 255,
            208, 255, 255, 209, 255, 255, 210, 255, 255, 211, 255, 255, 212, 255, 255, 213, 0, 0,
            192, 0, 0, 194, 0, 0, 196, 0, 0, 208, 0, 0, 209, 0, 0, 210, 0, 0, 211, 0, 0, 212, 0, 0,
            213, 254, 0, 128, 0, 1, 64, 0, 1, 66, 1, 0, 128, 1, 0, 132, 1, 255, 0, 1, 255, 3, 1,
            255, 4, 1, 1, 0, 1, 1, 2, 1, 1, 4,
        ],
    },
    Case {
        texture: "wide",
        frame: 3,
        info: Some(&[
            129, 128, 0, 63, 255, 254, 254, 62, 0, 0, 0, 0, 129, 128, 128, 59, 178, 97, 101, 188,
            248, 255, 127, 192, 164, 112, 93, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0,
            64, 65,
        ]),
        faces: &[
            255, 0, 1, 255, 0, 0, 255, 0, 5, 0, 255, 1, 0, 255, 0, 0, 255, 3, 0, 1, 1, 0, 1, 0, 0,
            1, 2, 0, 0, 193, 0, 0, 192, 0, 0, 194, 0, 0, 196, 0, 0, 208, 0, 0, 209, 0, 0, 210, 0,
            0, 211, 0, 0, 212, 0, 0, 213,
        ],
    },
    Case {
        texture: "square_edge",
        frame: 4,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 16, 82, 248, 191, 236, 81,
            40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            255, 0, 192, 255, 0, 194, 255, 0, 197, 255, 0, 208, 255, 0, 209, 255, 0, 210, 255, 0,
            211, 255, 0, 212, 255, 0, 213,
        ],
    },
    Case {
        texture: "tall_inner",
        frame: 4,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 16, 82, 248, 191, 236, 81,
            40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            0, 0, 0, 0, 0, 16, 0, 0, 17, 0, 0, 18, 0, 0, 19, 0, 0, 20, 0, 0, 21, 0, 255, 192, 0,
            255, 195, 0, 255, 196, 0, 255, 208, 0, 255, 209, 0, 255, 210, 0, 255, 211, 0, 255, 212,
            0, 255, 213, 0, 0, 192, 0, 0, 194, 0, 0, 196, 0, 0, 208, 0, 0, 209, 0, 0, 210, 0, 0,
            211, 0, 0, 212, 0, 0, 213,
        ],
    },
    Case {
        texture: "wide",
        frame: 4,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 82, 248, 191, 236, 81,
            40, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            0, 0, 64, 0, 0, 66, 0, 0, 80, 0, 0, 81, 0, 0, 82, 0, 0, 83, 0, 0, 84, 0, 0, 85, 255,
            255, 192, 255, 255, 195, 255, 255, 197, 255, 255, 208, 255, 255, 209, 255, 255, 210,
            255, 255, 211, 255, 255, 212, 255, 255, 213, 1, 0, 0, 1, 0, 4,
        ],
    },
];

/// A frame of a Java dump: the ARGB colour, fancy, the range, the camera, the game time and the
/// partial tick.
type JavaFrame = (u32, bool, i32, [f64; 3], i64, f64);

/// A texture of [`JAVA_TEXTURES`] by name: its width, height and filled cells.
fn java_texture(name: &str) -> (u32, u32, Vec<bool>) {
    let (_, w, h, cells) = JAVA_TEXTURES
        .iter()
        .find(|(texture, ..)| *texture == name)
        .expect("a texture");
    let mut filled = vec![false; (*w * *h) as usize];
    for [x, y] in *cells {
        filled[(y * w + x) as usize] = true;
    }
    (*w, *h, filled)
}

/// The input a frame gives Sunrise at a cloud height: the ARGB colour as `environment` gives it
/// (each channel over 255).
fn java_input<'a>(
    texture: &'a (u32, u32, Vec<bool>),
    frames: &[JavaFrame],
    frame: usize,
    height: f32,
) -> Input<'a> {
    let (argb, fancy, range, camera, time, partial) = frames[frame];
    let channel = |shift: u32| ((argb >> shift) & 0xFF) as f32 / 255.0;
    Input {
        texture,
        uv: [0.0; 2],
        camera: glam::DVec3::from_array(camera),
        time,
        partial_tick: partial,
        range,
        fast: !fancy,
        color: [channel(16), channel(8), channel(0), channel(24)],
        height,
    }
}

/// Sunrise's faces and CloudInfo are vanilla 26.3's, byte for byte, for the same textures and
/// frames: cloud drift, the camera's cell and place in it, the side of the layer, the colour's
/// channels at their ends (0, 1, 127, 128, 254, 255), fast and fancy clouds.
#[test]
fn faces_and_cloud_info_are_vanillas_byte_for_byte() {
    for case in JAVA {
        let texture = java_texture(case.texture);
        let input = java_input(&texture, JAVA_FRAMES, case.frame, CLOUD_HEIGHT);
        let what = format!("{} frame {}", case.texture, case.frame);
        let got: Vec<u8> = faces(&input).iter().map(|&b| b as i8 as u8).collect();
        assert_eq!(got, case.faces, "{what}: faces");
        if let Some(java) = case.info {
            assert_eq!(&info(&input)[..44], java, "{what}: CloudInfo");
        }
    }
}

/// The frames of [`JAVA_HEIGHT`]: the ARGB colour, fancy, the range, the camera, the game time and
/// the partial tick, in a world whose datapack sets the overworld's `minecraft:visual/cloud_height`
/// to 250 (dumped by a temporary bench-mod hook, not kept in the tree).
const JAVA_HEIGHT_FRAMES: &[JavaFrame] = &[
    (0xFFFF_FFFF, true, 1, [37.25, 255.5, -5.5], 12345, 0.37),
    (0x00FF_0001, false, 1, [-3.75, 251.0, 30.125], 0, 0.0),
    (0xFE7F_80FF, true, 2, [0.5, 150.25, 0.5], 999_999, 0.99),
];

/// What the bench mod's hook read of vanilla 26.3 in that world: `LevelExtractor`'s
/// `levelRenderState.cloudHeight` (`EnvironmentAttributes.CLOUD_HEIGHT` at the camera) was 250.0
/// (bits 1132068864), and `prepare` given it built these faces and `CloudInfo` for the wide
/// texture.
const JAVA_HEIGHT: &[Case] = &[
    Case {
        texture: "wide",
        frame: 0,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 0, 0, 128, 63, 17, 199, 57, 193, 0, 0,
            176, 192, 41, 92, 39, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[
            0, 0, 1, 0, 0, 16, 0, 0, 17, 0, 0, 18, 0, 0, 19, 0, 0, 20, 0, 0, 21, 0, 0, 129, 0, 0,
            144, 0, 0, 145, 0, 0, 146, 0, 0, 147, 0, 0, 148, 0, 0, 149, 255, 0, 193, 255, 0, 194,
            255, 0, 197, 255, 0, 208, 255, 0, 209, 255, 0, 210, 255, 0, 211, 255, 0, 212, 255, 0,
            213,
        ],
    },
    Case {
        texture: "wide",
        frame: 1,
        info: Some(&[
            0, 0, 128, 63, 0, 0, 0, 0, 129, 128, 128, 59, 0, 0, 0, 0, 0, 0, 4, 193, 0, 0, 128, 191,
            41, 92, 33, 193, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64, 65,
        ]),
        faces: &[0, 255, 96, 0, 0, 160, 255, 0, 224],
    },
    Case {
        texture: "wide",
        frame: 2,
        info: Some(&[
            255, 254, 254, 62, 129, 128, 0, 63, 0, 0, 128, 63, 255, 254, 126, 63, 0, 217, 255, 190,
            0, 128, 199, 66, 82, 184, 142, 192, 0, 0, 0, 0, 0, 0, 64, 65, 0, 0, 128, 64, 0, 0, 64,
            65,
        ]),
        faces: &[
            0, 255, 64, 0, 255, 67, 0, 255, 80, 0, 255, 81, 0, 255, 82, 0, 255, 83, 0, 255, 84, 0,
            255, 85, 255, 0, 0, 255, 0, 5, 255, 255, 192, 255, 255, 195, 255, 255, 197, 255, 255,
            208, 255, 255, 209, 255, 255, 210, 255, 255, 211, 255, 255, 212, 255, 255, 213, 0, 0,
            192, 0, 0, 194, 0, 0, 196, 0, 0, 208, 0, 0, 209, 0, 0, 210, 0, 0, 211, 0, 0, 212, 0, 0,
            213, 254, 0, 128, 0, 1, 64, 0, 1, 66, 1, 0, 128, 1, 0, 132, 1, 255, 0, 1, 255, 3, 1,
            255, 4, 1, 1, 0, 1, 1, 2, 1, 1, 4,
        ],
    },
];

/// A datapack's `visual/cloud_height` of 250 moves the layer as in vanilla: Sunrise's faces and
/// `CloudInfo` for that height are vanilla's, byte for byte, above, inside and below the layer.
#[test]
fn faces_and_cloud_info_follow_a_datapacks_cloud_height() {
    for case in JAVA_HEIGHT {
        let texture = java_texture(case.texture);
        let input = java_input(&texture, JAVA_HEIGHT_FRAMES, case.frame, 250.0);
        let what = format!("frame {}", case.frame);
        let got: Vec<u8> = faces(&input).iter().map(|&b| b as i8 as u8).collect();
        assert_eq!(got, case.faces, "{what}: faces");
        assert_eq!(&info(&input)[..44], case.info.unwrap(), "{what}: CloudInfo");
    }
}
