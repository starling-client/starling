//! Code-defined particle providers, as a client mod registers them in `register_particle_providers`
//! (NeoForge's `RegisterParticleProvidersEvent` and `ParticleProvider`).
//!
//! A sprite-set provider for a common `ParticleType`. The loaded sprite set and optional
//! `sunrise:behaviour` seed the particle's state; the provider creates it with typed options
//! and moves it in batches. The game resolves the provider once per sprite reload.
use super::particle_types::{ParticleCallback, TypedParticleOptions};
use crate::{BlockState, LightLayer, ParticleBlend, ParticleKey, ParticleType, RandomSource};
use std::{collections::HashMap, sync::Arc};

/// One particle of a provider's type, as its tick reads and writes it: the fields of vanilla's
/// `Particle` a `tick` moves. Plain numbers, so a tick allocates nothing.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it for each tick and a mod only edits its fields
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ModParticle {
    /// `Particle.x/y/z`: the position now. A tick moves the particle by writing it.
    pub position: [f64; 3],
    /// `Particle.xo/yo/zo`: where it was a tick ago, which a frame interpolates from. The game
    /// sets it to `position` before each tick; a tick that teleports the particle sets it too.
    pub previous: [f64; 3],
    /// `Particle.xd/yd/zd`: the velocity the spawn gave it (the declared one, or the spawn
    /// call's). The game applies no vanilla motion to a provider's particle, so a tick moves
    /// `position` by it, or not.
    pub velocity: [f64; 3],
    /// `Particle.age`, the ticks this particle has run: 1 on its first tick. The game advances it
    /// before each tick.
    pub age: i32,
    /// `Particle.lifetime`: the particle expires on the tick after `age` reaches it. A tick may
    /// change it.
    pub lifetime: i32,
    /// `SingleQuadParticle.quadSize`: the billboard's half-width in blocks.
    pub size: f32,
    /// `Particle.rCol/gCol/bCol/alpha`, each 0 to 1.
    pub rgba: [f32; 4],
    /// The sprite drawn, an index in the sprite set `particles/<path>.json` lists (the last one
    /// when it is past the list).
    pub frame: u32,
    /// `Particle.remove()`: set it to expire the particle before its lifetime is up.
    pub removed: bool,
    /// Four numbers that stay with this particle from one tick to the next, zero when it
    /// spawns.
    pub state: [f64; 4],
    /// How the quad turns toward the viewer (`SingleQuadParticle.getFacingCameraMode`).
    pub facing: ParticleFacing,
    /// `SingleQuadParticle.roll`: the quad's turn about its own normal, in radians. The game
    /// interpolates it from the value the previous tick left, as vanilla's `oRoll` does, and
    /// draws no turn at all while it is zero. A roll beyond half of `f32::MAX` in either
    /// direction expires the particle, so that the interpolation stays a finite number.
    pub roll: f32,
}

/// How a provider's particle quad is oriented (`SingleQuadParticle.FacingCameraMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ParticleFacing {
    /// `LOOKAT_XYZ`: the quad faces the camera. The default.
    #[default]
    Camera,
    /// `LOOKAT_Y`: the camera's rotation reduced to its Y turn, as vanilla sets it: the
    /// quaternion made of the camera's `y` and `w` components alone. The particle's own position
    /// does not enter.
    AroundY,
    /// A world-fixed orientation, as a unit quaternion `[x, y, z, w]` turning the quad's
    /// default plane (corners in XY, facing +Z). A quad lying flat facing up is
    /// `[-sin(pi/4), 0, 0, cos(pi/4)]`. `roll` turns the quad about its own normal after it.
    Fixed([f32; 4]),
}

/// The particles of one provider's batch. Edit them through [`ParticleBatch::get_mut`],
/// [`ParticleBatch::iter_mut`] or a `for` loop: those methods preserve the batch's length and
/// order. Set [`ModParticle::removed`] to expire a particle. The helpers that build the batch
/// and take it back are internal. If a callback returns a different length, the game expires
/// the batch's particles.
///
/// Dispatch reuses its batch storage once it has enough capacity.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: its field is private; the game builds it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ParticleBatch(Vec<ModParticle>);

impl ParticleBatch {
    /// How many particles the batch holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Whether the batch holds none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// The particle at `index`.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut ModParticle> {
        self.0.get_mut(index)
    }
    /// Every particle, in order.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, ModParticle> {
        self.0.iter_mut()
    }
}
/// Internal constructor: wrap the game's `particles` for a provider tick.
pub fn particle_batch_from_vec(particles: Vec<ModParticle>) -> ParticleBatch {
    ParticleBatch(particles)
}
/// The particles back, for the game to reuse their storage.
pub fn particle_batch_into_vec(batch: ParticleBatch) -> Vec<ModParticle> {
    batch.0
}
impl<'a> IntoIterator for &'a mut ParticleBatch {
    type Item = &'a mut ModParticle;
    type IntoIter = std::slice::IterMut<'a, ModParticle>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

/// What a provider's tick reads of the level around its particles: the loaded world as
/// `Particle` code reads it (`level.getBlockState`, `level.getBrightness`). Read-only. `block_state`
/// reads air where a chunk is absent. `brightness` reads the render level's stored light, with sky
/// always zero in a dimension without skylight. The current client gives absent columns and
/// positions above build height full sky where present and no block light; below a loaded column
/// it gives zero on both layers. Retained and padding light are tracked in Sunrise #2761.
pub trait ParticleLevelView {
    /// `BlockGetter.getBlockState`: the state at `pos`.
    fn block_state(&self, pos: [i32; 3]) -> BlockState;
    /// `BlockAndLightGetter.getBrightness`: the stored light of one layer at `pos`, 0 to 15.
    fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> u8;
}

/// The read-only camera and lighting for one call to [`ParticleProvider::extract`].
// no wire: frame-local renderer inputs.
#[non_exhaustive]
pub struct ParticleExtractView {
    /// World-space camera position in blocks. Subtract it in f64 before narrowing a center to f32.
    pub camera: [f64; 3],
    /// The camera's rotation in XYZW quaternion order.
    pub rotation: [f32; 4],
    /// Interpolation fraction between the previous and current tick.
    pub partial_tick: f32,
    /// The roll at the previous tick (`SingleQuadParticle.oRoll`).
    pub previous_roll: f32,
    /// The ordinary quad's vanilla packed light at the current particle position.
    pub light_coords: u32,
}

impl ParticleExtractView {
    // hidden: the renderer constructs the read-only view at its extraction boundary.
    #[doc(hidden)]
    pub fn new(
        camera: [f64; 3],
        rotation: [f32; 4],
        partial_tick: f32,
        previous_roll: f32,
        light_coords: u32,
    ) -> Self {
        Self {
            camera,
            rotation,
            partial_tick,
            previous_roll,
            light_coords,
        }
    }
}

/// One `SingleQuadParticle` quad submitted to the world's particle-atlas pass.
/// Start from `ParticleQuad::default()` and set the fields the quad needs.
// no wire: frame-local geometry.
#[non_exhaustive]
pub struct ParticleQuad {
    /// The draw layer. This wins over the sprite alpha and the particle's group blend.
    pub layer: ParticleBlend,
    /// Camera-relative center in blocks.
    pub center: [f32; 3],
    /// XYZW quaternion applied to the XY corners, without normalization.
    pub rotation: [f32; 4],
    /// Half-width and half-height in blocks.
    pub half_size: f32,
    /// Index of an existing frame in the particle's sprite set.
    pub frame: u32,
    /// Sprite-local UV rectangle `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Packed `0xAARRGGBB` color.
    pub color: u32,
    /// Vanilla packed light (`LightCoordsUtil.pack` or `smoothPack`).
    pub light_coords: u32,
}

impl Default for ParticleQuad {
    /// A zero-area translucent quad at the camera-relative origin, with identity rotation,
    /// the whole first sprite, opaque white tint and zero light.
    fn default() -> Self {
        Self {
            layer: ParticleBlend::Translucent,
            center: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            half_size: 0.0,
            frame: 0,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: 0xffffffff,
            light_coords: 0,
        }
    }
}

/// The frame's quad output for this particle. Only the current extract call may write to it.
pub trait QuadParticleRenderState {
    /// Append a quad. An unresolved frame or non-finite geometry/UV omits only this record.
    fn add(&mut self, quad: ParticleQuad);
    /// Append the ordinary quad, with built-in interpolation, sprite, tint, light and culling.
    fn add_default(&mut self);
}

/// A mod's particle provider: NeoForge's `ParticleProvider`, held by the game for every particle
/// of its type. Particles of a type share one provider, so per-particle data lives in
/// [`ModParticle::state`], not in the provider.
///
/// A tick has no `Host`: the engine ticks particles inside the render crate, which reaches no
/// gameplay. It does read the level through [`ParticleLevelView`].
pub trait ParticleProvider<T = ()>: Send + Sync + 'static {
    /// `ParticleProvider.createParticle`: create one admitted particle with this type's options,
    /// the current client level, the original position and auxiliary inputs, and the engine's
    /// provider RNG. Auxiliary inputs need not be velocity. Edit the initialized sprite particle
    /// to choose its starting state and frame; return `None` to discard it before it is queued.
    fn create_particle(
        &self,
        _options: &T,
        _level: &dyn ParticleLevelView,
        _position: [f64; 3],
        _aux: [f64; 3],
        _random: &mut dyn RandomSource,
        _particle: &mut ModParticle,
    ) -> Option<()> {
        Some(())
    }

    /// `Particle.tick` for every live particle of this provider's type: one call per type per
    /// tick, with the whole batch. The game has set `previous` and `age`
    /// on each already and expires a particle once `age` passes `lifetime`; the tick sets
    /// `position`, `frame`, `size`, `rgba`, `lifetime`, `removed`, `facing` and `roll`. A number
    /// that is not finite expires the particle. Batch iteration preserves length and order.
    /// Set `removed` to expire a particle. If a callback returns a different length, the engine
    /// expires the batch's particles. Not called when no particle of the type is alive.
    fn tick(&self, particles: &mut ParticleBatch, level: &dyn ParticleLevelView);

    /// `SingleQuadParticle.extract`: called once per visible particle per world frame, separate
    /// from ticking. The default emits the ordinary quad. An override may submit zero or more
    /// quads, and only emits the ordinary quad if it calls `output.add_default()`.
    /// Live scenes only draw the ordinary quad; they do not call this method.
    fn extract(
        &self,
        _particle: &ModParticle,
        _view: &ParticleExtractView,
        output: &mut dyn QuadParticleRenderState,
    ) {
        output.add_default();
    }
}

/// The particle provider registry `register_particle_providers` hands out: NeoForge's
/// `RegisterParticleProvidersEvent`.
pub trait ParticleProviders {
    // hidden: the typed registration wrapper erases the provider for storage.
    #[doc(hidden)]
    fn add_sprite_set(&mut self, key: ParticleKey, provider: Arc<dyn BoundParticleProvider>);
}

impl dyn ParticleProviders + '_ {
    /// Attach a provider to an already registered particle type (`registerSpriteSet`).
    /// Its sprite set comes from `assets/<namespace>/particles/<path>.json`. A later
    /// registration for that type replaces the earlier one. Missing sprite sets draw nothing.
    pub fn register_sprite_set<T: 'static>(
        &mut self,
        ty: ParticleType<T>,
        provider: Arc<dyn ParticleProvider<T>>,
    ) {
        self.add_sprite_set(ty.key(), bind_particle_provider(ty.key(), provider));
    }
}

// hidden: the renderer stores one type-erased provider per bound type.
#[doc(hidden)]
pub trait BoundParticleProvider: Send + Sync {
    fn create_particle(
        &self,
        options: &TypedParticleOptions,
        level: &dyn ParticleLevelView,
        position: [f64; 3],
        aux: [f64; 3],
        random: &mut dyn RandomSource,
        particle: &mut ModParticle,
    ) -> Result<Option<()>, String>;
    fn tick(&self, particles: &mut ParticleBatch, level: &dyn ParticleLevelView);
    fn extract(
        &self,
        particle: &ModParticle,
        view: &ParticleExtractView,
        output: &mut dyn QuadParticleRenderState,
    ) -> Result<(), String>;
}
struct BoundProvider<T> {
    key: ParticleKey,
    provider: Arc<dyn ParticleProvider<T>>,
    callback: ParticleCallback,
}
impl<T: 'static> BoundParticleProvider for BoundProvider<T> {
    fn create_particle(
        &self,
        options: &TypedParticleOptions,
        level: &dyn ParticleLevelView,
        position: [f64; 3],
        aux: [f64; 3],
        random: &mut dyn RandomSource,
        particle: &mut ModParticle,
    ) -> Result<Option<()>, String> {
        self.callback.call(options.key(), "create particle", || {
            let value = options.value().ok_or("wrong particle options type")?;
            Ok(self
                .provider
                .create_particle(value, level, position, aux, random, particle))
        })
    }
    fn tick(&self, particles: &mut ParticleBatch, level: &dyn ParticleLevelView) {
        self.provider.tick(particles, level);
    }
    fn extract(
        &self,
        particle: &ModParticle,
        view: &ParticleExtractView,
        output: &mut dyn QuadParticleRenderState,
    ) -> Result<(), String> {
        self.callback.call(self.key, "extract particle", || {
            self.provider.extract(particle, view, output);
            Ok(())
        })
    }
}

// hidden: the renderer binds the typed provider to its erased dispatch.
#[doc(hidden)]
pub fn bind_particle_provider<T: 'static>(
    key: ParticleKey,
    provider: Arc<dyn ParticleProvider<T>>,
) -> Arc<dyn BoundParticleProvider> {
    let owner = crate::content::crash::current_native_mod().unwrap_or("unowned provider");
    Arc::new(BoundProvider {
        key,
        provider,
        callback: ParticleCallback::new(crate::callback_owner::CallbackOwner::for_mod(owner)),
    })
}

// no wire: process-local callback storage.
/// The game's [`ParticleProviders`]: provider registrations by particle type's slot.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
pub struct ParticleProviderRegistry {
    providers: crate::runtime::RegistrationMap<Arc<dyn BoundParticleProvider>, u32>,
}

const REGISTRATION: (crate::generated::HookId, &str) = (
    crate::generated::HookId::RegisterParticleProviders,
    "register_particle_providers",
);

#[derive(Default)]
struct ProviderEdits(crate::runtime::RegistrationEdits<Arc<dyn BoundParticleProvider>, u32>);

impl ParticleProviders for ProviderEdits {
    fn add_sprite_set(&mut self, key: ParticleKey, provider: Arc<dyn BoundParticleProvider>) {
        self.0.insert(key.0, provider);
    }
}

impl ParticleProviders for ParticleProviderRegistry {
    fn add_sprite_set(&mut self, key: ParticleKey, provider: Arc<dyn BoundParticleProvider>) {
        self.providers
            .finish(REGISTRATION, true, [(key.0, provider)]);
    }
}

impl ParticleProviderRegistry {
    /// Apply the current listener's provider edits only if its registration succeeds.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn ParticleProviders) -> bool,
    ) {
        let mut edits = ProviderEdits::default();
        let publish = listen(&mut edits);
        self.providers
            .finish(REGISTRATION, publish, edits.0.ordered());
    }

    /// Each type's provider, the last successful registration for a type winning.
    pub fn into_providers(self) -> HashMap<ParticleKey, Arc<dyn BoundParticleProvider>> {
        self.providers
            .into_values()
            .into_iter()
            .map(|(slot, provider)| (ParticleKey(slot), provider))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Drift(f64);
    impl ParticleProvider for Drift {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                particle.position[0] += self.0;
            }
        }
    }

    fn particle() -> ModParticle {
        ModParticle {
            position: [0.0; 3],
            previous: [0.0; 3],
            velocity: [0.0; 3],
            age: 1,
            lifetime: 20,
            size: 0.1,
            rgba: [1.0; 4],
            frame: 0,
            removed: false,
            state: [0.0; 4],
            facing: ParticleFacing::default(),
            roll: 0.0,
        }
    }

    struct Air;
    impl ParticleLevelView for Air {
        fn block_state(&self, _: [i32; 3]) -> BlockState {
            unreachable!("the test providers read no blocks")
        }
        fn brightness(&self, _: LightLayer, _: [i32; 3]) -> u8 {
            0
        }
    }

    #[test]
    fn only_a_successful_later_registration_replaces_a_provider() {
        let mut registry = ParticleProviderRegistry::default();
        let a = ParticleType::new(ParticleKey(0));
        let b = ParticleType::new(ParticleKey(1));
        let event = &mut registry as &mut dyn ParticleProviders;
        event.register_sprite_set(a, Arc::new(Drift(1.0)));
        event.register_sprite_set(b, Arc::new(Drift(2.0)));
        event.register_sprite_set(a, Arc::new(Drift(3.0)));
        registry.register_listener(&mut |providers| {
            providers.register_sprite_set(a, Arc::new(Drift(4.0)));
            false
        });
        let providers = registry.into_providers();
        assert_eq!(providers.len(), 2);
        let mut batch_a = particle_batch_from_vec(vec![particle()]);
        let mut batch_b = particle_batch_from_vec(vec![particle()]);
        providers[&a.key()].tick(&mut batch_a, &Air);
        providers[&b.key()].tick(&mut batch_b, &Air);
        let at = |batch: &mut ParticleBatch| batch.get_mut(0).unwrap().position[0];
        assert_eq!((at(&mut batch_a), at(&mut batch_b)), (3.0, 2.0));
    }
}
