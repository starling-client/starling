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

/// A shake is scaled by Distortion Effects, bounded on each axis, and a
/// value that isn't finite is none.
#[test]
fn a_shake_is_scaled_and_bounded() {
    use super::{Shake, bounded_shake, shake_pose};
    let shake = Shake {
        pitch: 4.0,
        yaw: -30.0,
        roll: f32::NAN,
        offset: [0.2, 2.0, f32::INFINITY],
    };
    assert_eq!(
        bounded_shake(shake, 1.0),
        Shake {
            pitch: 4.0,
            yaw: -10.0,
            roll: 0.0,
            offset: [0.2, 0.5, 0.0],
        }
    );
    assert_eq!(bounded_shake(shake, 0.5).pitch, 2.0);
    assert_eq!(bounded_shake(shake, 0.0), Shake::default());
    assert_eq!(shake_pose(Shake::default()), glam::Mat4::IDENTITY);
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

/// A client mod's sound needs a namespaced event, finite values and a
/// start a minute away at most.
#[test]
fn a_local_sound_is_checked() {
    use super::local_sound_ok;
    use mod_api::{SoundPlay, SoundSource};
    let at = |id| SoundPlay::at(id, SoundSource::Hostile, [0.5, 64.0, 0.5]);
    assert!(local_sound_ok(
        &at("ironfall:boom_far").with_volume(8.0),
        140
    ));
    assert!(local_sound_ok(&at("minecraft:entity.generic.explode"), 0));
    assert!(local_sound_ok(
        &SoundPlay::ui("minecraft:ui.button.click"),
        1200
    ));
    assert!(!local_sound_ok(&at("boom"), 0));
    assert!(!local_sound_ok(&at("ironfall:"), 0));
    assert!(!local_sound_ok(&at("Ironfall:Boom"), 0));
    let nowhere = SoundPlay::at("ironfall:boom", SoundSource::Hostile, [f64::NAN, 0.0, 0.0]);
    assert!(!local_sound_ok(&nowhere, 0));
    assert!(!local_sound_ok(&at("ironfall:boom").with_volume(-1.0), 0));
    assert!(!local_sound_ok(
        &at("ironfall:boom").with_pitch(f32::NAN),
        0
    ));
    assert!(!local_sound_ok(&at("ironfall:boom").looping(-1), 0));
    assert!(!local_sound_ok(&at("ironfall:boom"), 1201));
}

/// `play_local_sound` runs `SoundEffects::valid`, the check `play_sound` edits run: the ends of
/// every range play, and a value past one, NaN or an infinity plays nothing.
#[test]
fn a_local_sounds_effects_are_checked() {
    use super::local_sound_ok;
    use mod_api::{SoundEffects, SoundPlay, SoundSource};
    let with = |set: fn(&mut SoundEffects)| {
        let mut effects = SoundEffects::NONE;
        set(&mut effects);
        SoundPlay::at("ironfall:hum", SoundSource::Blocks, [0.5, 64.0, 0.5]).with_effects(effects)
    };
    let played: [fn(&mut SoundEffects); 6] = [
        |_| {},
        |e| e.gain = 0.0,
        |e| e.lowpass = 20.0,
        |e| e.highpass = 20_000.0,
        |e| {
            e.room = 1.0;
            e.hall = 1.0;
            e.cavern = 1.0;
        },
        |e| e.reverb_lowpass = 20.0,
    ];
    for set in played {
        assert!(local_sound_ok(&with(set), 0));
    }
    let refused: [fn(&mut SoundEffects); 9] = [
        |e| e.gain = f32::NAN,
        |e| e.gain = 1.5,
        |e| e.direct = -0.1,
        |e| e.lowpass = 19.0,
        |e| e.lowpass = f32::INFINITY,
        |e| e.highpass = f32::NEG_INFINITY,
        |e| e.room = 2.0,
        |e| e.cavern = f32::NAN,
        |e| e.reverb_lowpass = 20_001.0,
    ];
    for set in refused {
        assert!(!local_sound_ok(&with(set), 0));
    }
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
