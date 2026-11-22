fn body_yaw(value: f32, m: &Mob) -> f32 {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("MODIFY_EXPRESSION_VALUE mobs::render::base_transform_at"));
    value
}
