fn skeleton_shot(m: &Mob, power: f32) -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = (m, power);
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::undead::perform_ranged_attack"));
    ::core::ops::ControlFlow::Continue(())
}
