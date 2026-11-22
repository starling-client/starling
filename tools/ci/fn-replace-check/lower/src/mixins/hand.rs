fn hand(value: Option<glam::Mat4>, m: &Mob, left: bool) -> Option<glam::Mat4> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (m, left);
    SEEN.call_once(|| fnr_lower::note("RETURN mobs::render::translate_to_hand"));
    let elbow = glam::Vec3::new(0.0, 0.25, 0.0);
    value.map(|hand| {
        hand * glam::Mat4::from_translation(elbow)
            * glam::Mat4::from_rotation_x(0.5)
            * glam::Mat4::from_translation(-elbow)
    })
}
