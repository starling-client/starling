fn witch_throw(m: &Mob) -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::witch::witch_attack"));
    ::core::ops::ControlFlow::Continue(())
}
