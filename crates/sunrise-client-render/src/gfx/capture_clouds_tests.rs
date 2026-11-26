//! A captured frame's clouds keep the layer's height (`clouds::Input::height`); a capture written
//! before the height was kept reads as `EnvironmentAttributes.CLOUD_HEIGHT`'s default, 192.33.
use super::{Clouds, FrameInputs};

const FIELDS: &str = r#""texture": [1, 1, [true]], "uv": [0.0, 0.0], "camera": [0.5, 64.0, 0.5],
    "time": 0, "partial_tick": 0.0, "range": 2, "fast": false, "color": [1.0, 1.0, 1.0, 0.8]"#;

#[test]
fn a_capture_keeps_its_cloud_height() {
    let clouds: Clouds = serde_json::from_str(&format!("{{{FIELDS}, \"height\": 250.0}}")).unwrap();
    assert_eq!(clouds.height.to_bits(), 1_132_068_864);
}

#[test]
fn a_capture_without_a_cloud_height_has_the_default() {
    let clouds: Clouds = serde_json::from_str(&format!("{{{FIELDS}}}")).unwrap();
    // 192.33 as a float: 0x4340547B.
    assert_eq!(clouds.height.to_bits(), 0x4340_547B);
}

/// A replay draws the clouds at the height the frame had: recorded, written, read back and
/// replayed, 250 stays 250.
#[test]
fn a_replay_draws_the_recorded_cloud_height() {
    let texture = (1, 1, vec![true]);
    let input = crate::clouds::Input {
        texture: &texture,
        uv: [0.0; 2],
        camera: glam::DVec3::new(0.5, 64.0, 0.5),
        time: 0,
        partial_tick: 0.0,
        range: 2,
        fast: false,
        color: [1.0; 4],
        height: 250.0,
    };
    let json = serde_json::to_string(&Clouds::record(&input)).unwrap();
    let clouds: Clouds = serde_json::from_str(&json).unwrap();
    assert_eq!(clouds.input().height.to_bits(), 1_132_068_864);
}

#[test]
fn a_capture_round_trips_distinct_draw_and_cull_fovs() {
    let hud = crate::hud::HudBatch::scratch([4, 4]);
    let mut frame = crate::gfx::frame_tests::frame(&hud);
    frame.fov_degrees = 10.0;
    frame.cull_fov_degrees = 70.0;
    let json = serde_json::to_string(&FrameInputs::record(&frame)).unwrap();
    let loaded: FrameInputs = serde_json::from_str(&json).unwrap();
    let replayed = loaded.to_frame(&[], &hud);
    assert_eq!(replayed.fov_degrees.to_bits(), 10.0f32.to_bits());
    assert_eq!(replayed.cull_fov_degrees.to_bits(), 70.0f32.to_bits());

    // Captures written before `cull_fov_degrees` existed fall back to `fov_degrees`.
    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy.as_object_mut().unwrap().remove("cull_fov_degrees");
    let legacy_loaded: FrameInputs = serde_json::from_value(legacy).unwrap();
    let legacy_replayed = legacy_loaded.to_frame(&[], &hud);
    assert_eq!(
        legacy_replayed.cull_fov_degrees.to_bits(),
        10.0f32.to_bits()
    );
}
