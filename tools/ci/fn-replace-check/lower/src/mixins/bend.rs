fn bend(
    layer: Option<&str>,
    part: &Part,
    matrix: glam::Mat4,
    normals: ModelNormals,
    paint: Paint,
    poses: &Poses,
    quads: &mut Quads<'_>,
) -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::render::compile_part"));
    let angle = poses
        .data::<fnr_lower::Bend>(&part.name)
        .map(|bend| bend.angle);
    match angle {
        Some(angle) if layer == Some("minecraft:player#main") && part.name == "right_arm" => {
            // The arm's polygons squashed to its upper half, then again turned about X at the
            // elbow, half way down the arm.
            let half = glam::Mat4::from_scale(glam::Vec3::new(1.0, 0.5, 1.0));
            let elbow = glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.375, 0.0));
            push_polygons(&part.polygons, matrix * half, normals, paint, quads);
            let lower = matrix * elbow * glam::Mat4::from_rotation_x(angle) * half;
            push_polygons(&part.polygons, lower, normals, paint, quads);
            ::core::ops::ControlFlow::Break(())
        }
        _ => ::core::ops::ControlFlow::Continue(()),
    }
}
