use super::*;
use sunrise_assets::{
    bake::bake_flat_face,
    model::{Direction, Material},
};

fn face() -> BakedFace {
    BakedFace::new(
        bake_flat_face(
            [0.0; 3],
            [16.0; 3],
            [0.0, 0.0, 16.0, 16.0],
            Direction::South,
            Material {
                sprite: "test:source".into(),
                force_translucent: false,
            },
            -1,
        )
        .unwrap(),
        7,
        true,
        false,
        None,
    )
}
fn sprite(name: &str) -> Option<u32> {
    match name {
        "test:source" => Some(7),
        "test:target" => Some(11),
        _ => None,
    }
}
fn draw(
    faces: &[BakedFace],
    pose: Mat4,
    options: NativeModelRenderOptions<'_>,
) -> (bool, Vec<Vertex>) {
    let mut output = Vec::new();
    let accepted = draw_model(faces, pose, 0xf0f0, options, sprite, |v| output.push(v));
    (accepted, output)
}
#[test]
fn late_unknown_material_or_sprite_rejects_the_whole_model() {
    let first = face();
    let mut second = face();
    second.quad.render_type = Some(RenderType::Unsupported);
    let mut faces = [first, second];
    let (accepted, output) = draw(&faces, Mat4::IDENTITY, Default::default());
    assert!(!accepted);
    assert!(output.is_empty());
    let (accepted, output) = draw(
        &faces,
        Mat4::IDENTITY,
        NativeModelRenderOptions {
            material: NativeModelMaterial::Cutout,
            ..Default::default()
        },
    );
    assert!(accepted);
    assert_eq!(output.len(), 8);
    faces[1].quad.material.sprite = "test:missing".into();
    let (accepted, output) = draw(&faces, Mat4::IDENTITY, Default::default());
    assert!(!accepted);
    assert!(output.is_empty());
}
#[test]
fn exact_sprite_shift_and_explicit_json_shrink_precede_array_mapping() {
    let options = NativeModelRenderOptions {
        uv_shrink_ratio: 4.0 / 2048.0,
        uv_shift: Some(NativeModelUvShift {
            source_sprite: "test:source",
            target_sprite: "test:target",
            scale: [0.5; 2],
            offset: [0.25, 0.125],
        }),
        ..Default::default()
    };
    let (accepted, output) = draw(&[face()], Mat4::IDENTITY, options);
    assert!(accepted);
    assert_eq!(output[0].uv, [0.25048828, 0.12548828]);
    assert_eq!(output[2].uv, [0.7495117, 0.6245117]);
    assert!(output.iter().all(|v| v.layer >> 16 == 11));
    let (accepted, output) = draw(
        &[face()],
        Mat4::IDENTITY,
        NativeModelRenderOptions {
            uv_shift: Some(NativeModelUvShift {
                source_sprite: "test:target",
                target_sprite: "test:source",
                ..options.uv_shift.unwrap()
            }),
            ..options
        },
    );
    assert!(!accepted);
    assert!(output.is_empty());
}
#[test]
fn transformed_slope_normal_stays_float_and_fullbright_keeps_shading() {
    let mut face = face();
    face.quad.positions = [
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 0.0],
    ];
    let pose = Mat4::from_rotation_y(0.372) * Mat4::from_scale(Vec3::new(-2.0, 3.0, 1.0));
    let (accepted, output) = draw(&[face], pose, Default::default());
    assert!(accepted);
    let normal = Vec3::new(
        output[0].glint_uv[0],
        output[0].glint_uv[1],
        f32::from_ne_bytes(output[0].normal.map(|v| v as u8)),
    );
    assert!((normal.length() - 1.0).abs() < 1e-6);
    assert!((normal.y - 0.31622776).abs() < 1e-6);
    assert!((normal.x + 0.3448316).abs() < 1e-5, "{normal:?}");
    assert_eq!(output[0].light, 0xf0f0);
    assert_eq!(output[0].color, u32::MAX);
    // Float payload, not a second signed-byte normal quantization.
    assert_ne!(normal.y, (normal.y * 127.0) as i8 as f32 / 127.0);
}
#[test]
fn invalid_shift_and_singular_pose_emit_nothing() {
    let options = NativeModelRenderOptions {
        uv_shrink_ratio: f32::NAN,
        ..Default::default()
    };
    let (ok, output) = draw(&[face()], Mat4::IDENTITY, options);
    assert!(!ok);
    assert!(output.is_empty());
    let (ok, output) = draw(
        &[face()],
        Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0)),
        Default::default(),
    );
    assert!(!ok);
    assert!(output.is_empty());
}

#[test]
fn a_deduplicated_array_layer_is_not_the_same_named_source_sprite() {
    let mut alias = face();
    alias.quad.material.sprite = "test:alias".into();
    let mut output = Vec::new();
    assert!(draw_model(
        &[face(), alias],
        Mat4::IDENTITY,
        0xf0f0,
        NativeModelRenderOptions {
            uv_shift: Some(NativeModelUvShift {
                source_sprite: "test:source",
                target_sprite: "test:target",
                scale: [0.5; 2],
                offset: [0.25; 2]
            }),
            ..Default::default()
        },
        |name| if name == "test:alias" {
            Some(7)
        } else {
            sprite(name)
        },
        |v| output.push(v)
    ));
    assert_eq!(output[0].layer >> 16, 11);
    assert_eq!(output[4].layer >> 16, 7);
    assert_eq!(output[4].uv, [0.0, 0.0]);
    assert_eq!(output[6].uv, [1.0, 1.0]);
}
#[test]
fn the_draw_colour_multiplies_every_channel_and_white_changes_nothing() {
    let tinted = || {
        let mut tinted = face();
        tinted.quad.tint_index = 0;
        tinted
    };
    // The tint (0x80 red) first, then the draw colour 0x80/0xFF/0x40 and alpha 0xC0, as ABGR.
    let options = NativeModelRenderOptions {
        tint: 0x80_ffff,
        color: 0xC0_80_FF_40,
        ..Default::default()
    };
    let (accepted, output) = draw(&[tinted(), face()], Mat4::IDENTITY, options);
    assert!(accepted);
    // Red: 255 * 0x80 / 255 = 0x80, then * 0x80 / 255 = 0x40. Green and blue: 255, then * 0xFF
    // and * 0x40 / 255. The alpha makes the quads translucent, and a translucent quad carries its
    // face's shade in the colour: the south face's 0.8, as 204 / 255 on each colour channel.
    assert_eq!(output[0].color, 0xC0 << 24 | 0x33 << 16 | 0xCC << 8 | 0x33);
    // The untinted quad takes the draw colour alone, then the same shade.
    assert_eq!(output[4].color, 0xC0 << 24 | 0x33 << 16 | 0xCC << 8 | 0x66);
    assert!(
        output
            .iter()
            .all(|v| v.layer & 0xffff == gfx::TERRAIN_MODEL_TRANSLUCENT_OVERLAY)
    );
    // The same tint under the default white draw colour: the tint alone, opaque and unshaded.
    let white = NativeModelRenderOptions {
        tint: 0x80_ffff,
        ..Default::default()
    };
    let (_, plain) = draw(&[tinted()], Mat4::IDENTITY, white);
    assert_eq!(plain[0].layer & 0xffff, gfx::TERRAIN_MODEL_SOLID_OVERLAY);
    assert_eq!(plain[0].color, 0xFFFF_FF80);
}
#[test]
fn hidden_faces_skip_quads_by_cull_side_and_keep_those_without() {
    let south = || {
        let mut south = face();
        south.quad.cull = Some(Direction::South);
        south
    };
    let open = || {
        let mut open = face();
        open.quad.cull = None;
        open
    };
    let hide = |hidden_faces| {
        let options = NativeModelRenderOptions {
            hidden_faces,
            ..Default::default()
        };
        draw(&[south(), open()], Mat4::IDENTITY, options).1.len() / 4
    };
    assert_eq!(hide(0), 2);
    assert_eq!(hide(1 << 3), 1, "south is bit 3");
    assert_eq!(hide(!(1 << 3)), 2);
    assert_eq!(hide(0b11_1111), 1);
}
#[test]
fn translucent_quads_bake_their_shade_into_the_colour() {
    let mut glass = face();
    glass.quad.render_type = Some(RenderType::Translucent);
    glass.layer = 7;
    let (accepted, output) = draw(&[glass], Mat4::IDENTITY, Default::default());
    assert!(accepted);
    // South faces shade 204/255 (0.8), baked into the colour; the sprite layer rides the high
    // half of `layer`.
    assert_eq!(output[0].color & 0xff, 204);
    assert_eq!(output[0].layer >> 16, 7);
    assert_eq!(output[0].glint_uv, [0.0; 2]);
    assert_eq!(output[0].normal, [0; 4]);
}
