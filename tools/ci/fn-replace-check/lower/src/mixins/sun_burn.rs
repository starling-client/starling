fn sun_burn(m: &Mob) -> ::core::ops::ControlFlow<bool> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::ai::sun_burn_tick"));
    ::core::ops::ControlFlow::Continue(())
}
