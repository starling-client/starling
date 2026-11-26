//! Starling's tests of pack_uniforms.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
fn a_kept_shadow_map_stays_where_it_was_drawn() {
    let view = Mat4::from_translation(Vec3::new(1.0, 2.0, -128.0));
    let (drawn, camera) = (
        DVec3::new(3500.25, 70.0, -12.5),
        DVec3::new(3501.0, 70.5, -11.0),
    );
    let world = DVec3::new(3510.0, 64.0, -20.0);
    let then = view.transform_point3((world - drawn).as_vec3());
    let now = kept_shadow_view(view, drawn, camera).transform_point3((world - camera).as_vec3());
    assert!((then - now).length() < 1e-3, "{then} {now}");
}
