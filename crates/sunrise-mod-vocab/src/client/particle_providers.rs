//! Typed logical-client sprite factories and the owned instances they create.

use super::particle_types::TypedParticleOptions;
use crate::callback_owner::{CallbackOwner, CallbackValue};
use crate::{BlockState, LightLayer, ParticleBlend, ParticleKey, ParticleType, RandomSource};
use std::{collections::HashMap, marker::PhantomData, sync::Arc};

const CALLBACK: &str = "particle provider";

/// Orientation of an extracted particle quad relative to the camera.
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

/// The read-only camera and lighting for one call to [`CustomParticle::extract`].
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

/// Inputs borrowed on the particle engine's owning thread for one construction call.
/// Keep only owned data in the particle returned by the provider.
// no wire: holds live options, level and random loans for the duration of the call.
#[non_exhaustive]
pub struct ParticleSpawn<'a, T> {
    /// The registered type’s value, whether supplied locally or decoded by its stream codec.
    pub options: &'a T,
    /// The raw position the caller requested, in world coordinates.
    pub position: [f64; 3],
    /// The raw auxiliary velocity slots the caller supplied, before constructor defaults.
    pub velocity: [f64; 3],
    /// Read-only access to loaded block states and stored light.
    pub level: &'a dyn ParticleLevelView,
    /// The engine's provider random. Draws advance the live stream in call order.
    pub random: &'a mut dyn RandomSource,
}

/// Read-only access to one live particle, borrowed for the current callback only.
// no wire: reads the engine-owned particle directly, not a snapshot.
pub trait ParticleView {
    /// The current position, in world coordinates.
    fn position(&self) -> [f64; 3];
    /// The previous tick's position, which rendering interpolates from.
    fn previous_position(&self) -> [f64; 3];
    /// The current velocity in blocks per tick.
    fn velocity(&self) -> [f64; 3];
    /// `Particle.age`: zero at construction, advanced by the particle's tick.
    fn age(&self) -> i32;
    /// `Particle.lifetime`: the base tick removes the particle when its pre-increment age reaches this.
    fn lifetime(&self) -> i32;
    /// Whether the latest collision step hit the ground.
    fn on_ground(&self) -> bool;
    /// Whether the particle has not been removed.
    fn is_alive(&self) -> bool;
    /// `SingleQuadParticle.quadSize`: the billboard half-width in blocks.
    fn size(&self) -> f32;
    /// Red, green, blue and alpha of the quad.
    fn rgba(&self) -> [f32; 4];
    /// The selected zero-based index in the bound sprite set.
    fn sprite_index(&self) -> usize;
    /// How the quad faces the viewer or world.
    fn facing(&self) -> ParticleFacing;
    /// `SingleQuadParticle.roll`: rotation about the quad normal, in radians.
    fn roll(&self) -> f32;
    /// The prior roll that rendering interpolates from if the current roll is nonzero.
    fn previous_roll(&self) -> f32;
}

/// Authoritative access to one particle during its construction or tick on the owning thread.
/// A write takes effect immediately. Invalid setter inputs are reported with the mod's name
/// and return `false` without changing the field or killing the particle. Do not retain this view.
// no wire: mutates a live particle and borrows its level and sprite set in process.
pub trait ParticleTickView: ParticleView {
    /// `Particle.setPos`: set the position and rebuild its collision box.
    fn set_position(&mut self, position: [f64; 3]) -> bool;
    /// Set the interpolation origin, for example to snap to a teleported position.
    fn set_previous_position(&mut self, position: [f64; 3]) -> bool;
    /// `Particle.setParticleSpeed`: replace the velocity, without moving yet.
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool;
    /// Set the current age in ticks.
    fn set_age(&mut self, age: i32);
    /// `Particle.setLifetime`: set the age at which the base tick will remove this particle.
    fn set_lifetime(&mut self, lifetime: i32);
    /// Set the gravity coefficient applied by the base tick.
    fn set_gravity(&mut self, gravity: f32) -> bool;
    /// Set the velocity multiplier applied by the base tick after movement.
    fn set_friction(&mut self, friction: f32) -> bool;
    /// Whether to collide with blocks when moving.
    fn set_has_physics(&mut self, has_physics: bool);
    /// Set the billboard half-width, independent of the physics box.
    fn set_size(&mut self, size: f32) -> bool;
    /// Set red, green, blue and alpha without clamping them.
    fn set_rgba(&mut self, rgba: [f32; 4]) -> bool;
    /// Select an exact zero-based sprite index. An absent index is reported, not clamped.
    fn set_sprite_index(&mut self, index: usize) -> bool;
    /// `SingleQuadParticle.setSpriteFromAge`: select `age * (frame_count - 1) / lifetime`
    /// with Java integer arithmetic. A removed particle is left unchanged; zero lifetime
    /// or a missing index is reported. Returns whether a sprite was selected.
    fn set_sprite_from_age(&mut self) -> bool;
    /// Set the quad facing. A fixed quaternion must be finite and nonzero and is normalized.
    fn set_facing(&mut self, facing: ParticleFacing) -> bool;
    /// Set rotation about the quad normal, in radians.
    fn set_roll(&mut self, roll: f32) -> bool;
    /// `Particle.move`: apply a displacement using the particle's collision box and live level.
    fn move_by(&mut self, displacement: [f64; 3]) -> bool;
    /// Record the previous position and roll, increment age, and remove at the same pre-increment
    /// lifetime check as `Particle.tick`. Returns false on expiry. Does not move the particle.
    fn advance_age(&mut self) -> bool;
    /// Run `Particle.tick`: capture the previous pose, check lifetime, then apply gravity,
    /// movement and friction. Sprite selection is separate.
    fn tick_base(&mut self);
    /// `Particle.remove`: mark this particle to be retired by the engine.
    fn remove(&mut self);
}

/// An owned native particle instance. Its fields are its private per-particle state. It is
/// constructed, ticked, drawn and destroyed on the particle engine's owning thread.
// no wire: owns executable behavior and private instance state.
pub trait CustomParticle: Send + Sync + 'static {
    /// Tick this instance with live particle and level loans. The default runs the base tick.
    /// An override can run `tick_base` or use `advance_age` and move the particle itself.
    fn tick(&mut self, particle: &mut dyn ParticleTickView, _level: &dyn ParticleLevelView) {
        particle.tick_base();
    }

    /// Extract zero or more sprite quads for this frame without changing simulation state.
    /// The default submits the ordinary quad with its interpolated pose and sprite.
    fn extract(
        &self,
        _particle: &dyn ParticleView,
        _view: &ParticleExtractView,
        output: &mut dyn QuadParticleRenderState,
    ) {
        output.add_default();
    }
}

/// The provider bound to one sprite set for a registered [`ParticleType<T>`]. Live state
/// belongs to the [`CustomParticle`] returned by each call, not to the shared provider.
// no wire: the engine calls retained Rust provider code in process.
pub trait ParticleProvider<T = ()>: Send + Sync + 'static {
    /// Edit the starting pose and return the new particle's owned behavior, or return `None`
    /// to decline this request. Called before group admission, like `ParticleProvider.createParticle`.
    fn create_particle(
        &self,
        spawn: &mut ParticleSpawn<'_, T>,
        particle: &mut dyn ParticleTickView,
    ) -> Option<Box<dyn CustomParticle>>;
}

/// The sprite set lent to a factory after the atlas and particle descriptions are loaded.
// no wire: identifies the available frames in a bound local resource generation.
// exhaustive: private field; mods read through methods.
#[derive(Clone, Copy, Debug)]
pub struct ParticleSpriteSet {
    frame_count: usize,
}

impl ParticleSpriteSet {
    /// Number of selectable frames in the sprite set.
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }
}

/// The staged typed registrations of a logical-client listener.
pub trait ParticleProviders {
    /// Internal erased registration. Mods call [`Self::register_sprite_set`] on the trait object.
    // hidden: erased registration seam called by the typed register_sprite_set method.
    #[doc(hidden)]
    fn add_sprite_set(&mut self, key: ParticleKey, factory: Arc<ParticleProviderFactory>);
}

impl dyn ParticleProviders + '_ {
    /// Bind an already registered typed particle to a factory. Each resource load calls
    /// the factory with the current sprite set. It is not called during hook dispatch.
    pub fn register_sprite_set<T: 'static, P: ParticleProvider<T>>(
        &mut self,
        ty: ParticleType<T>,
        factory: impl Fn(ParticleSpriteSet) -> P + Send + Sync + 'static,
    ) {
        self.add_sprite_set(ty.key(), particle_provider_factory(ty, factory));
    }
}

/// The renderer's erased provider. It lends the value of the registered type without encoding it.
// hidden: erased renderer provider trait exposed only through __private.
#[doc(hidden)]
pub trait BoundParticleProvider: Send + Sync {
    fn create_particle(
        &self,
        options: &TypedParticleOptions,
        position: [f64; 3],
        velocity: [f64; 3],
        level: &dyn ParticleLevelView,
        random: &mut dyn RandomSource,
        particle: &mut dyn ParticleTickView,
    ) -> Result<Option<Box<dyn CustomParticle>>, String>;
}

struct BoundProvider<T, P>(P, PhantomData<fn(T)>);

impl<T: 'static, P: ParticleProvider<T>> BoundParticleProvider for BoundProvider<T, P> {
    fn create_particle(
        &self,
        options: &TypedParticleOptions,
        position: [f64; 3],
        velocity: [f64; 3],
        level: &dyn ParticleLevelView,
        random: &mut dyn RandomSource,
        particle: &mut dyn ParticleTickView,
    ) -> Result<Option<Box<dyn CustomParticle>>, String> {
        let options = options.value().ok_or("wrong particle options type")?;
        let mut spawn = ParticleSpawn {
            options,
            position,
            velocity,
            level,
            random,
        };
        Ok(self.0.create_particle(&mut spawn, particle))
    }
}

type Factory = dyn Fn(ParticleSpriteSet) -> Box<dyn BoundParticleProvider> + Send + Sync;

/// An attributed factory and the independently quarantined callbacks of its registration.
// hidden: erased renderer factory struct exposed only through __private.
// exhaustive: private fields; constructed only by particle_provider_factory.
#[doc(hidden)]
pub struct ParticleProviderFactory {
    factory: CallbackValue<Box<Factory>>,
    bind: CallbackOwner,
    pub create: CallbackOwner,
    pub tick: CallbackOwner,
    pub extract: CallbackOwner,
    pub dispose: CallbackOwner,
}

impl ParticleProviderFactory {
    pub fn mod_id(&self) -> &'static str {
        self.bind.id()
    }

    pub fn bind(
        &self,
        frame_count: usize,
    ) -> Result<CallbackValue<Box<dyn BoundParticleProvider>>, String> {
        self.bind
            .call(CALLBACK, "bind_particle_provider", || {
                (self.factory.value())(ParticleSpriteSet { frame_count })
            })
            .map(|provider| {
                CallbackValue::new(
                    self.dispose.clone(),
                    CALLBACK,
                    "drop_particle_provider",
                    provider,
                )
            })
    }
}

/// Create an erased factory under the current listener's mod scope. This is also used by
/// scoped renderer fixtures, which do not publish a process-wide provider table.
// hidden: erased factory builder exposed only through __private.
#[doc(hidden)]
pub fn particle_provider_factory<T: 'static, P: ParticleProvider<T>>(
    _ty: ParticleType<T>,
    factory: impl Fn(ParticleSpriteSet) -> P + Send + Sync + 'static,
) -> Arc<ParticleProviderFactory> {
    let id = crate::content::crash::current_native_mod().unwrap_or("unowned provider");
    let owner = || CallbackOwner::for_mod(id);
    let factory: Box<Factory> =
        Box::new(move |sprites| Box::new(BoundProvider::<T, P>(factory(sprites), PhantomData)));
    Arc::new(ParticleProviderFactory {
        factory: CallbackValue::new(owner(), CALLBACK, "drop_particle_provider_factory", factory),
        bind: owner(),
        create: owner(),
        tick: owner(),
        extract: owner(),
        dispose: owner(),
    })
}

// no wire: process-local callback storage.
/// The game's [`ParticleProviders`]: provider registrations by particle type's slot.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
pub struct ParticleProviderRegistry {
    providers: crate::runtime::RegistrationMap<Arc<ParticleProviderFactory>, u32>,
}

const REGISTRATION: (crate::generated::HookId, &str) = (
    crate::generated::HookId::RegisterParticleProviders,
    "register_particle_providers",
);

#[derive(Default)]
struct ProviderEdits(crate::runtime::RegistrationEdits<Arc<ParticleProviderFactory>, u32>);

impl ParticleProviders for ProviderEdits {
    fn add_sprite_set(&mut self, key: ParticleKey, provider: Arc<ParticleProviderFactory>) {
        self.0.insert(key.0, provider);
    }
}

impl ParticleProviders for ParticleProviderRegistry {
    fn add_sprite_set(&mut self, key: ParticleKey, provider: Arc<ParticleProviderFactory>) {
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

    /// Each type's factory, the last successful registration for a type winning.
    pub fn into_providers(self) -> HashMap<ParticleKey, Arc<ParticleProviderFactory>> {
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
    use std::sync::Mutex;

    struct Provider;
    impl ParticleProvider for Provider {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            None
        }
    }

    #[test]
    fn only_a_successful_later_registration_replaces_a_provider() {
        let mut registry = ParticleProviderRegistry::default();
        let a = ParticleType::new(ParticleKey(0));
        let b = ParticleType::new(ParticleKey(1));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let factory = |marker| {
            let observed = observed.clone();
            move |sprites: ParticleSpriteSet| {
                observed
                    .lock()
                    .unwrap()
                    .push((marker, sprites.frame_count()));
                Provider
            }
        };
        let event = &mut registry as &mut dyn ParticleProviders;
        event.register_sprite_set(a, factory(1));
        event.register_sprite_set(b, factory(2));
        event.register_sprite_set(a, factory(3));
        registry.register_listener(&mut |event| {
            event.register_sprite_set(a, factory(4));
            false
        });
        assert!(observed.lock().unwrap().is_empty());
        let factories = registry.into_providers();
        assert_eq!(factories.len(), 2);
        let _a = factories[&a.key()].bind(8).unwrap();
        let _b = factories[&b.key()].bind(9).unwrap();
        assert_eq!(*observed.lock().unwrap(), [(3, 8), (2, 9)]);
    }
}
