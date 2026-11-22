//! Opt-in, compiled particle behavior. The ordinary engine still owns the
//! particle, its sprite set, interpolation, rendering and total count limits.
use super::{Particle, ParticleLevel, ParticleOptions};
use mod_api::{
    __private::{BoundParticleProvider, TypedParticleOptions},
    BlockState, LightLayer, ModParticle, ParticleFacing, ParticleKey, ParticleLevelView,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::HashMap, fmt::Debug, sync::Arc, sync::OnceLock};

/// Requests made by one particle tick, submitted through normal spawn limits.
#[derive(Default)]
pub struct ParticleUpdate {
    /// An index in this particle's declared sprite set. Out-of-range is clamped.
    pub frame: Option<usize>,
    pub spawned: Vec<(ParticleOptions, [f64; 3], [f64; 3])>,
}

/// Owned behavior for a compiled native particle. Cloning a particle clones its
/// controller state. Do not retain world references; inspect `level` each tick.
pub trait ParticleController: Clone + Debug + Send + Sync + 'static {
    /// Call `particle.advance_age()` for ordinary lifetime/interpolation, or
    /// explicitly maintain those fields for an overridden source tick.
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
}

pub(super) trait Controller: Debug + Send + Sync {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate;
    fn clone_box(&self) -> Box<dyn Controller>;
}
impl<T: ParticleController> Controller for T {
    fn tick(&mut self, particle: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate {
        ParticleController::tick(self, particle, level)
    }
    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }
}
impl Clone for Box<dyn Controller> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// The mods' particle providers by registered key: set once before the first resource load,
/// and read by every engine built after.
static REGISTERED: OnceLock<HashMap<ParticleKey, Arc<dyn BoundParticleProvider>>> = OnceLock::new();

/// Sets the mods' particle providers (`register_particle_providers`), once, before
/// the resources first load; each engine then resolves its sprite sets' providers at every load.
/// False when already set.
pub fn register_particle_providers(
    providers: HashMap<ParticleKey, Arc<dyn BoundParticleProvider>>,
) -> bool {
    REGISTERED.set(providers).is_ok()
}

/// What [`register_particle_providers`] set; none before.
pub(super) fn registered() -> &'static HashMap<ParticleKey, Arc<dyn BoundParticleProvider>> {
    static NONE: std::sync::LazyLock<HashMap<ParticleKey, Arc<dyn BoundParticleProvider>>> =
        std::sync::LazyLock::new(HashMap::new);
    REGISTERED.get().unwrap_or(&NONE)
}

impl super::ParticleEngine {
    pub(super) fn make_typed_particle(
        &mut self,
        world: &super::World,
        value: &TypedParticleOptions,
        position: [f64; 3],
        aux: [f64; 3],
        seed: i64,
    ) -> Result<Option<Particle>, String> {
        let Some((set, (_, provider))) = self
            .typed_sets
            .get(&value.key())
            .and_then(|set| Some((set, set.provider.as_ref()?)))
        else {
            return Ok(None);
        };
        let Some(states) = (self.block_state_source)() else {
            return Ok(None);
        };
        let level = ProviderLevel {
            world,
            block: |x, y, z| world.block(x, y, z),
            states: (*states).as_ref(),
        };
        let mut particle = self.make_sprite_particle(set, position, aux, seed);
        let Some(provided) = &particle.provided else {
            return Ok(None);
        };
        let mut view = particle.provider_view(provided);
        if provider
            .create_particle(
                value,
                &level,
                position,
                aux,
                &mut mod_api::__private::BorrowedRandom(&mut self.random),
                &mut view,
            )?
            .is_none()
        {
            return Ok(None);
        }
        if let Some(frame) = particle.take(&view) {
            self.set_sprite_frame(&mut particle, frame);
        }
        Ok((!particle.removed).then_some(particle))
    }
}

/// How far from the origin a provider may move a particle: the world border's limit. Past it, or
/// not a number, the particle expires rather than reach the renderer.
const LIMIT: f64 = 3.0e7;

/// How large a roll a provider may leave, in radians: half of `f32::MAX`, so the difference of
/// two accepted rolls, which the renderer interpolates, is a finite number. Past it, or not a
/// number, the particle expires.
const ROLL_LIMIT: f32 = f32::MAX / 2.0;

/// A particle of a mod's code-defined type. It owns the particle's behaviour for the engine: age
/// and expiry stay the engine's, the motion is the provider's, called once per type per tick with
/// every live particle of the type ([`Batch::run`]). Holds the provider the type resolved to when
/// its sprite sets loaded, so a tick dispatches straight to it.
pub(super) struct Provided {
    provider: Arc<dyn BoundParticleProvider>,
    /// The provider's dense slot among the registered ones, which its batch is filled by.
    slot: u32,
    /// `ModParticle::state`, kept from one tick to the next.
    state: [f64; 4],
    /// The frame of the sprite set drawn, as the provider last saw it.
    frame: u32,
    /// The engine aged the particle this tick and the provider has not run on it yet.
    pending: bool,
    /// How the quad turns toward the viewer, as the provider last set it.
    facing: Facing,
    /// The engine's count of live provider particles, which this one is part of until it drops.
    live: Arc<AtomicUsize>,
}
/// A provider particle's facing, as the renderer keeps it (`ParticleFacing`, the quaternion
/// already normalised).
#[derive(Clone, Copy, Debug)]
enum Facing {
    Camera,
    AroundY,
    Fixed(glam::Quat),
}
impl Clone for Provided {
    fn clone(&self) -> Self {
        self.live.fetch_add(1, Ordering::Relaxed);
        Self {
            provider: self.provider.clone(),
            slot: self.slot,
            state: self.state,
            frame: self.frame,
            pending: self.pending,
            facing: self.facing,
            live: self.live.clone(),
        }
    }
}
impl Drop for Provided {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Debug for Provided {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provided")
            .field("state", &self.state)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}
impl Provided {
    pub(super) fn new(
        provider: Arc<dyn BoundParticleProvider>,
        slot: u32,
        frame: u32,
        live: &Arc<AtomicUsize>,
    ) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Self {
            provider,
            slot,
            state: [0.0; 4],
            frame,
            pending: false,
            facing: Facing::Camera,
            live: live.clone(),
        }
    }
    pub(super) fn extract(
        &self,
        particle: &Particle,
        view: &mod_api::ParticleExtractView,
        output: &mut dyn mod_api::QuadParticleRenderState,
    ) -> Result<(), String> {
        self.provider
            .extract(&particle.provider_view(self), view, output)
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
        let facing = match self.facing {
            Facing::Camera => Quat {
                x: camera.x,
                y: camera.y,
                z: camera.z,
                w: camera.w,
            },
            Facing::AroundY => Quat {
                x: 0.0,
                y: camera.y,
                z: 0.0,
                w: camera.w,
            },
            Facing::Fixed(q) => Quat {
                x: q.x,
                y: q.y,
                z: q.z,
                w: q.w,
            },
        };
        let turned = if roll == 0.0 {
            facing
        } else {
            facing.rotate_z(o_roll + partial * (roll - o_roll))
        };
        glam::Quat::from_xyzw(turned.x, turned.y, turned.z, turned.w)
    }
    /// The engine's part of the tick: age, `previous` and expiry. A particle that survives it
    /// joins this tick's batch.
    pub(super) fn age(&mut self, particle: &mut Particle) {
        self.pending = particle.advance_age();
    }
}

/// The batches of one tick, kept between ticks so native dispatch allocates nothing once warm.
/// One pass over the particles sorts them into a bucket per provider (by the dense slot the
/// provider was given when it registered); each provider then runs once on its bucket.
#[derive(Default)]
pub(super) struct Batch {
    /// The particle indices of each provider slot's batch.
    buckets: Vec<Vec<usize>>,
    /// The slots whose bucket holds a particle this pass, in the order they were first met.
    active: Vec<usize>,
    views: Vec<ModParticle>,
}
impl Batch {
    /// Sorts the particles the engine aged this tick into a bucket per provider slot. Whether
    /// any provider has a particle to tick.
    pub(super) fn gather(&mut self, particles: &[Particle]) -> bool {
        for slot in self.active.drain(..) {
            self.buckets[slot].clear();
        }
        for (i, p) in particles.iter().enumerate() {
            if let Some(provided) = p.provided.as_ref().filter(|q| q.pending) {
                let slot = provided.slot as usize;
                if self.buckets.len() <= slot {
                    self.buckets.resize_with(slot + 1, Vec::new);
                }
                if self.buckets[slot].is_empty() {
                    self.active.push(slot);
                }
                self.buckets[slot].push(i);
            }
        }
        !self.active.is_empty()
    }
    /// Calls each provider once with the particles [`Batch::gather`] found for it, then takes the
    /// results back: a particle that leaves a number past the world's limit or not finite
    /// expires.
    pub(super) fn run(
        &mut self,
        particles: &mut [Particle],
        level: &dyn ParticleLevelView,
        mut set_frame: impl FnMut(&mut Particle, usize),
    ) {
        for &slot in &self.active {
            let bucket = &self.buckets[slot];
            let provider = particles[bucket[0]]
                .provided
                .as_ref()
                .map(|provided| provided.provider.clone())
                .expect("a bucket holds provider particles");
            self.views.clear();
            for &i in bucket {
                let p = &particles[i];
                let provided = p
                    .provided
                    .as_ref()
                    .expect("a bucket holds provider particles");
                self.views.push(p.provider_view(provided));
            }
            let mut batch =
                mod_api::__private::particle_batch_from_vec(std::mem::take(&mut self.views));
            provider.tick(&mut batch, level);
            self.views = mod_api::__private::particle_batch_into_vec(batch);
            if self.views.len() != bucket.len() {
                // A guest handed back a batch of another size: nothing of it can be matched to a particle.
                for &i in bucket {
                    particles[i].remove();
                }
                continue;
            }
            for (view, &i) in self.views.iter().zip(bucket) {
                if let Some(frame) = particles[i].take(view) {
                    set_frame(&mut particles[i], frame);
                }
            }
        }
    }
}
impl Particle {
    fn provider_view(&self, provided: &Provided) -> ModParticle {
        ModParticle {
            position: self.position,
            previous: self.previous,
            velocity: self.velocity,
            age: self.age,
            lifetime: self.lifetime,
            size: self.size,
            rgba: self.rgba,
            frame: provided.frame,
            removed: false,
            state: provided.state,
            facing: self.facing(),
            roll: self.roll,
        }
    }
    /// The facing the provider sees.
    fn facing(&self) -> ParticleFacing {
        match self.provided.as_ref().map(|q| q.facing) {
            Some(Facing::Fixed(q)) => ParticleFacing::Fixed(q.to_array()),
            Some(Facing::AroundY) => ParticleFacing::AroundY,
            _ => ParticleFacing::Camera,
        }
    }
    /// Apply native output after validating that the renderer can use its numbers.
    fn take(&mut self, view: &ModParticle) -> Option<usize> {
        let finite = |v: &[f64; 3]| v.iter().all(|c| c.abs() <= LIMIT);
        let facing = match view.facing {
            ParticleFacing::Fixed(q) => {
                let q = glam::Quat::from_array(q);
                let length_squared = q.length_squared();
                (length_squared.is_finite() && length_squared > 0.0)
                    .then(|| Facing::Fixed(q.normalize()))
            }
            ParticleFacing::AroundY => Some(Facing::AroundY),
            ParticleFacing::Camera => Some(Facing::Camera),
        };
        let provided = self.provided.as_mut()?;
        provided.pending = false;
        let Some(facing) = facing else {
            self.remove();
            return None;
        };
        if view.removed
            || !finite(&view.position)
            || !finite(&view.previous)
            || !finite(&view.velocity)
            || !view.size.is_finite()
            || !view.roll.is_finite()
            || view.roll.abs() > ROLL_LIMIT
            || !view.rgba.iter().all(|c| c.is_finite())
        {
            self.remove();
            return None;
        }
        self.position = view.position;
        self.previous = view.previous;
        self.velocity = view.velocity;
        self.lifetime = view.lifetime;
        // SingleQuadParticle's scale and colour setters do not apply the pack's ranges.
        self.size = view.size;
        self.rgba = view.rgba;
        self.roll = view.roll;
        provided.facing = facing;
        provided.state = view.state;
        let changed = view.frame != provided.frame;
        provided.frame = view.frame;
        changed.then_some(view.frame as usize)
    }
}

/// `ParticleLevelView` over the engine's level: the loaded chunks' states and stored light.
pub(super) struct ProviderLevel<'a, B> {
    pub(super) world: &'a super::World,
    pub(super) block: B,
    /// Every engine state as a mod reads it, by state id.
    pub(super) states: &'a [BlockState],
}
impl<B: Fn(i32, i32, i32) -> sunrise_world::registry::BlockStateId> ParticleLevelView
    for ProviderLevel<'_, B>
{
    fn block_state(&self, [x, y, z]: [i32; 3]) -> BlockState {
        // The table holds every state the world can: an id outside it is the game's bug.
        self.states[(self.block)(x, y, z).0 as usize]
    }
    fn brightness(&self, layer: LightLayer, [x, y, z]: [i32; 3]) -> u8 {
        let world = self.world;
        match layer {
            LightLayer::Block => world.light(x, y, z) & 15,
            // A dimension without skylight has no sky layer to read.
            LightLayer::Sky if !world.shape.sky_light => 0,
            LightLayer::Sky => world.light(x, y, z) >> 4,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::*;
    use super::ROLL_LIMIT;
    use crate::chunk::{Chunk, Light};
    use mod_api::__private::bind_particle_provider;
    use mod_api::{
        BlockState, LightLayer, ModParticle, ParticleBatch, ParticleFacing, ParticleLevelView,
        ParticleProvider,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };
    use {
        sunrise_core::nbt::Tag,
        sunrise_world::{
            DATA_VERSION,
            collision::CachedCollisionRegistry,
            registry::{BlockRegistry, BlockStateId},
        },
    };
    #[derive(Clone, Debug)]
    struct Moving(u32);
    impl ParticleController for Moving {
        fn tick(&mut self, p: &mut Particle, _: &ParticleLevel<'_>) -> ParticleUpdate {
            if p.advance_age() {
                self.0 += 1;
                p.position[0] += f64::from(self.0);
            }
            ParticleUpdate {
                frame: Some(self.0 as usize),
                ..Default::default()
            }
        }
    }
    /// An empty world with no blocks, which a tick reads nothing of.
    pub(in crate::particles) struct Scene {
        world: World,
        facts: BlockFacts,
        collision: BlockCollisionRegistry<'static>,
    }
    impl Scene {
        pub(in crate::particles) fn new() -> Self {
            // Leaked: the collision registry borrows it for as long as the scene lives.
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_nbt(&Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
                ]))
                .unwrap(),
            ));
            let cached = CachedCollisionRegistry::from_nbt(
                &Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    (
                        "states",
                        Tag::List(vec![Tag::String("minecraft:air".into())]),
                    ),
                    ("shapes", Tag::List(vec![])),
                    ("cached_shapes", Tag::IntArray(vec![-1])),
                ]),
                registry,
            )
            .unwrap();
            let collision = BlockCollisionRegistry::new(std::sync::Arc::new(cached)).unwrap();
            let facts = BlockFacts {
                states: vec![],
                air: BlockStateId(0),
                woods: HashMap::new(),
                sign_texts: HashMap::new(),
            };
            Self {
                world: World::new(BlockStateId(0), 0),
                facts,
                collision,
            }
        }
        pub(in crate::particles) fn level(&self) -> ParticleLevel<'_> {
            ParticleLevel {
                world: &self.world,
                facts: &self.facts,
                collision: &self.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            }
        }
    }
    fn engine() -> ParticleEngine {
        let mut e = ParticleEngine::new(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        // The game's table of block states; the providers here read none of them.
        e.set_block_state_source(|| Some(Arc::new(Vec::<BlockState>::new())));
        e.mod_sets.insert(
            "example:orbit".into(),
            ModSet {
                set: 0,
                behaviour: None,
                provider: None,
                group: None,
            },
        );
        e.set_sprites.push(vec![0, 1]);
        e.rects = vec![
            Some(Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 16,
                h: 16
            });
            2
        ];
        e
    }
    #[test]
    fn controlled_particle_keeps_owned_motion_sprite_interpolation_and_expiry() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = e
            .controlled("example:orbit", [0.0; 3], true, Moving(0))
            .unwrap();
        p.lifetime = 2;
        p.rgba[3] = 0.5;
        p.velocity = [99.0; 3]; // no vanilla drift may leak into the custom tick
        let mut copy = p.clone();
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut p, &level);
        e.tick_particle(&mut copy, &level);
        assert_eq!((p.position, p.previous), ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(copy.position, [1.0, 0.0, 0.0]); // controller state cloned independently
        assert!(matches!(p.sprite, Sprite::Index(1))); // clamped frame
        p.size = 0.1;
        e.particles.push(p.clone());
        let mut vertices = Vec::new();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert_eq!(vertices.len(), 4);
        assert!(vertices.iter().all(
            |v| v.color >> 24 == 127 && v.layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0
        ));
        e.particles[0].set_visible(false);
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        assert_eq!(e.particles[0].age, 2);
        e.push(copy);
        e.reload(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(e.pending.is_empty());
        vertices.clear();
        e.render(
            DVec3::ZERO,
            glam::Quat::IDENTITY,
            0.5,
            &|_| 0,
            &mut vertices,
        );
        assert!(vertices.is_empty());
        e.tick_particle(&mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [3.0, 0.0, 0.0]);
    }
    /// Counts its ticks, so a test sees which calls a particle's expiry skipped.
    #[derive(Default)]
    struct Probe(std::sync::atomic::AtomicUsize);
    impl ParticleProvider for Probe {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                self.one(particle);
            }
        }
    }
    impl Probe {
        fn one(&self, particle: &mut ModParticle) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            particle.position[0] = f64::from(particle.age) * 2.0;
            particle.position[1] = particle.state[0];
            particle.state[0] += 0.5;
            particle.frame = particle.age as u32;
            if particle.age == 3 {
                particle.removed = true;
                particle.position[0] = 999.0;
            }
        }
    }
    struct Broken(fn(&mut ModParticle));
    impl ParticleProvider for Broken {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            for particle in particles {
                self.0(particle);
            }
        }
    }
    /// One engine tick of a single provider particle: the age step, then the provider's batch.
    fn tick_one(e: &mut ParticleEngine, p: &mut Particle, level: &ParticleLevel<'_>) {
        e.tick_particle(p, level);
        e.tick_providers(std::slice::from_mut(p), level);
    }
    /// Counts its calls and the particles each held.
    #[derive(Default)]
    struct Calls(std::sync::Mutex<Vec<usize>>);
    impl ParticleProvider for Calls {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            self.0.lock().unwrap().push(particles.len());
        }
    }
    #[test]
    fn a_provider_runs_once_per_tick_on_all_its_particles() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let calls = Arc::new(Calls::default());
        let mut all: Vec<Particle> = (0..3).map(|_| provided(&mut e, calls.clone())).collect();
        for p in &mut all {
            p.lifetime = 20;
            e.tick_particle(p, &level);
        }
        e.tick_providers(&mut all, &level);
        assert_eq!(*calls.0.lock().unwrap(), [3]);
        // Nothing aged since: no particle is pending, so no call.
        e.tick_providers(&mut all, &level);
        assert_eq!(calls.0.lock().unwrap().len(), 1);
    }
    fn provided(e: &mut ParticleEngine, provider: Arc<dyn ParticleProvider>) -> Particle {
        let [ty] = unit_types(["fixture"]);
        let set = e.mod_sets.get_mut("example:orbit").unwrap();
        set.provider = Some((0, bind_particle_provider(ty.key(), provider)));
        e.make_sprite_particle(&e.mod_sets["example:orbit"], [1.0; 3], [0.0; 3], 5)
    }

    struct Extract<F>(F);
    impl<F> ParticleProvider for Extract<F>
    where
        F: Fn(
                &ModParticle,
                &mod_api::ParticleExtractView,
                &mut dyn mod_api::QuadParticleRenderState,
            ) + Send
            + Sync
            + 'static,
    {
        fn tick(&self, _: &mut ParticleBatch, _: &dyn ParticleLevelView) {}
        fn extract(
            &self,
            particle: &ModParticle,
            view: &mod_api::ParticleExtractView,
            output: &mut dyn mod_api::QuadParticleRenderState,
        ) {
            self.0(particle, view, output);
        }
    }

    #[test]
    fn provider_extracts_independent_lit_opaque_quads_in_order() {
        use mod_api::{ParticleBlend, ParticleExtractView, ParticleQuad, QuadParticleRenderState};
        let mut engine = engine();
        engine.rects[1] = Some(Rect {
            u0: 0.25,
            v0: 0.5,
            u1: 0.5,
            v1: 0.75,
            w: 16,
            h: 16,
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(Extract({
            let calls = calls.clone();
            move |p: &ModParticle,
                  view: &ParticleExtractView,
                  out: &mut dyn QuadParticleRenderState| {
                calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(
                    (p.previous, p.position),
                    ([10.0, 20.0, 25.0], [14.0, 20.0, 25.0])
                );
                assert_eq!(view.camera, [10.0, 20.0, 30.0]);
                assert_eq!(view.rotation, [0.0, 1.0, 0.0, 0.0]);
                assert_eq!(view.previous_roll, 0.25);
                assert_eq!(view.partial_tick, 0.5);
                assert_eq!(view.light_coords, 0xb0_0040);
                let x = (p.previous[0]
                    + (p.position[0] - p.previous[0]) * f64::from(view.partial_tick)
                    - view.camera[0]) as f32;
                for frame in 0..1025 {
                    let mut quad = ParticleQuad::default();
                    quad.layer = ParticleBlend::Opaque;
                    quad.center = if frame == 0 {
                        [x, 0.0, -5.0]
                    } else {
                        [4.0, 1.0, -8.0]
                    };
                    if frame != 0 {
                        quad.rotation = [0.0, 0.0, 2.0, 0.0];
                    }
                    quad.half_size = if frame == 0 { 0.5 } else { 2.0 };
                    quad.frame = u32::from(frame != 0);
                    quad.uv = [0.25, 0.0, 0.75, 0.5];
                    quad.color = 0xc9_12_45_67;
                    quad.light_coords = 0x73_0081;
                    out.add(quad);
                }
            }
        }));
        let mut particle = provided(&mut engine, provider);
        particle.previous = [10.0, 20.0, 25.0];
        particle.position = [14.0, 20.0, 25.0];
        particle.o_roll = 0.25;
        particle.rgba[3] = 0.5;
        engine.particles.push(particle);
        let camera = DVec3::new(10.0, 20.0, 30.0);
        let cull = ParticleCullView::new(camera, [180.0, 0.0], [512, 512], 70.0, 70.0, 128.0);
        let mut explicit = ParticleQuads::default();
        engine.render_world(
            camera,
            Quat::from_xyzw(0.0, 1.0, 0.0, 0.0),
            0.5,
            Some((&cull, &mut explicit)),
            None,
            &|_| 0xb4,
            &mut Vec::new(),
        );
        let vertices = explicit.opaque;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(vertices.len(), 1025 * 4);
        assert_eq!(
            vertices[0..4].iter().map(|v| v.pos).collect::<Vec<_>>(),
            [
                [2.5, -0.5, -5.0],
                [2.5, 0.5, -5.0],
                [1.5, 0.5, -5.0],
                [1.5, -0.5, -5.0]
            ]
        );
        assert_eq!(
            vertices[4..8].iter().map(|v| v.pos).collect::<Vec<_>>(),
            [
                [2.0, 3.0, -8.0],
                [2.0, -1.0, -8.0],
                [6.0, -1.0, -8.0],
                [6.0, 3.0, -8.0]
            ]
        );
        assert_eq!(vertices[0].uv, [0.75, 0.5]);
        assert_eq!(vertices[4].uv, [0.4375, 0.625]);
        assert_eq!(vertices[6].uv, [0.3125, 0.5]);
        assert!(vertices.iter().all(|v| (v.layer, v.color, v.light)
            == (crate::gfx::EXPLICIT_PARTICLE_OVERLAY, 0xc9_67_45_12, 0x8173)));
    }

    #[test]
    fn provider_extracts_live_points_in_the_wider_cull_view_with_both_settings() {
        use crate::gfx::{Frustum, JoinedFrusta, world_projection};
        use mod_api::{ParticleExtractView, ParticleQuad, QuadParticleRenderState};
        let mut engine = engine();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(Extract({
            let calls = calls.clone();
            move |p: &ModParticle,
                  _: &ParticleExtractView,
                  out: &mut dyn QuadParticleRenderState| {
                calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(p.position, [3.0, 0.0, -10.0]);
                out.add_default();
                let mut quad = ParticleQuad::default();
                quad.center = [0.0, 0.0, -10.0];
                quad.half_size = 0.5;
                out.add(quad);
            }
        }));
        for (position, removed) in [
            ([3.0, 0.0, -10.0], false), // Outside the 10° picture but inside the 70° cull view.
            ([50.0, 0.0, -10.0], false),
            ([0.0, 0.0, 10.0], false),
            ([3.0, 0.0, -10.0], true),
        ] {
            let mut p = provided(&mut engine, provider.clone());
            p.previous = [0.0, 0.0, -10.0]; // This does not decide whether extract runs.
            p.position = position;
            p.size = 0.0;
            p.removed = removed;
            engine.particles.push(p);
        }
        let cull = ParticleCullView::new(DVec3::ZERO, [180.0, 0.0], [512, 512], 70.0, 10.0, 512.0);
        let narrow = JoinedFrusta::from(Frustum::from_view_projection(world_projection(
            10.0, 1.0, 512.0,
        )));
        assert!(!narrow.contains_point(FVec3::new(3.0, 0.0, -10.0)));
        let mut not_occluded = |_, _| panic!("subpixel default is rejected before terrain culling");
        for stock in [
            None,
            Some(ParticleWorldCull {
                pixel_focal_length: 10.0,
                frustum: Some(&narrow),
                occluded: &mut not_occluded,
            }),
        ] {
            let mut explicit = ParticleQuads::default();
            let mut vertices = Vec::new();
            let stock_expected = if stock.is_none() { 4 } else { 0 };
            engine.render_world(
                DVec3::ZERO,
                Quat::IDENTITY,
                0.5,
                Some((&cull, &mut explicit)),
                stock,
                &|_| 0,
                &mut vertices,
            );
            assert_eq!(vertices.len(), stock_expected);
            assert_eq!(explicit.translucent.len(), 4);
            assert_eq!(explicit.translucent[0].pos, [0.5, -0.5, -10.0]);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_failed_extract_keeps_the_next_providers_quad_and_scene_draw_is_stock() {
        use mod_api::{ParticleExtractView, ParticleQuad, QuadParticleRenderState};
        let mut engine = engine();
        let fail = Arc::new(Extract(
            |_: &ModParticle, _: &ParticleExtractView, _: &mut dyn QuadParticleRenderState| {
                panic!("provider cannot finish extracting");
            },
        ));
        let healthy = Arc::new(Extract(
            |_: &ModParticle, _: &ParticleExtractView, output: &mut dyn QuadParticleRenderState| {
                let mut quad = ParticleQuad::default();
                quad.center = [0.0, 0.0, 10.0];
                quad.half_size = 1.0;
                output.add(quad);
            },
        ));
        let first = provided(&mut engine, fail);
        let second = provided(&mut engine, healthy);
        engine.particles.extend([first, second]);
        let mut scene = Vec::new();
        engine.render(DVec3::ZERO, Quat::IDENTITY, 0.0, &|_| 0, &mut scene);
        assert_eq!(
            scene.len(),
            8,
            "the regular scene render uses the two stock quads"
        );
        let cull = ParticleCullView::new(DVec3::ZERO, [0.0, 0.0], [512, 512], 70.0, 70.0, 128.0);
        let mut quads = ParticleQuads::default();
        let mut stock = Vec::new();
        engine.render_world(
            DVec3::ZERO,
            Quat::IDENTITY,
            0.0,
            Some((&cull, &mut quads)),
            None,
            &|_| 0,
            &mut stock,
        );
        assert!(stock.is_empty());
        assert_eq!(quads.translucent.len(), 4);
        assert_eq!(quads.translucent[0].pos, [1.0, -1.0, 10.0]);
    }

    #[test]
    fn bad_provider_quads_do_not_append_or_remove_neighboring_vertices() {
        use mod_api::{ParticleBlend, ParticleExtractView, ParticleQuad, QuadParticleRenderState};
        let mut engine = engine();
        let provider = Arc::new(Extract(
            |_: &ModParticle, _: &ParticleExtractView, output: &mut dyn QuadParticleRenderState| {
                for (frame, x) in [(0, 1.0), (5, 2.0), (1, f32::INFINITY), (1, 3.0)] {
                    let mut quad = ParticleQuad::default();
                    quad.layer = ParticleBlend::Opaque;
                    quad.center = [x, 0.0, 0.0];
                    quad.half_size = 1.0;
                    quad.frame = frame;
                    output.add(quad);
                }
            },
        ));
        let particle = provided(&mut engine, provider);
        engine.particles.push(particle);
        let cull = ParticleCullView::new(DVec3::ZERO, [0.0, 0.0], [512, 512], 70.0, 70.0, 128.0);
        let mut quads = ParticleQuads::default();
        engine.render_world(
            DVec3::ZERO,
            Quat::IDENTITY,
            0.5,
            Some((&cull, &mut quads)),
            None,
            &|_| 0,
            &mut Vec::new(),
        );
        let vertices = quads.opaque;
        assert_eq!(vertices.len(), 8);
        assert_eq!(
            (vertices[0].pos, vertices[4].pos),
            ([2.0, -1.0, 0.0], [4.0, -1.0, 0.0])
        );
    }

    fn registered_types<T: 'static, const N: usize>(
        paths: [&'static str; N],
        register: fn(&mut dyn mod_api::Registrar, &str) -> mod_api::ParticleType<T>,
    ) -> [mod_api::ParticleType<T>; N] {
        type Types<T, const N: usize> = Arc<Mutex<Option<[mod_api::ParticleType<T>; N]>>>;
        struct Register<T, const N: usize> {
            paths: [&'static str; N],
            types: Types<T, N>,
            register: fn(&mut dyn mod_api::Registrar, &str) -> mod_api::ParticleType<T>,
        }
        impl<T: 'static, const N: usize> mod_api::Hooks for Register<T, N> {
            fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
                &[]
            }
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                *self.types.lock().unwrap() =
                    Some(self.paths.map(|path| (self.register)(registrar, path)));
            }
        }
        let types: Types<T, N> = Default::default();
        let entries = [mod_api::ModEntry::new(
            "mymod",
            Box::new(Register {
                paths,
                types: types.clone(),
                register,
            }),
        )];
        mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
        mod_api::__private::resolve_particles([], |_| {
            Some(mod_api::__private::ParticleKind::Custom)
        })
        .unwrap();
        types.lock().unwrap().take().unwrap()
    }

    pub(in crate::particles) fn unit_types<const N: usize>(
        paths: [&'static str; N],
    ) -> [mod_api::ParticleType<()>; N] {
        registered_types(paths, |registrar, path| {
            registrar.register_particle_type(
                path,
                false,
                mod_api::ParticleMapCodec::unit(),
                mod_api::ParticleStreamCodec::wire(),
            )
        })
    }

    #[test]
    fn typed_factory_initializes_each_admitted_request_before_tick_and_can_decline() {
        let [ty] = registered_types(["spark"], |registrar, path| {
            registrar.register_particle_type(
                path,
                true,
                mod_api::ParticleMapCodec::new(
                    |fields, _| match fields.get(&"frame".into()) {
                        Some(mod_api::Tag::Int(frame)) => {
                            u32::try_from(*frame).map_err(|err| err.to_string())
                        }
                        _ => Err("missing frame".into()),
                    },
                    |value, _| {
                        Ok(mod_api::Compound::from([(
                            "frame".into(),
                            mod_api::Tag::Int(*value as i32),
                        )]))
                    },
                ),
                mod_api::ParticleStreamCodec::wire(),
            )
        });
        struct Factory(Arc<Mutex<Vec<u32>>>);
        impl ParticleProvider<u32> for Factory {
            fn create_particle(
                &self,
                value: &u32,
                level: &dyn ParticleLevelView,
                at: [f64; 3],
                aux: [f64; 3],
                rng: &mut dyn mod_api::RandomSource,
                particle: &mut ModParticle,
            ) -> Option<()> {
                assert_eq!(at, [1.0, 64.0, 3.0]);
                assert_eq!(aux, [0.125, 0.25, 0.75]);
                assert_eq!(level.brightness(LightLayer::Block, [1, 64, 3]), 4);
                self.0.lock().unwrap().push(*value);
                let tint = rng.next_float();
                particle.rgba = [tint, -0.25, 2.0, 1.5];
                particle.size = 20.0;
                particle.frame = *value;
                particle.state[0] = f64::from(*value);
                particle.velocity = aux;
                (*value != 2).then_some(())
            }
            fn tick(&self, _: &mut ParticleBatch, _: &dyn ParticleLevelView) {}
        }
        let mut scene = Scene::new();
        scene.world.chunks.insert((0, 0), Arc::new(lit_chunk()));
        let mut e = engine();
        e.camera = [0.0; 3];
        e.particle_level = 2;
        let calls = Arc::new(Mutex::new(Vec::new()));
        e.typed_sets.insert(
            ty.key(),
            ModSet {
                set: 0,
                behaviour: None,
                provider: Some((
                    0,
                    bind_particle_provider(ty.key(), Arc::new(Factory(calls.clone()))),
                )),
                group: None,
            },
        );
        e.seed_randoms(37, 0, 0);
        let mut expected_provider_rng = LegacyRandom::new(37);
        let first = expected_provider_rng.next_float();
        expected_provider_rng.next_float();
        let value = ty.options(1);
        let options = ParticleOptions::typed(value.typed().unwrap().clone()).unwrap();
        e.add_always_visible_with_world(
            &scene.world,
            &options,
            [1.0, 64.0, 3.0],
            [0.125, 0.25, 0.75],
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "no-flag overload keeps the distance gate"
        );
        e.camera = [1.0, 64.0, 3.0];
        e.level_random = LegacyRandom::new(1);
        e.add_always_visible_with_world(
            &scene.world,
            &options,
            [1.0, 64.0, 3.0],
            [0.125, 0.25, 0.75],
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "no-flag overload keeps the setting gate"
        );
        for value in [1, 2] {
            let value = ty.options(value);
            let options = ParticleOptions::typed(value.typed().unwrap().clone()).unwrap();
            e.add_with_world(
                Some(&scene.world),
                &options,
                [1.0, 64.0, 3.0],
                [0.125, 0.25, 0.75],
                false,
                false,
            );
        }
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        assert_eq!(e.pending.len(), 1);
        let particle = e.pending[0].ordinary().unwrap();
        assert_eq!(particle.age, 0);
        assert_eq!(particle.velocity, [0.125, 0.25, 0.75]);
        assert_eq!(particle.rgba, [first, -0.25, 2.0, 1.5]);
        assert_eq!(particle.size, 20.0);
        assert!(matches!(particle.sprite, Sprite::Index(1)));
        assert_eq!(particle.provided.as_ref().unwrap().state[0], 1.0);
        assert_eq!(e.random.next_long(), expected_provider_rng.next_long());

        let level = scene.level();
        e.tick(&level); // Admit the pending particle.
        e.tick(&level); // The provider's no-op batched tick.
        let particle = e.live().next().unwrap();
        assert_eq!(particle.age, 1);
        assert_eq!(
            (particle.size, particle.rgba),
            (20.0, [first, -0.25, 2.0, 1.5])
        );
    }

    #[test]
    fn packet_spread_is_independent_of_factory_rng_and_a_failed_callback_ends_only_its_packet() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_protocol::{
            entity_data::Particle as NetworkParticle, packets::play::LevelParticles,
        };

        struct Factory {
            calls: Arc<AtomicUsize>,
            panic: bool,
        }
        impl ParticleProvider<()> for Factory {
            fn create_particle(
                &self,
                _: &(),
                _: &dyn ParticleLevelView,
                _: [f64; 3],
                aux: [f64; 3],
                rng: &mut dyn mod_api::RandomSource,
                particle: &mut ModParticle,
            ) -> Option<()> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                assert!(!self.panic, "test factory failed");
                particle.rgba[0] = rng.next_float();
                particle.velocity = aux;
                Some(())
            }
            fn tick(&self, _: &mut ParticleBatch, _: &dyn ParticleLevelView) {}
        }
        let [broken, healthy] = unit_types(["broken", "healthy"]);
        let mut e = engine();
        let broken_calls = Arc::new(AtomicUsize::new(0));
        let healthy_calls = Arc::new(AtomicUsize::new(0));
        for (ty, panic, calls) in [
            (broken, true, &broken_calls),
            (healthy, false, &healthy_calls),
        ] {
            e.typed_sets.insert(
                ty.key(),
                ModSet {
                    set: 0,
                    behaviour: None,
                    provider: Some((
                        0,
                        bind_particle_provider(
                            ty.key(),
                            Arc::new(Factory {
                                calls: calls.clone(),
                                panic,
                            }),
                        ),
                    )),
                    group: None,
                },
            );
        }
        let scene = Scene::new();
        let mut random = LegacyRandom::new(19);
        let packet = LevelParticles {
            // The loop receives the already-decoded options from the packet handler.
            particle: NetworkParticle { id: 0, data: None },
            override_limiter: true,
            always_show: false,
            pos: [1.0, 2.0, 3.0],
            dist: [0.25, 0.5, 1.0],
            max_speed: [0.125, 0.25, 0.5],
            count: 3,
            randomization: 2,
        };
        let options = |ty: mod_api::ParticleType<()>| {
            ParticleOptions::typed(ty.options(()).typed().unwrap().clone()).unwrap()
        };
        let mut reference = LegacyRandom::new(19);
        e.handle_level_particles(&mut random, &scene.world, &packet, &options(broken));
        assert_eq!(broken_calls.load(Ordering::Relaxed), 1);
        assert!(e.pending.is_empty());
        for _ in 0..6 {
            reference.next_double();
        }
        assert_eq!(
            random.next_long(),
            reference.next_long(),
            "the failed packet drew only one spawn"
        );
        e.seed_randoms(37, 0, 0);
        let mut provider_random = LegacyRandom::new(37);
        e.handle_level_particles(&mut random, &scene.world, &packet, &options(healthy));
        assert_eq!(healthy_calls.load(Ordering::Relaxed), 3);
        assert_eq!(e.pending.len(), 3);
        for p in &e.pending {
            let p = p.ordinary().unwrap();
            let at = std::array::from_fn(|i| {
                packet.pos[i] + reference.next_double() * f64::from(packet.dist[i])
            });
            let velocity = packet
                .max_speed
                .map(|max| reference.next_double() * f64::from(max));
            assert_eq!(
                (p.position, p.velocity, p.rgba[0]),
                (at, velocity, provider_random.next_float())
            );
        }
        assert_eq!(random.next_long(), reference.next_long());
    }

    #[test]
    fn a_providers_particle_moves_changes_frame_and_expires_by_its_tick() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 20;
        let mut copy = p.clone();
        tick_one(&mut e, &mut p, &level);
        assert_eq!((p.position, p.previous), ([2.0, 0.0, 1.0], [1.0; 3]));
        tick_one(&mut e, &mut p, &level);
        assert_eq!((p.position, p.previous), ([4.0, 0.5, 1.0], [2.0, 0.0, 1.0]));
        assert!(matches!(p.sprite, Sprite::Index(1))); // frame 2, clamped to the 2-sprite set
        // State is per particle: the copy starts from zero state, not the original's.
        tick_one(&mut e, &mut copy, &level);
        assert_eq!(copy.position, [2.0, 0.0, 1.0]);
        // The third tick of `p` removes it before its 20-tick lifetime, and its writes are dropped.
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, [4.0, 0.5, 1.0]);
    }
    #[test]
    fn the_engine_expires_a_providers_particle_at_its_lifetime_without_calling_the_provider() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 2;
        for _ in 0..2 {
            tick_one(&mut e, &mut p, &level);
            assert!(!p.removed);
        }
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
    #[test]
    fn a_provider_cannot_hand_the_renderer_a_bad_number() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let bad: [fn(&mut ModParticle); 5] = [
            |p| p.position[1] = f64::NAN,
            |p| p.position[0] = f64::INFINITY,
            |p| p.position[2] = 1.0e300,
            |p| p.velocity[0] = f64::NEG_INFINITY,
            |p| p.size = f32::NAN,
        ];
        for tick in bad {
            let mut p = provided(&mut e, Arc::new(Broken(tick)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed);
            assert_eq!(p.position, [1.0; 3]);
        }
        let mut p = provided(
            &mut e,
            Arc::new(Broken(|p| {
                p.size = 1.0e9;
                p.rgba = [2.0, -1.0, 0.5, 9.0];
                p.frame = u32::MAX;
                p.lifetime = i32::MIN;
                p.age = i32::MAX;
            })),
        );
        tick_one(&mut e, &mut p, &level);
        assert!(!p.removed);
        assert_eq!((p.size, p.rgba), (1.0e9, [2.0, -1.0, 0.5, 9.0]));
        assert!(matches!(p.sprite, Sprite::Index(1)));
        assert_eq!((p.lifetime, p.age), (i32::MIN, 1)); // the age stays the engine's
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
    }
    /// Hands back a batch of another size, as only a guest's raw bytes could.
    struct Replacing;
    impl ParticleProvider for Replacing {
        fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
            *particles = mod_api::__private::particle_batch_from_vec(Vec::new());
        }
    }
    #[test]
    fn a_batch_handed_back_at_another_size_expires_its_particles() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Replacing));
        p.lifetime = 20;
        let at = p.position;
        tick_one(&mut e, &mut p, &level);
        assert!(p.removed);
        assert_eq!(p.position, at);
    }
    /// Reads the batch's two nonfinite-length quaternions.
    #[test]
    fn a_quaternion_with_an_infinite_squared_length_expires_the_particle() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let facings = [
            ParticleFacing::Fixed([1.0e30, 0.0, 0.0, 0.0]),
            ParticleFacing::Fixed([f32::NAN, 0.0, 0.0, 1.0]),
            ParticleFacing::Fixed([0.0; 4]),
        ];
        for facing in facings {
            struct Set(ParticleFacing);
            impl ParticleProvider for Set {
                fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                    for particle in particles {
                        particle.facing = self.0;
                    }
                }
            }
            let mut p = provided(&mut e, Arc::new(Set(facing)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed, "{facing:?}");
        }
    }
    /// A particle a provider poses on each of its ticks (`rolls[age - 1]`), drawn half a tick
    /// into the last one with the camera turned by `base`, as the renderer builds its corners.
    fn drawn(
        facing: ParticleFacing,
        rolls: &[f32],
        center: [f64; 3],
        base: glam::Quat,
    ) -> Vec<[f32; 3]> {
        struct Pose(ParticleFacing, Vec<f32>, [f64; 3]);
        impl ParticleProvider for Pose {
            fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                for particle in particles {
                    particle.position = self.2;
                    particle.previous = self.2;
                    particle.size = 1.0;
                    particle.facing = self.0;
                    particle.roll = self.1[particle.age as usize - 1];
                }
            }
        }
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Pose(facing, rolls.to_vec(), center)));
        p.lifetime = 20;
        for _ in rolls {
            tick_one(&mut e, &mut p, &level);
        }
        assert!(!p.removed);
        e.particles.push(p);
        let mut vertices = Vec::new();
        e.render(DVec3::ZERO, base, 0.5, &|_| 0, &mut vertices);
        assert_eq!(vertices.len(), 4);
        vertices.iter().map(|v| v.pos).collect()
    }
    // The expected corners below come from running JOML 1.10.9 itself: `Quaternionf.rotateZ`
    // of `Mth.lerp(0.5, oRoll, roll)` on the facing's quaternion, then `transform` of the quad's
    // corners (1,-1), (1,1), (-1,1), (-1,-1), added to the centre.
    #[test]
    fn a_providers_fixed_facing_and_roll_turn_the_drawn_quad() {
        // A unit quaternion, rolled from 0 (the first tick's) to 1: half a tick in, 0.5 radians.
        let corners = drawn(
            ParticleFacing::Fixed([-0.5, -0.5, -0.5, 0.5]),
            &[1.0],
            [0.0; 3],
            glam::Quat::IDENTITY,
        );
        assert_eq!(
            corners,
            [
                [-0.39815706, 0.0, 1.3570082],
                [1.3570082, 0.0, 0.39815706],
                [0.39815706, 0.0, -1.3570082],
                [-1.3570082, 0.0, -0.39815706],
            ]
        );
    }
    #[test]
    fn a_providers_camera_facing_rotates_the_quad_by_the_camera_then_the_interpolated_roll() {
        let corners = drawn(
            ParticleFacing::Camera,
            &[4.1, 4.5],
            [3.25, -1.5, 2.75],
            glam::Quat::from_xyzw(0.5, -0.5, 0.5, 0.5),
        );
        assert_eq!(
            corners,
            [
                [3.765367, -1.5, 1.4330349],
                [4.566965, -1.5, 3.265367],
                [2.734633, -1.5, 4.066965],
                [1.9330349, -1.5, 2.234633],
            ]
        );
    }
    #[test]
    fn a_providers_around_y_facing_takes_the_cameras_y_and_w_and_rolls_from_the_last_tick() {
        // The camera's pitch and roll components (x and z) do not reach the quad, and the
        // particle is off the camera's axes, so a yaw taken from its direction would differ.
        let camera = glam::Quat::from_xyzw(0.25, 0.5, -0.25, 0.8);
        let corners = drawn(
            ParticleFacing::AroundY,
            &[0.3, 0.9],
            [3.0, 1.0, -4.0],
            camera,
        );
        assert_eq!(
            corners,
            [
                [3.6090915, 0.7393068, -5.2494183],
                [3.1142364, 2.389978, -4.234331],
                [2.3909085, 1.2606932, -2.7505815],
                [2.8857636, -0.38997805, -3.765669],
            ]
        );
    }
    #[test]
    fn a_roll_that_returns_to_zero_draws_unrolled_as_vanilla_does() {
        // `SingleQuadParticle.extract` rolls only when the current roll is not zero.
        let camera = glam::Quat::from_xyzw(0.25, 0.5, -0.25, 0.8);
        let corners = drawn(
            ParticleFacing::AroundY,
            &[1.0, 0.0],
            [3.0, 1.0, -4.0],
            camera,
        );
        assert_eq!(
            corners,
            [
                [3.4382024, 0.0, -4.898876],
                [3.4382024, 2.0, -4.898876],
                [2.5617976, 2.0, -3.1011236],
                [2.5617976, 0.0, -3.1011236],
            ]
        );
    }
    #[test]
    fn rolls_at_the_accepted_limit_interpolate_to_finite_corners() {
        // The difference of the two extremes is `f32::MAX`, still a number.
        let corners = drawn(
            ParticleFacing::Camera,
            &[-ROLL_LIMIT, ROLL_LIMIT],
            [0.0; 3],
            glam::Quat::IDENTITY,
        );
        assert!(
            corners.iter().flatten().all(|c| c.is_finite()),
            "{corners:?}"
        );
    }
    #[test]
    fn a_roll_past_the_limit_expires_the_particle() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        for roll in [f32::MAX, -f32::MAX, ROLL_LIMIT * 1.0000001] {
            struct Set(f32);
            impl ParticleProvider for Set {
                fn tick(&self, particles: &mut ParticleBatch, _: &dyn ParticleLevelView) {
                    for particle in particles {
                        particle.roll = self.0;
                    }
                }
            }
            let mut p = provided(&mut e, Arc::new(Set(roll)));
            tick_one(&mut e, &mut p, &level);
            assert!(p.removed, "{roll}");
        }
    }
    #[test]
    fn a_second_tick_reuses_the_batch_storage() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut all: Vec<Particle> = (0..3).map(|_| provided(&mut e, probe.clone())).collect();
        let tick = |e: &mut ParticleEngine, all: &mut Vec<Particle>| {
            for p in all.iter_mut() {
                p.lifetime = 20;
                e.tick_particle(p, &level);
            }
            e.tick_providers(all, &level);
        };
        tick(&mut e, &mut all);
        let shape = |e: &ParticleEngine| {
            (
                e.batch.views.as_ptr(),
                e.batch.views.capacity(),
                e.batch.buckets.len(),
                e.batch.buckets[0].as_ptr(),
                e.batch.buckets[0].capacity(),
                e.batch.active.capacity(),
            )
        };
        let warm = shape(&e);
        tick(&mut e, &mut all);
        assert_eq!(shape(&e), warm);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 6);
    }
    /// A provider type in a high slot visits only its own bucket, and an empty list visits none.
    #[test]
    fn only_the_slots_with_particles_are_visited() {
        let mut batch = native::Batch::default();
        assert!(!batch.gather(&[]));
        assert!(batch.buckets.is_empty());
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let mut p = provided(&mut e, Arc::new(Probe::default()));
        p.provided.as_mut().unwrap().slot = 40;
        p.lifetime = 20;
        e.tick_particle(&mut p, &level);
        assert!(batch.gather(std::slice::from_ref(&p)));
        assert_eq!(batch.active, [40]);
        assert!(!batch.gather(&[]));
        assert!(batch.active.is_empty());
        assert!(batch.buckets[40].is_empty());
    }
    /// The block state table is asked for only when a provider has a particle to tick.
    #[test]
    fn the_block_state_table_is_not_asked_while_no_provider_particle_is_ready() {
        static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn counting() -> Option<BlockStates> {
            ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(Arc::new(Vec::<BlockState>::new()))
        }
        let asked = || ASKED.load(std::sync::atomic::Ordering::Relaxed);
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        e.set_block_state_source(counting);
        // A provider is registered, and no particle of it lives.
        let [ty] = unit_types(["orbit"]);
        e.providers.insert(
            ty.key(),
            bind_particle_provider(ty.key(), Arc::new(Probe::default())),
        );
        e.tick_providers(&mut [], &level);
        assert_eq!(asked(), 0);
        // One lives and the engine has not aged it.
        let mut p = provided(&mut e, Arc::new(Probe::default()));
        p.lifetime = 20;
        e.tick_providers(std::slice::from_mut(&mut p), &level);
        assert_eq!(asked(), 0);
        // Aged: it is ticked, and the table is asked once.
        tick_one(&mut e, &mut p, &level);
        assert_eq!(asked(), 1);
        // Gone: nothing runs.
        drop(p);
        e.tick_providers(&mut [], &level);
        assert_eq!(asked(), 1);
    }
    #[test]
    fn a_provider_does_not_run_while_the_game_has_no_block_state_table() {
        let scene = Scene::new();
        let level = scene.level();
        let mut e = engine();
        let probe = Arc::new(Probe::default());
        let mut p = provided(&mut e, probe.clone());
        p.lifetime = 20;
        e.set_block_state_source(|| None);
        tick_one(&mut e, &mut p, &level);
        assert_eq!(probe.0.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(p.position, [1.0; 3]);
        assert!(!p.removed);
    }
    /// A chunk whose section holds sky light 11 over block light 4.
    fn lit_chunk() -> Chunk {
        let mut chunk = Chunk::empty(0, 0);
        let section = chunk.section_index(64).unwrap();
        Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(Box::new([0xB4; 4096]));
        chunk
    }
    fn lights(view: &dyn ParticleLevelView, pos: [i32; 3]) -> (u8, u8) {
        (
            view.brightness(LightLayer::Sky, pos),
            view.brightness(LightLayer::Block, pos),
        )
    }
    #[test]
    fn the_providers_light_reads_follow_the_dimensions_sky() {
        for (shape, sky) in [
            (crate::chunk::LevelShape::OVERWORLD, true),
            (crate::chunk::LevelShape::NETHER, false),
        ] {
            let mut scene = Scene::new();
            scene.world.shape = shape;
            scene.world.chunks.insert((0, 0), Arc::new(lit_chunk()));
            let level = scene.level();
            let view = super::ProviderLevel {
                world: level.world,
                block: |x, y, z| level.block(x, y, z),
                states: &[],
            };
            let full = if sky { 15 } else { 0 };
            // The stored nibbles, sky 11 and block 4, but a dimension without skylight has no sky layer.
            assert_eq!(lights(&view, [1, 64, 3]), (if sky { 11 } else { 0 }, 4));
            // No chunk is loaded there: the sky the dimension has, and no block light.
            assert_eq!(lights(&view, [100, 64, 3]), (full, 0));
            // Outside the build height the view uses the render world's light read as well.
            // The world owns padding and retained border light (#2761), not the provider.
            assert_eq!(lights(&view, [1, 10_000, 3]), (full, 0));
        }
    }
    /// The provider's view reads a chunk by numeric state ID, using the game's air if absent.
    #[test]
    fn the_providers_level_view_reads_the_loaded_chunks_states() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
            mod_api::__private::resolve_block_keys(["minecraft:air", "minecraft:stone"], |_| {
                sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&BTreeMap::new()], 0)
                    .map_err(|error| format!("{error:?}"))
            })
            .unwrap();
            let key = |name| mod_api::__private::block_key(name).unwrap();
            let air = key("minecraft:air");
            let stone = key("minecraft:stone");
            let states =
                [air, stone].map(|block| mod_api::__private::block_state_from_ordinal(block, 0));
            let mut scene = Scene::new();
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(1, 64, 3, 1);
            scene.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = scene.level();
            let view = super::ProviderLevel {
                world: level.world,
                block: |x, y, z| level.block(x, y, z),
                states: &states,
            };
            assert_eq!(view.block_state([1, 64, 3]).block(), stone);
            assert_eq!(view.block_state([2, 64, 3]).block(), air);
            assert_eq!(view.block_state([100, 64, 3]).block(), air);
        });
    }
    #[test]
    fn a_type_resolves_to_its_provider_at_load_and_other_types_stay_declarative() {
        let resources = ResourceSource::indexed([
            ("orbit", br#"{"textures":[],"sunrise:behaviour":{"lifetime":7,"size":[0.3,0.3]}}"#.as_slice()),
            ("empty_asset", br#"{"textures":[]}"#.as_slice()),
            ("plain", br#"{"textures":["mymod:one","mymod:two"],"sunrise:behaviour":{"lifetime":7,"size":[0.3,0.3]}}"#.as_slice()),
        ].map(|(name, bytes)| (
            format!("assets/mymod/particles/{name}.json"),
            sunrise_assets::resource_source::Resource::bytes(bytes.to_vec()),
        )).into());
        let [orbit, unmatched] = unit_types(["orbit", "unmatched"]);
        let mut e = engine();
        for ty in [orbit, unmatched] {
            e.providers.insert(
                ty.key(),
                bind_particle_provider(ty.key(), Arc::new(Probe::default())),
            );
        }
        let missing = e.rects[0].unwrap();
        e.reload(
            &resources,
            &[(crate::hud::MISSING_SPRITE.to_owned(), missing)].into(),
            &HashSet::new(),
        );
        assert!(e.typed_sets.contains_key(&orbit.key()));
        assert!(e.mod_sets["mymod:plain"].provider.is_none());
        assert!(!e.typed_sets.contains_key(&unmatched.key()));
        assert!(
            e.make(
                &ParticleOptions::named("mymod:empty_asset"),
                [0.0; 3],
                [0.0; 3],
                3
            )
            .is_none()
        );
        let scene = Scene::new();
        let value = orbit.options(());
        let particle = e
            .make_typed_particle(&scene.world, value.typed().unwrap(), [0.0; 3], [0.0; 3], 3)
            .unwrap()
            .unwrap();
        // The declared spawn values seed it, and its sprite is drawn by frame from the set.
        assert!(particle.provided.is_some());
        assert_eq!((particle.lifetime, particle.size), (7, 0.3));
        assert_eq!(particle.set, Some(e.typed_sets[&orbit.key()].set));
        let Sprite::Index(index) = particle.sprite else {
            panic!("expected missing sprite binding")
        };
        assert_eq!(e.names[index as usize], crate::hud::MISSING_SPRITE);
        assert!(e.rects[index as usize].is_some());
        let plain = e
            .make(
                &ParticleOptions::named("mymod:plain"),
                [0.0; 3],
                [0.0; 3],
                3,
            )
            .unwrap();
        assert!(plain.provided.is_none());
        assert!(matches!(plain.behaviour, Behaviour::Mod { .. }));
    }
    #[test]
    fn controlled_spawn_respects_distance_settings_and_declared_sprite_sets() {
        let mut e = engine();
        assert!(
            e.controlled("example:missing", [0.0; 3], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [33.0, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [f64::NAN, 0.0, 0.0], true, Moving(0))
                .is_none()
        );
        assert!(
            e.controlled("example:orbit", [32.0, 0.0, 0.0], true, Moving(0))
                .is_some()
        );
        e.particle_level = 2;
        assert!(
            e.controlled("example:orbit", [0.0; 3], true, Moving(0))
                .is_none()
        );
    }
}
