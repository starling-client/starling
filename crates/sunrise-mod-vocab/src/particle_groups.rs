//! The render groups a client mod's particles draw in, as it registers them in
//! `register_particle_groups` (NeoForge's `RegisterParticleGroupsEvent` and vanilla's
//! `ParticleRenderType`).

use crate::{Identifier, RegisterParticleGroupsEvent};

/// The definition built by a factory registered through [`RegisterParticleGroupsEvent::register`].
/// The engine keeps each group's live particles and 16384-particle limit separate, and draws
/// the groups after the built-in particles, in registration order.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal
// no wire: the factory builds this definition in-process for the particle engine.
pub struct ParticleGroup {
    /// How the group's particles blend, whatever their own sprites hold.
    pub blend: ParticleBlend,
    /// The namespaced particle types that draw in this group, such as `mymod:spark` for
    /// `assets/mymod/particles/spark.json`. Each needs a sprite set, with or without a
    /// `register_particle_providers` provider. Types already claimed by an earlier group or
    /// without a matching sprite set are logged and left out.
    pub particles: Vec<String>,
}

impl RegisterParticleGroupsEvent<'_> {
    /// Register a render group under an [`Identifier`] with a nonempty namespace and path (any
    /// namespace, including `minecraft`, is allowed). Duplicate IDs fail client startup without
    /// invoking any factory, matching NeoForge's `RegisterParticleGroupsEvent`. A phase-refused
    /// listener's registrations are discarded. An accepted factory runs once per particle engine
    /// construction, not on sprite reload. Capture owned data and return a fresh definition each
    /// time.
    pub fn register(
        &mut self,
        id: Identifier,
        factory: impl Fn() -> ParticleGroup + Send + Sync + 'static,
    ) {
        self.groups.push(ParticleGroupRegistration {
            id,
            factory: Box::new(factory),
        });
    }
}

/// A staged group factory; dispatch attributes it to the registering mod.
// no wire: registration retains executable native code.
#[non_exhaustive]
pub struct ParticleGroupRegistration {
    /// The render group ID.
    pub id: Identifier,
    /// The factory called when a particle engine is constructed.
    pub factory: Box<dyn Fn() -> ParticleGroup + Send + Sync>,
}

/// How a [`ParticleGroup`]'s particles blend: the layer they draw on, which decides their order
/// against the translucent terrain (`SingleQuadParticle.Layer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's two quad layers, matched without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ParticleBlend {
    /// `SingleQuadParticle.Layer.OPAQUE`: drawn with the terrain, writing depth; the sprite's
    /// transparent texels are cut out.
    Opaque,
    /// `SingleQuadParticle.Layer.TRANSLUCENT`: drawn after the translucent terrain, blended by
    /// each particle's alpha.
    Translucent,
}
