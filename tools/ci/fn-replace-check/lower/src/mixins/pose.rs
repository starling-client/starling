fn pose(value: (), m: &Mob, p: &mut Poses) {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("RETURN mobs::anim::setup"));
    // The bend the `bend` row draws, on every model with a `right_arm`; `bend` draws it on the
    // player layer only.
    p.set_data("right_arm", fnr_lower::Bend { angle: 0.8 });
    value
}
