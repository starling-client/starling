use super::*;

impl Game<'_> {
    /// `ClientLevel.doAddParticle`'s head: the client's mods may move, re-aim or drop the
    /// particle `options` asks for at `pos` with `velocity` before the camera distance check
    /// and the Particles setting decide whether it is created. The particle engine calls it
    /// through its `particle_add` slot, which [`Game::sync_render_hooks`] fills only while
    /// an enabled mod listens.
    pub(in crate::game) fn native_client_particle_add(
        options: &ParticleOptions,
        pos: &mut [f64; 3],
        velocity: &mut [f64; 3],
        override_limiter: bool,
        always_show: bool,
        cancel: &mut bool,
    ) {
        Self::fire_client_particle_add(
            options,
            pos,
            velocity,
            override_limiter,
            always_show,
            cancel,
        );
    }
}
