fn crossbow_shot(m: &Mob) -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::illager::shoot_crossbow"));
    ::core::ops::ControlFlow::Continue(())
}
