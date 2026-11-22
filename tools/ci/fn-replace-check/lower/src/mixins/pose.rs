fn pose(value: (), m: &Mob, p: &mut Poses) {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (m, p);
    SEEN.call_once(|| fnr_lower::note("RETURN mobs::anim::setup"));
    value
}
