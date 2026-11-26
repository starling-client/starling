use super::super::*;
use crate::chunk::{Chunk, Light};
use mod_api::{
    __private::particle_provider_factory, BlockState, CustomParticle, LightLayer, ParticleFacing,
    ParticleLevelView, ParticleProvider, ParticleSpawn, ParticleTickView, ParticleView,
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
    let mut other = e
        .controlled("example:orbit", [0.0; 3], true, Moving(0))
        .unwrap();
    other.lifetime = 2;
    e.tick_particle(&mut p, &level);
    e.tick_particle(&mut p, &level);
    e.tick_particle(&mut other, &level);
    assert_eq!((p.position, p.previous), ([3.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
    assert_eq!(other.position, [1.0, 0.0, 0.0]);
    assert!(matches!(p.sprite, Sprite::Index(1))); // clamped frame
    p.size = 0.1;
    e.particles.push(p);
    let mut vertices = Vec::new();
    e.render(
        DVec3::ZERO,
        glam::Quat::IDENTITY,
        0.5,
        &|_| 0,
        &mut vertices,
    );
    assert_eq!(vertices.len(), 4);
    assert!(
        vertices.iter().all(
            |v| v.color >> 24 == 127 && v.layer & crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY != 0
        )
    );
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
    let mut p = e.particles.pop().unwrap();
    e.tick_particle(&mut p, &level);
    assert!(p.removed);
    assert_eq!(p.position, [3.0, 0.0, 0.0]);
    e.particles.push(p);
    e.push(other);
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
    mod_api::__private::resolve_particles([], |_| Some(mod_api::__private::ParticleKind::Custom))
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

struct DefaultParticle;
impl CustomParticle for DefaultParticle {}

fn typed<T: PartialEq + Send + Sync + 'static>(
    ty: mod_api::ParticleType<T>,
    value: T,
) -> ParticleOptions {
    ParticleOptions::typed(ty.options(value).typed().unwrap().clone()).unwrap()
}

fn bind<T: 'static, P: ParticleProvider<T>>(
    engine: &mut ParticleEngine,
    ty: mod_api::ParticleType<T>,
    factory: impl Fn(mod_api::ParticleSpriteSet) -> P + Send + Sync + 'static,
) {
    let factory = particle_provider_factory(ty, factory);
    let provider = factory.bind(engine.set_sprites[0].len()).unwrap();
    engine.typed_sets.insert(
        ty.key(),
        ModSet {
            set: 0,
            behaviour: None,
            provider: Some(native::BoundProvider { factory, provider }),
            group: None,
        },
    );
}

fn provided(engine: &mut ParticleEngine, instance: Arc<dyn CustomParticle>) -> Particle {
    struct Factory(Arc<dyn CustomParticle>);
    struct Instance(Arc<dyn CustomParticle>);
    impl CustomParticle for Instance {
        fn extract(
            &self,
            particle: &dyn ParticleView,
            view: &mod_api::ParticleExtractView,
            output: &mut dyn mod_api::QuadParticleRenderState,
        ) {
            self.0.extract(particle, view, output);
        }
    }
    impl ParticleProvider for Factory {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            Some(Box::new(Instance(self.0.clone())))
        }
    }
    let [ty] = unit_types(["fixture"]);
    bind(engine, ty, move |_| Factory(instance.clone()));
    let scene = Scene::new();
    engine
        .make_typed_particle(
            &scene.level(),
            ty.options(()).typed().unwrap(),
            [1.0; 3],
            [0.0; 3],
            5,
        )
        .unwrap()
        .unwrap()
}

struct Extract<F>(F);
impl<F> CustomParticle for Extract<F>
where
    F: Fn(
            &dyn ParticleView,
            &mod_api::ParticleExtractView,
            &mut dyn mod_api::QuadParticleRenderState,
        ) + Send
        + Sync
        + 'static,
{
    fn extract(
        &self,
        particle: &dyn ParticleView,
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
        move |p: &dyn ParticleView,
              view: &ParticleExtractView,
              out: &mut dyn QuadParticleRenderState| {
            calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(
                (p.previous_position(), p.position()),
                ([10.0, 20.0, 25.0], [14.0, 20.0, 25.0])
            );
            assert_eq!(view.camera, [10.0, 20.0, 30.0]);
            assert_eq!(view.rotation, [0.0, 1.0, 0.0, 0.0]);
            assert_eq!(view.previous_roll, 0.25);
            assert_eq!(view.partial_tick, 0.5);
            assert_eq!(view.light_coords, 0xb0_0040);
            let x = (p.previous_position()[0]
                + (p.position()[0] - p.previous_position()[0]) * f64::from(view.partial_tick)
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
        move |p: &dyn ParticleView,
              _: &ParticleExtractView,
              out: &mut dyn QuadParticleRenderState| {
            calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(p.position(), [3.0, 0.0, -10.0]);
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
        |_: &dyn ParticleView, _: &ParticleExtractView, out: &mut dyn QuadParticleRenderState| {
            out.add_default();
            out.add(ParticleQuad::default());
            panic!("provider cannot finish extracting");
        },
    ));
    let healthy = Arc::new(Extract(
        |_: &dyn ParticleView,
         _: &ParticleExtractView,
         output: &mut dyn QuadParticleRenderState| {
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
        |_: &dyn ParticleView,
         _: &ParticleExtractView,
         output: &mut dyn QuadParticleRenderState| {
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
            spawn: &mut ParticleSpawn<'_, u32>,
            particle: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            assert_eq!(spawn.position, [1.0, 64.0, 3.0]);
            assert_eq!(spawn.velocity, [0.125, 0.25, 0.75]);
            assert_eq!(spawn.level.brightness(LightLayer::Block, [1, 64, 3]), 4);
            self.0.lock().unwrap().push(*spawn.options);
            let tint = spawn.random.next_float();
            particle.set_rgba([tint, -0.25, 2.0, 1.5]);
            particle.set_size(20.0);
            particle.set_sprite_index(*spawn.options as usize);
            particle.set_velocity(spawn.velocity);
            (*spawn.options != 2).then(|| Box::new(DefaultParticle) as Box<dyn CustomParticle>)
        }
    }
    let mut scene = Scene::new();
    scene.world.chunks.insert((0, 0), Arc::new(lit_chunk()));
    let mut e = engine();
    e.camera = [0.0; 3];
    e.particle_level = 2;
    let calls = Arc::new(Mutex::new(Vec::new()));
    bind(&mut e, ty, {
        let calls = calls.clone();
        move |_| Factory(calls.clone())
    });
    let level = scene.level();
    e.seed_randoms(37, 0, 0);
    let mut expected_provider_rng = LegacyRandom::new(37);
    let first = expected_provider_rng.next_float();
    expected_provider_rng.next_float();
    let value = ty.options(1);
    let options = ParticleOptions::typed(value.typed().unwrap().clone()).unwrap();
    e.add_always_visible_with_level(&level, &options, [1.0, 64.0, 3.0], [0.125, 0.25, 0.75]);
    assert!(
        calls.lock().unwrap().is_empty(),
        "no-flag overload keeps the distance gate"
    );
    e.camera = [1.0, 64.0, 3.0];
    e.level_random = LegacyRandom::new(1);
    e.add_always_visible_with_level(&level, &options, [1.0, 64.0, 3.0], [0.125, 0.25, 0.75]);
    assert!(
        calls.lock().unwrap().is_empty(),
        "no-flag overload keeps the setting gate"
    );
    for value in [1, 2] {
        let value = ty.options(value);
        let options = ParticleOptions::typed(value.typed().unwrap().clone()).unwrap();
        e.add_with(
            Some(&level),
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
    assert_eq!(e.random.next_long(), expected_provider_rng.next_long());

    e.tick(&level); // Admit the pending particle.
    e.tick(&level); // The instance's default tick.
    let particle = e.live().next().unwrap();
    assert_eq!(particle.age, 1);
    assert_eq!(
        (particle.size, particle.rgba),
        (20.0, [first, -0.25, 2.0, 1.5])
    );
}

#[test]
fn packet_spread_is_independent_of_factory_rng_and_a_failed_callback_ends_only_its_packet() {
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
            spawn: &mut ParticleSpawn<'_, ()>,
            particle: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert!(!self.panic, "test factory failed");
            particle.set_rgba([spawn.random.next_float(), 1.0, 1.0, 1.0]);
            particle.set_velocity(spawn.velocity);
            Some(Box::new(DefaultParticle))
        }
    }
    let [broken, healthy] = unit_types(["broken", "healthy"]);
    let mut e = engine();
    let broken_calls = Arc::new(AtomicUsize::new(0));
    let healthy_calls = Arc::new(AtomicUsize::new(0));
    for (ty, panic, calls) in [
        (broken, true, &broken_calls),
        (healthy, false, &healthy_calls),
    ] {
        let calls = calls.clone();
        bind(&mut e, ty, move |_| Factory {
            calls: calls.clone(),
            panic,
        });
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
    e.handle_level_particles(&mut random, Some(&scene.level()), &packet, &options(broken));
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
    e.handle_level_particles(
        &mut random,
        Some(&scene.level()),
        &packet,
        &options(healthy),
    );
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

fn lit_chunk() -> Chunk {
    let mut chunk = Chunk::empty(0, 0);
    let section = chunk.section_index(64).unwrap();
    Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(Box::new([0xB4; 4096]));
    chunk
}
fn lights(view: &super::ProviderLevel, pos: [i32; 3]) -> (u8, u8) {
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
            level: &level,
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
            level: &level,
            states: &states,
        };
        assert_eq!(view.block_state([1, 64, 3]).block(), stone);
        assert_eq!(view.block_state([2, 64, 3]).block(), air);
        assert_eq!(view.block_state([100, 64, 3]).block(), air);
    });
}
#[test]
fn owned_particles_construct_before_fifo_admission_tick_in_order_and_drop_once() {
    #[derive(Debug, PartialEq)]
    enum Event {
        Create(i32, i32),
        Tick(i32, i32, i32, f64, f64),
        Drop(i32),
    }
    type Events = Arc<Mutex<Vec<Event>>>;

    struct Start {
        events: Events,
        stone: BlockState,
        thread: std::thread::ThreadId,
    }
    struct Move {
        events: Events,
        id: i32,
        steps: i32,
        thread: std::thread::ThreadId,
    }
    impl ParticleProvider<f32> for Start {
        fn create_particle(
            &self,
            spawn: &mut ParticleSpawn<'_, f32>,
            particle: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            assert_eq!(std::thread::current().id(), self.thread);
            assert_eq!(*spawn.options, 2.5);
            assert_eq!(spawn.velocity, [0.25, 0.0, 0.0]);
            assert_eq!(spawn.position, particle.position());
            assert_eq!(spawn.level.block_state([1, 64, 3]), self.stone);
            assert_eq!(spawn.level.brightness(LightLayer::Block, [1, 64, 3]), 4);
            let id = spawn.position[0] as i32;
            let draw = spawn.random.next_int();
            self.events.lock().unwrap().push(Event::Create(id, draw));
            particle.set_has_physics(false);
            particle.set_velocity([1.0, 0.0, 0.0]);
            particle.set_friction(1.0);
            particle.set_lifetime(if id == 10 { 2 } else { 7 });
            Some(Box::new(Move {
                events: Arc::clone(&self.events),
                id,
                steps: 0,
                thread: self.thread,
            }))
        }
    }
    impl CustomParticle for Move {
        fn tick(&mut self, particle: &mut dyn ParticleTickView, level: &dyn ParticleLevelView) {
            assert_eq!(std::thread::current().id(), self.thread);
            assert_eq!(level.brightness(LightLayer::Sky, [1, 64, 3]), 11);
            self.steps += 1;
            if self.id == 10 {
                particle.tick_base();
            } else if particle.advance_age() {
                particle.move_by([f64::from(self.steps), 0.0, 0.0]);
                if self.steps == 2 {
                    particle.remove();
                }
            }
            self.events.lock().unwrap().push(Event::Tick(
                self.id,
                self.steps,
                particle.age(),
                particle.previous_position()[0],
                particle.position()[0],
            ));
        }
    }
    impl Drop for Move {
        fn drop(&mut self) {
            self.events.lock().unwrap().push(Event::Drop(self.id));
        }
    }

    mod_platform::testing::with_mods(Vec::new().into(), || {
        let [ty] = registered_types(["orbit"], |r, name| {
            r.register_particle_type(
                name,
                false,
                mod_api::ParticleMapCodec::new(
                    |_, _| Ok(2.5f32),
                    |_, _| Ok(mod_api::Compound::default()),
                ),
                mod_api::ParticleStreamCodec::wire(),
            )
        });
        mod_api::__private::resolve_block_keys(["minecraft:air", "minecraft:stone"], |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&BTreeMap::new()], 0)
                .map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        let state = |name| {
            mod_api::__private::block_state_from_ordinal(
                mod_api::__private::block_key(name).unwrap(),
                0,
            )
        };
        let events = Events::default();
        let stone = state("minecraft:stone");
        let thread = std::thread::current().id();
        let mut scene = Scene::new();
        let mut chunk = lit_chunk();
        chunk.set(1, 64, 3, 1);
        scene.world.chunks.insert((0, 0), Arc::new(chunk));
        let level = scene.level();
        let mut e = engine();
        bind(&mut e, ty, {
            let events = events.clone();
            move |_| Start {
                events: events.clone(),
                stone,
                thread,
            }
        });
        e.set_block_state_source(|| {
            let states = ["minecraft:air", "minecraft:stone"].map(|name| {
                mod_api::__private::block_state_from_ordinal(
                    mod_api::__private::block_key(name).unwrap(),
                    0,
                )
            });
            Some(Arc::new(states))
        });
        e.camera = [0.0, 64.0, 0.0];
        e.live_limit = Some(2);
        e.parallel_min = 1;
        e.parallel_threads = 2;
        e.seed_randoms(0, 0, 0);
        let options = typed(ty, 2.5);
        for x in [40, 10, 20, 30] {
            e.add_with(
                Some(&level),
                &options,
                [f64::from(x), 64.0, 3.0],
                [0.25, 0.0, 0.0],
                false,
                false,
            );
        }
        use Event::*;
        // The far-away spawn never reaches the provider. All three near ones are built and
        // draw from java.util.Random(0) before the FIFO admits the first two.
        let created = [
            Create(10, -1155484576),
            Create(20, -723955400),
            Create(30, 1033096058),
        ];
        assert_eq!(*events.lock().unwrap(), created);
        assert_eq!((e.len(), e.pending.len()), (0, 3));
        e.tick(&level);
        assert_eq!((e.len(), e.pending.len()), (2, 0));
        e.tick(&level);
        e.tick(&level);
        assert_eq!(e.len(), 1);
        e.tick(&level);
        assert_eq!(e.len(), 0);
        let mut expected = Vec::from(created);
        expected.extend([
            Drop(30),
            Tick(10, 1, 1, 10.0, 11.0),
            Tick(20, 1, 1, 20.0, 21.0),
            Tick(10, 2, 2, 11.0, 12.0),
            Tick(20, 2, 2, 21.0, 23.0),
            Drop(20),
            Tick(10, 3, 3, 12.0, 12.0),
            Drop(10),
        ]);
        assert_eq!(*events.lock().unwrap(), expected);
    });
}

/// A particle a provider poses on each of its ticks (`rolls[age - 1]`), drawn half a tick
/// into the last one with the camera turned by `base`, as the renderer builds its corners.
fn drawn(
    facing: ParticleFacing,
    rolls: &[f32],
    center: [f64; 3],
    base: glam::Quat,
) -> Vec<[f32; 3]> {
    struct Pose(Arc<[f32]>, ParticleFacing);
    impl ParticleProvider for Pose {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            particle: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            particle.set_lifetime(20);
            particle.set_size(1.0);
            particle.set_facing(self.1);
            Some(Box::new(Roll(Arc::clone(&self.0))))
        }
    }
    struct Roll(Arc<[f32]>);
    impl CustomParticle for Roll {
        fn tick(&mut self, particle: &mut dyn ParticleTickView, _: &dyn ParticleLevelView) {
            if particle.advance_age() {
                particle.set_roll(self.0[particle.age() as usize - 1]);
            }
        }
    }
    let scene = Scene::new();
    let level = scene.level();
    let mut e = engine();
    let [ty] = unit_types(["orbit"]);
    let vals: Arc<[f32]> = rolls.into();
    bind(&mut e, ty, move |_| Pose(vals.clone(), facing));
    let mut p = e
        .make_typed_particle(&level, ty.options(()).typed().unwrap(), center, [0.0; 3], 5)
        .unwrap()
        .unwrap();
    for _ in rolls {
        e.tick_particle(&mut p, &level);
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
fn malformed_writes_are_refused_without_expiry_and_age_sprite_selection_is_exact() {
    struct Probe;
    impl ParticleProvider for Probe {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            particle: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            let position = [31_000_000.0, 0.0, 0.0];
            assert!(particle.set_position(position));
            assert!(!particle.set_position([f64::NAN; 3]));
            assert_eq!(particle.position(), position);
            assert!(particle.set_velocity([f64::MAX, 0.0, 0.0]));
            assert!(!particle.set_velocity([f64::INFINITY; 3]));
            assert!(!particle.move_by([f64::NAN; 3]));
            assert_eq!(particle.velocity(), [f64::MAX, 0.0, 0.0]);
            assert!(particle.set_size(32.0));
            assert!(!particle.set_size(f32::INFINITY));
            assert_eq!(particle.size(), 32.0);
            assert!(particle.set_roll(f32::MAX));
            assert!(!particle.set_roll(f32::NAN));
            assert_eq!(particle.roll(), f32::MAX);
            assert!(particle.set_facing(ParticleFacing::Fixed([0.0, 0.0, 0.0, f32::MAX])));
            assert!(!particle.set_facing(ParticleFacing::Fixed([0.0; 4])));
            assert_eq!(
                particle.facing(),
                ParticleFacing::Fixed([0.0, 0.0, 0.0, 1.0])
            );

            assert!(particle.set_sprite_index(1));
            assert!(!particle.set_sprite_index(2));
            particle.set_lifetime(0);
            assert!(!particle.set_sprite_from_age());
            assert_eq!(particle.sprite_index(), 1);
            particle.set_lifetime(4);
            particle.set_age(3);
            assert!(particle.set_sprite_from_age());
            assert_eq!(particle.sprite_index(), 0);
            particle.set_age(4);
            assert!(particle.set_sprite_from_age());
            assert_eq!(particle.sprite_index(), 1);
            particle.set_age(8);
            assert!(!particle.set_sprite_from_age());
            assert_eq!(particle.sprite_index(), 1);
            assert!(particle.is_alive());

            particle.set_age(i32::MAX);
            particle.set_lifetime(i32::MAX);
            Some(Box::new(Self))
        }
    }
    impl CustomParticle for Probe {}

    let scene = Scene::new();
    let level = scene.level();
    let mut e = engine();
    let [ty] = unit_types(["orbit"]);
    bind(&mut e, ty, |_| Probe);
    let mut particle = e
        .make_typed_particle(
            &level,
            ty.options(()).typed().unwrap(),
            [0.0; 3],
            [0.0; 3],
            1,
        )
        .unwrap()
        .unwrap();
    assert!(particle.is_alive());
    assert!(matches!(particle.sprite, Sprite::Index(1)));
    e.tick_particle(&mut particle, &level);
    assert!(!particle.is_alive());
    assert_eq!(particle.age, i32::MIN); // Java post-increment, even on the expiry tick.
    assert_eq!(particle.previous, [31_000_000.0, 0.0, 0.0]);
}

#[test]
fn a_type_resolves_to_its_provider_at_load_and_other_types_stay_declarative() {
    use sunrise_assets::resource_source::Resource;
    struct Provider;
    impl ParticleProvider for Provider {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            Some(Box::new(DefaultParticle))
        }
    }
    let [ty] = unit_types(["spark"]);
    let binds = Arc::new(Mutex::new(Vec::new()));
    let factory = particle_provider_factory(ty, {
        let binds = binds.clone();
        move |sprites| {
            binds.lock().unwrap().push(sprites.frame_count());
            Provider
        }
    });
    let root = ResourceSource::indexed(
        ["spark", "plain"]
            .into_iter()
            .map(|path| {
                (
                    format!("assets/mymod/particles/{path}.json"),
                    Resource::bytes(br#"{"textures":["mymod:one","mymod:two"]}"#.to_vec()),
                )
            })
            .collect(),
    );
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    let atlas = HashMap::from([
        ("mymod:particle/one".into(), rect),
        ("mymod:particle/two".into(), rect),
    ]);
    let mut e = ParticleEngine::with_factories(&[], HashMap::from([(ty.key(), factory)]));
    e.reload(&root, &atlas, &HashSet::new());
    let scene = Scene::new();
    let level = scene.level();
    let options = typed(ty, ());
    // No state table: a typed spawn does not call mod code, and no fallback case runs.
    e.create_with_level(Some(&level), &options, [1.0; 3], [0.0; 3]);
    assert!(e.pending.is_empty());
    e.set_block_state_source(|| Some(Arc::new(Vec::<BlockState>::new())));
    e.create_with_level(Some(&level), &options, [1.0; 3], [0.0; 3]);
    e.add(&ParticleOptions::named("mymod:plain"), [2.0; 3], [0.0; 3]);
    e.tick(&level);
    assert_eq!(e.len(), 2);
    assert!(e.has_provider_particles());
    e.create_with_level(Some(&level), &options, [3.0; 3], [0.0; 3]);
    assert_eq!(e.pending.len(), 1);
    e.reload(&root, &atlas, &HashSet::new());
    assert_eq!(*binds.lock().unwrap(), [2, 2]);
    assert_eq!(e.len(), 1);
    assert!(e.pending.is_empty());
    assert!(!e.has_provider_particles());
}

#[test]
fn a_tick_panic_quarantines_only_its_registration_and_retires_its_instances() {
    struct Provider(bool);
    struct Instance(bool);
    impl ParticleProvider for Provider {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            Some(Box::new(Instance(self.0)))
        }
    }
    impl CustomParticle for Instance {
        fn tick(&mut self, p: &mut dyn ParticleTickView, _: &dyn ParticleLevelView) {
            assert!(!self.0, "tick fails only in one registration");
            p.advance_age();
        }
    }
    mod_platform::testing::with_mods(Default::default(), || {
        let [broken, healthy] = unit_types(["broken", "healthy"]);
        let mut e = engine();
        bind(&mut e, broken, |_| Provider(true));
        bind(&mut e, healthy, |_| Provider(false));
        let scene = Scene::new();
        let level = scene.level();
        for (ty, x) in [(broken, 1.0), (broken, 2.0), (healthy, 3.0)] {
            e.create_with_level(Some(&level), &typed(ty, ()), [x, 0.0, 0.0], [0.0; 3]);
        }
        e.tick(&level);
        assert_eq!(e.len(), 3);
        e.tick(&level);
        let p = e.live().next().unwrap();
        assert_eq!((e.len(), p.position[0], p.age), (1, 3.0, 1));
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "unowned provider",
                hook: "tick_custom_particle",
            }]
        );
    });
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

#[test]
fn the_block_state_table_is_not_asked_while_no_provider_particle_is_ready() {
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    fn counting() -> Option<BlockStates> {
        ASKED.fetch_add(1, Ordering::Relaxed);
        Some(Arc::new(Vec::<BlockState>::new()))
    }
    struct Provider;
    impl ParticleProvider for Provider {
        fn create_particle(
            &self,
            _: &mut ParticleSpawn<'_, ()>,
            _: &mut dyn ParticleTickView,
        ) -> Option<Box<dyn CustomParticle>> {
            Some(Box::new(DefaultParticle))
        }
    }
    let scene = Scene::new();
    let level = scene.level();
    let mut e = engine();
    e.set_block_state_source(counting);
    let [ty] = unit_types(["orbit"]);
    bind(&mut e, ty, |_| Provider);
    e.tick(&level);
    assert_eq!(ASKED.load(Ordering::Relaxed), 0);
    e.create_with_level(Some(&level), &typed(ty, ()), [1.0; 3], [0.0; 3]);
    assert_eq!(ASKED.load(Ordering::Relaxed), 1);
    e.tick(&level);
    assert_eq!(ASKED.load(Ordering::Relaxed), 1);
    e.set_block_state_source(|| None);
    e.tick(&level);
    assert_eq!(e.live().next().unwrap().age, 0);
    e.set_block_state_source(counting);
    e.tick(&level);
    assert_eq!(e.live().next().unwrap().age, 1);
    assert_eq!(ASKED.load(Ordering::Relaxed), 2);
    e.clear();
    e.tick(&level);
    assert_eq!(ASKED.load(Ordering::Relaxed), 2);
}
