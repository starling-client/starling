#[test]
fn hand_pose_rejects_invalid_components() {
    let identity = super::hand_pose_matrix([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(identity, glam::Mat4::IDENTITY);
    assert_eq!(
        super::hand_pose_matrix([
            f32::NAN,
            17.0,
            f32::INFINITY,
            3601.0,
            -3601.0,
            f32::NAN,
            0.0
        ]),
        identity
    );
    let point = super::hand_pose_matrix([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 2.0])
        .transform_point3(glam::Vec3::X);
    assert_eq!(point, glam::Vec3::new(3.0, 2.0, 3.0));
}

/// A mod's shader values that aren't finite read as 0.
#[test]
fn shader_params_that_arent_finite_read_as_zero() {
    let mut params = [[1.0; 4]; 8];
    params[0] = [f32::NAN, 2.0, f32::INFINITY, -3.0];
    let out = super::finite_params(params);
    assert_eq!(out[0], [0.0, 2.0, 0.0, -3.0]);
    assert_eq!(out[7], [1.0; 4]);
}

/// `native_play_sound` takes vanilla's and mods' sound ids, nothing else.
#[test]
fn sound_ids_are_identifiers() {
    for id in [
        "minecraft:block.note_block.harp",
        "mymod:machine/hum",
        "my_mod.x-1:a",
    ] {
        assert!(super::sound_id(id), "{id}");
    }
    for id in [
        "",
        "hum",
        ":hum",
        "mymod:",
        "MyMod:hum",
        "mymod:Hum",
        "my/mod:hum",
        "a:b c",
    ] {
        assert!(!super::sound_id(id), "{id}");
    }
}
