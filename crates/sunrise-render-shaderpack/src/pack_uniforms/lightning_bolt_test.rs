//! `lightningBoltPosition` (spec 5.2): the last bolt in Sunrise's draw list, in player space,
//! w = 1.

use glam::DVec3;

use super::initial_history_tests::test_frame;
use super::{FrameUniforms, PackInputs, Value, index, zero};

fn bolt_position(u: &FrameUniforms) -> [f32; 4] {
    match u.values[index("lightningBoltPosition").unwrap()] {
        Value::F(v) => [v[0], v[1], v[2], v[3]],
        Value::I(_) => panic!("lightningBoltPosition is a float vector"),
    }
}

/// No bolt reads zeros, w included; a bolt reads its position minus the camera's with w = 1,
/// worked out in doubles: at x = 29 999 000.25 a float can't hold the quarter, so subtracting
/// floats would give 10 or 11, not 10.25. Of two bolts, the last in the draw list counts.
#[test]
fn the_last_bolt_in_the_draw_list_reaches_the_pack_in_player_space() {
    let mut f = test_frame();
    f.camera = DVec3::new(29_999_000.25, 64.5, -30.75);
    let mut u = FrameUniforms::default();
    u.update(&f, &PackInputs::default(), [800, 450]);
    assert_eq!(bolt_position(&u), [0.0; 4]);

    f.lightning_bolt = Some(DVec3::new(29_999_010.5, 63.0, -20.25));
    u.update(&f, &PackInputs::default(), [800, 450]);
    assert_eq!(bolt_position(&u), [10.25, -1.5, 10.5, 1.0]);

    f.lightning_bolt = Some(DVec3::new(29_998_990.0, 70.25, -40.5));
    u.update(&f, &PackInputs::default(), [800, 450]);
    assert_eq!(bolt_position(&u), [-10.25, 5.75, -9.75, 1.0]);

    // Gone again: zeros.
    f.lightning_bolt = None;
    u.update(&f, &PackInputs::default(), [800, 450]);
    assert_eq!(bolt_position(&u), [0.0; 4]);
}

/// It is a `vec4`: a program declaring it so reads it. A `vec3` or `bool` declaration keeps its
/// initial value (zero unless the shader specifies an initializer), not the first component.
/// Custom uniforms see four components before the first frame.
#[test]
fn the_bolt_position_is_a_vec4() {
    let v = Value::f(&[1.0, 2.0, 3.0, 1.0]);
    assert!(v.read_as("lightningBoltPosition", "vec4"));
    assert!(!v.read_as("lightningBoltPosition", "vec3"));
    assert!(!v.read_as("lightningBoltPosition", "bool"));
    assert_eq!(zero("lightningBoltPosition").unwrap().n, 4);
}
