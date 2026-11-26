//! Mod hooks the render crate reaches first, answered by the game above it.
//!
//! The render crate never names the mod host. For each such hook the engine that reaches it
//! holds a [`ModHooks`] and calls the hook's slot, a plain function the game installs while at
//! least one enabled mod listens to that hook and clears otherwise, so with no listener a call
//! site costs one `Option` check. The game keeps every engine's copy current once per client
//! tick, which follows mods loading, being switched off and unloading. The installed function
//! dispatches to the mods and contains a panicking listener. A later render-crate hook adds its
//! slot here and its line where the game fills them.
use crate::particles::ParticleOptions;

/// `client_particle_add` (`ClientLevel.doAddParticle`): the particle `options` asks for at
/// `pos` with `velocity`, with vanilla's `override_limiter` and `always_show` flags. The mods
/// may move it or change its velocity in place; false drops it.
pub type ParticleAdd = fn(&ParticleOptions, &mut [f64; 3], &mut [f64; 3], bool, bool) -> bool;

/// The installed slots, one per hook; `None` while no enabled mod listens.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModHooks {
    /// [`ParticleAdd`], called by `ParticleEngine` for every particle it is asked to add.
    pub particle_add: Option<ParticleAdd>,
}
