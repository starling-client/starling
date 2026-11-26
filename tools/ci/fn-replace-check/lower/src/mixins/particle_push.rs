fn particle_push() -> ::core::ops::ControlFlow<()> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| {
        fnr_lower::note("HEAD particles::ParticleEngine::push (sunrise_client_render)")
    });
    ::core::ops::ControlFlow::Continue(())
}
