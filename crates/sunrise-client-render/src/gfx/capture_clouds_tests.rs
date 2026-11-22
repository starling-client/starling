//! A captured frame's clouds keep the layer's height (`clouds::Input::height`); a capture written
//! before the height was kept reads as `EnvironmentAttributes.CLOUD_HEIGHT`'s default, 192.33.
use super::Clouds;

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
