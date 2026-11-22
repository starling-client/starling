#![cfg(test)]

/// A mod's camera replaces vanilla's value by value; one that isn't finite keeps vanilla's.
#[test]
fn a_mod_camera_replaces_each_finite_value() {
    use super::posed_camera;
    let vanilla = (glam::DVec3::new(1.0, 2.0, 3.0), 10.0, 20.0);
    assert_eq!(
        posed_camera(vanilla, [-40.5, 90.0, 1e7], 270.0, -35.0),
        (glam::DVec3::new(-40.5, 90.0, 1e7), 270.0, -35.0)
    );
    assert_eq!(
        posed_camera(
            vanilla,
            [f64::NAN, 5.0, f64::INFINITY],
            f32::NAN,
            f32::NEG_INFINITY
        ),
        (glam::DVec3::new(1.0, 5.0, 3.0), 10.0, 20.0)
    );
}
