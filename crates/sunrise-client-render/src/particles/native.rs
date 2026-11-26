//! Spawn and tick owned mod particles under the shared callback policy.
use super::{Particle, ParticleEngine, ParticleLevel, ParticleOptions, Sprite};
use mod_api::{
    __private::{
        BorrowedRandom, BoundParticleProvider, ParticleProviderFactory, TypedParticleOptions,
        game::CallbackValue,
    },
    BlockState, CustomParticle, LightLayer, ParticleExtractView, ParticleFacing, ParticleKey,
    ParticleLevelView, ParticleTickView, ParticleView, QuadParticleRenderState,
};
use std::{
    collections::HashMap,
    fmt::Debug,
    ops::Deref,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Requests made by one particle tick, submitted through normal spawn limits.
#[derive(Default)]
pub struct ParticleUpdate {
    /// An index in this particle's declared sprite set. Out-of-range is clamped.
    pub frame: Option<usize>,
    pub spawned: Vec<(ParticleOptions, [f64; 3], [f64; 3])>,
}

/// Engine-side controller for a compiled particle, separate from mod providers.
/// Do not retain world references; inspect `level` each tick.
pub trait ParticleController: Debug + Send + Sync + 'static {
    /// Call `particle.advance_age()` for ordinary lifetime/interpolation, or
    /// explicitly maintain those fields for an overridden source tick.
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
}

pub(super) trait Controller: Debug + Send + Sync {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
}
impl<T: ParticleController> Controller for T {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate {
        ParticleController::tick(self, particle, level)
    }
}

/// The mods' particle providers by registered key: set once before the first resource load,
/// and read by every engine built after.
static REGISTERED: OnceLock<HashMap<ParticleKey, Arc<ParticleProviderFactory>>> = OnceLock::new();

/// Sets the mods' particle providers (`register_particle_providers`), once, before
/// the resources first load; each engine then resolves its sprite sets' providers at every load.
/// False when already set.
pub fn register_particle_providers(
    providers: HashMap<ParticleKey, Arc<ParticleProviderFactory>>,
) -> bool {
    REGISTERED.set(providers).is_ok()
}

/// What [`register_particle_providers`] set; none before.
pub(super) fn registered() -> &'static HashMap<ParticleKey, Arc<ParticleProviderFactory>> {
    static NONE: std::sync::LazyLock<HashMap<ParticleKey, Arc<ParticleProviderFactory>>> =
        std::sync::LazyLock::new(HashMap::new);
    REGISTERED.get().unwrap_or(&NONE)
}

pub(super) struct BoundProvider {
    pub(super) factory: Arc<ParticleProviderFactory>,
    pub(super) provider: CallbackValue<Box<dyn BoundParticleProvider>>,
}

/// The engine's sole owner of a particle instance and its sprite orientation.
pub(super) struct Provided {
    factory: Arc<ParticleProviderFactory>,
    instance: CallbackValue<Box<dyn CustomParticle>>,
    presentation: Presentation,
    live: Arc<AtomicUsize>,
}

struct Presentation {
    frame: usize,
    facing: ParticleFacing,
}

impl Drop for Provided {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Debug for Provided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provided")
            .field("mod", &self.factory.mod_id())
            .finish_non_exhaustive()
    }
}

impl Provided {
    pub(super) fn tick(
        &mut self,
        engine: &ParticleEngine,
        particle: &mut Particle,
        level: &ParticleLevel<'_>,
    ) {
        let owner = &self.factory.tick;
        if !owner.is_enabled() {
            if owner.is_poisoned() {
                particle.remove();
            }
            return;
        }
        let Some(states) = (engine.block_state_source)() else {
            return;
        };
        let level = ProviderLevel {
            level,
            states: (*states).as_ref(),
        };
        let set = particle.set.expect("provided particle has a sprite set");
        let mut view = LiveParticle {
            particle: &mut *particle,
            presentation: &mut self.presentation,
            context: TickContext {
                level: level.level,
                sprites: &engine.set_sprites[set as usize],
                factory: &self.factory,
            },
        };
        if let Err(error) = owner.call("particle provider", "tick_custom_particle", || {
            self.instance.value_mut().tick(&mut view, &level);
        }) {
            log::warn!("Could not tick particle: {error}");
            if owner.is_poisoned() {
                particle.remove();
            }
        }
    }

    pub(super) fn extract(
        &self,
        particle: &Particle,
        view: &ParticleExtractView,
        output: &mut dyn QuadParticleRenderState,
    ) -> Result<(), String> {
        if !self.factory.extract.is_enabled() {
            return Ok(());
        }
        self.factory
            .extract
            .call("particle provider", "extract_custom_particle", || {
                let particle = LiveParticle {
                    particle,
                    presentation: &self.presentation,
                    context: (),
                };
                self.instance.value().extract(&particle, view, output);
            })
    }

    /// The quad's rotation at `partial` of a tick (`SingleQuadParticle.extract`): the facing's
    /// quaternion (`LOOKAT_XYZ` the camera's, `LOOKAT_Y` its y and w alone, as vanilla sets it,
    /// or the world-fixed one), then `rotateZ` of `Mth.lerp(partial, oRoll, roll)` when the
    /// particle's roll is not zero. All of it in JOML's f32 maths.
    pub(super) fn rotation(
        &self,
        camera: glam::Quat,
        o_roll: f32,
        roll: f32,
        partial: f32,
    ) -> glam::Quat {
        use crate::camera::joml::Quat;
        let facing = match self.presentation.facing {
            ParticleFacing::Camera => Quat {
                x: camera.x,
                y: camera.y,
                z: camera.z,
                w: camera.w,
            },
            ParticleFacing::AroundY => Quat {
                x: 0.0,
                y: camera.y,
                z: 0.0,
                w: camera.w,
            },
            ParticleFacing::Fixed(q) => Quat {
                x: q[0],
                y: q[1],
                z: q[2],
                w: q[3],
            },
        };
        let turned = if roll == 0.0 {
            facing
        } else {
            facing.rotate_z(o_roll + partial * (roll - o_roll))
        };
        glam::Quat::from_xyzw(turned.x, turned.y, turned.z, turned.w)
    }
}

// Read and write callbacks share the same getters. Only the tick/create loan carries
// mutable state and the level/sprite context for ParticleTickView.
struct LiveParticle<P, S, C> {
    particle: P,
    presentation: S,
    context: C,
}

struct TickContext<'a, 'l> {
    level: &'a ParticleLevel<'l>,
    sprites: &'a [u32],
    factory: &'a ParticleProviderFactory,
}

type TickParticle<'a, 'l> =
    LiveParticle<&'a mut Particle, &'a mut Presentation, TickContext<'a, 'l>>;

impl TickParticle<'_, '_> {
    #[cold]
    fn reject(&self, input: &'static str) -> bool {
        let factory = self.context.factory;
        log::warn!(
            "Particle provider from mod {}: invalid {input}",
            factory.mod_id(),
        );
        false
    }
}

impl<P: Deref<Target = Particle>, S: Deref<Target = Presentation>, C> ParticleView
    for LiveParticle<P, S, C>
{
    fn position(&self) -> [f64; 3] {
        self.particle.position
    }
    fn previous_position(&self) -> [f64; 3] {
        self.particle.previous
    }
    fn velocity(&self) -> [f64; 3] {
        self.particle.velocity
    }
    fn age(&self) -> i32 {
        self.particle.age
    }
    fn lifetime(&self) -> i32 {
        self.particle.lifetime
    }
    fn on_ground(&self) -> bool {
        self.particle.on_ground
    }
    fn is_alive(&self) -> bool {
        self.particle.is_alive()
    }
    fn size(&self) -> f32 {
        self.particle.size
    }
    fn rgba(&self) -> [f32; 4] {
        self.particle.rgba
    }
    fn sprite_index(&self) -> usize {
        self.presentation.frame
    }
    fn facing(&self) -> ParticleFacing {
        self.presentation.facing
    }
    fn roll(&self) -> f32 {
        self.particle.roll
    }
    fn previous_roll(&self) -> f32 {
        self.particle.o_roll
    }
}

impl ParticleTickView for TickParticle<'_, '_> {
    fn set_position(&mut self, position: [f64; 3]) -> bool {
        if !position.into_iter().all(f64::is_finite) {
            return self.reject("position");
        }
        self.particle.set_pos(position);
        true
    }

    fn set_previous_position(&mut self, position: [f64; 3]) -> bool {
        if !position.into_iter().all(f64::is_finite) {
            return self.reject("previous position");
        }
        self.particle.previous = position;
        true
    }

    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.into_iter().all(f64::is_finite) {
            return self.reject("velocity");
        }
        self.particle.velocity = velocity;
        true
    }

    fn set_age(&mut self, age: i32) {
        self.particle.age = age;
    }
    fn set_lifetime(&mut self, lifetime: i32) {
        self.particle.lifetime = lifetime;
    }
    fn set_has_physics(&mut self, has_physics: bool) {
        self.particle.has_physics = has_physics;
    }

    fn set_gravity(&mut self, gravity: f32) -> bool {
        if !gravity.is_finite() {
            return self.reject("gravity");
        }
        self.particle.gravity = gravity;
        true
    }

    fn set_friction(&mut self, friction: f32) -> bool {
        if !friction.is_finite() {
            return self.reject("friction");
        }
        self.particle.friction = friction;
        true
    }

    fn set_size(&mut self, size: f32) -> bool {
        if !size.is_finite() {
            return self.reject("quad size");
        }
        self.particle.size = size;
        true
    }

    fn set_rgba(&mut self, rgba: [f32; 4]) -> bool {
        if !rgba.into_iter().all(f32::is_finite) {
            return self.reject("quad color");
        }
        self.particle.rgba = rgba;
        true
    }

    fn set_sprite_index(&mut self, index: usize) -> bool {
        let Some(&sprite) = self.context.sprites.get(index) else {
            return self.reject("sprite index");
        };
        self.presentation.frame = index;
        self.particle.sprite = Sprite::Index(sprite);
        true
    }

    fn set_sprite_from_age(&mut self) -> bool {
        if !self.particle.is_alive() {
            return false;
        }
        let lifetime = self.particle.lifetime;
        if lifetime == 0 {
            return self.reject("zero lifetime for sprite selection");
        }
        let index = self
            .particle
            .age
            .wrapping_mul((self.context.sprites.len() - 1) as i32)
            .wrapping_div(lifetime);
        self.set_sprite_index(index as usize)
    }

    fn set_facing(&mut self, facing: ParticleFacing) -> bool {
        self.presentation.facing = if let ParticleFacing::Fixed(q) = facing {
            // Widen for the norm so every finite nonzero f32 quaternion can be normalized.
            let q = glam::DQuat::from_array(q.map(f64::from));
            let length = q.length();
            if !length.is_finite() || length == 0.0 {
                return self.reject("fixed facing");
            }
            ParticleFacing::Fixed(q.to_array().map(|value| (value / length) as f32))
        } else {
            facing
        };
        true
    }

    fn set_roll(&mut self, roll: f32) -> bool {
        if !roll.is_finite() {
            return self.reject("roll");
        }
        self.particle.roll = roll;
        true
    }

    fn move_by(&mut self, displacement: [f64; 3]) -> bool {
        if !displacement.into_iter().all(f64::is_finite) {
            return self.reject("displacement");
        }
        self.particle.move_by(self.context.level, displacement);
        true
    }

    fn advance_age(&mut self) -> bool {
        self.particle.advance_age()
    }

    fn tick_base(&mut self) {
        self.particle.o_roll = self.particle.roll;
        self.particle.base_tick(self.context.level);
    }

    fn remove(&mut self) {
        self.particle.remove();
    }
}

/// `ParticleLevelView` over the engine's level: the loaded chunks' states and stored light.
pub(super) struct ProviderLevel<'a, 'l> {
    pub(super) level: &'a ParticleLevel<'l>,
    /// Every engine state as a mod reads it, by state id.
    pub(super) states: &'a [BlockState],
}
impl ParticleLevelView for ProviderLevel<'_, '_> {
    fn block_state(&self, [x, y, z]: [i32; 3]) -> BlockState {
        // The table holds every state the world can: an id outside it is the game's bug.
        self.states[self.level.block(x, y, z).0 as usize]
    }
    fn brightness(&self, layer: LightLayer, [x, y, z]: [i32; 3]) -> u8 {
        let world = self.level.world;
        match layer {
            LightLayer::Block => world.light(x, y, z) & 15,
            // A dimension without skylight has no sky layer to read.
            LightLayer::Sky if !world.shape.sky_light => 0,
            LightLayer::Sky => world.light(x, y, z) >> 4,
        }
    }
}

impl ParticleEngine {
    pub(super) fn make_typed_particle(
        &mut self,
        level: &ParticleLevel<'_>,
        value: &TypedParticleOptions,
        position: [f64; 3],
        velocity: [f64; 3],
        seed: i64,
    ) -> Result<Option<Particle>, String> {
        let Some(set) = self.typed_sets.get(&value.key()) else {
            return Ok(None);
        };
        let Some(bound) = &set.provider else {
            return Ok(None);
        };
        let Some(states) = (self.block_state_source)() else {
            return Ok(None);
        };
        let level = ProviderLevel {
            level,
            states: (*states).as_ref(),
        };
        let mut q = self.make_sprite_particle(set, position, velocity, seed);
        let sprites = &self.set_sprites[set.set as usize];
        let frame = sprites
            .iter()
            .position(|&s| matches!(q.sprite, Sprite::Index(i) if i == s))
            .expect("native sprite particle starts in its own set");
        let mut presentation = Presentation {
            frame,
            facing: ParticleFacing::Camera,
        };
        let mut view = LiveParticle {
            particle: &mut q,
            presentation: &mut presentation,
            context: TickContext {
                level: level.level,
                sprites,
                factory: &bound.factory,
            },
        };
        let instance =
            bound
                .factory
                .create
                .call("particle provider", "create_custom_particle", || {
                    bound.provider.value().create_particle(
                        value,
                        position,
                        velocity,
                        &level,
                        &mut BorrowedRandom(&mut self.random),
                        &mut view,
                    )
                })??;
        let Some(instance) = instance else {
            return Ok(None);
        };
        self.provided_live.fetch_add(1, Ordering::Relaxed);
        q.behaviour = super::Behaviour::Controlled { translucent: true };
        q.set = Some(set.set);
        q.provided = Some(Box::new(Provided {
            factory: Arc::clone(&bound.factory),
            instance: CallbackValue::new(
                bound.factory.dispose.clone(),
                "particle provider",
                "drop_custom_particle",
                instance,
            ),
            presentation,
            live: Arc::clone(&self.provided_live),
        }));
        Ok((!q.removed).then_some(q))
    }
}

#[cfg(test)]
pub(super) mod tests;
