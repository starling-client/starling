//! Every particle the client is asked to add (`ClientLevel.doAddParticle`).

pub trait Hooks {
    /// A particle is about to enter this client's world (`ClientLevel.doAddParticle`): every
    /// one, whether the server sent it, a block's or entity's ambient tick, an explosion or
    /// another mod's `ClientHost::add_particle` asked for it. `options` is the particle's
    /// type and data, read-only: a mod that wants a different particle cancels this one and
    /// adds its own. Move it by writing `pos`, or change `velocity`. Return
    /// [`Flow::Cancel`](mod_api::Flow::Cancel) to drop it before it is created.
    /// `override_limiter` and `always_show` are vanilla's flags for this request; they are
    /// not writable. The hook runs before the 32-block camera distance check and the
    /// Particles setting, so a moved particle is checked at the position the listener left;
    /// the setting's random roll is made once, as in vanilla. A position or velocity left
    /// non-finite drops the particle. A panicking listener is contained and the particle
    /// goes on as it was. The listener has no host: it runs inside the particle engine, so
    /// a mod keeps what it needs (a wind, a limit) in its own state, updated from
    /// `client_tick`. With no enabled listener the cost is one check per particle.
    /// Client-side. Diverges from upstream: new hook.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "cancel",
        cancellable = true
    )]
    fn client_particle_add(
        options: &ParticleOptions,
        pos: &mut [f64; 3],
        velocity: &mut [f64; 3],
        override_limiter: bool,
        always_show: bool,
        cancel: &mut bool,
    );
}
