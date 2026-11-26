use super::*;
use serde_json::json;
use std::cell::Cell;
use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};
use sunrise_world::environment_attributes::{AMBIENT_PARTICLES, AttributeMap, Value, attribute};

#[test]
#[ignore = "daily"]
fn animate_tick_ambient_particles_keep_the_collision_gate_and_java_draw_order() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let collision = crate::collisions::load(&root, assets.registry).unwrap();
    let tables = AnimateTables::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&ResourceRoot::directory(root.clone())).unwrap();
    let enchant_power = EnchantPower::default();
    let mut world = World::new(facts.air, 0);
    world
        .chunks
        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
    world
        .chunks
        .insert((-1, 0), Arc::new(crate::chunk::Chunk::empty(-1, 0)));
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    world.set(10, 52, 12, stone).unwrap();

    let mut engine = ParticleEngine::new(
        &ResourceSource::directory(root),
        &Default::default(),
        &Default::default(),
    );
    let ambient = attribute(AMBIENT_PARTICLES).unwrap();
    let config = AttributeMap::parse(&json!({"minecraft:visual/ambient_particles": [
        {"particle":{"type":"minecraft:ash"},"probability":0.0},
        {"particle":{"type":"minecraft:dust","color":16711680,"scale":1.5},"probability":1.0}
    ]}))
    .unwrap();
    let Value::AmbientParticles(entries) = config.apply(ambient, &ambient.default_value().unwrap())
    else {
        panic!("expected a typed particle list");
    };
    let sampled = Cell::new(None);
    let sample = |pos| {
        sampled.set(Some(pos));
        Arc::clone(&entries)
    };
    let ctx = AnimateContext {
        enchant_power: &enchant_power,
        world: &world,
        facts: &facts,
        collision: &collision,
        biomes: &biomes,
        registry: assets.registry,
        raining_at: &|_| false,
        height_no_leaves: &|_| 0,
        firefly_bush_sounds: false,
        creaking_active: false,
        sky_darken: 0,
        marker: None,
        ambient_particles: Some(&sample),
        model_offset: &|_, _, _| None,
        gateway_particles: &|_, _| 0,
        thundering: false,
        ultrawarm: false,
        block_parts: None,
    };

    // SyntheticParticles: the witness recorded addParticle, not the renderer's providers.
    engine.requested = Some(Vec::new());
    engine.seed_randoms(1, 777, 1);
    do_animate_tick(&tables, &mut engine, &ctx, [0, 64, 0], 16, true);
    assert!(engine.requested.as_ref().unwrap().is_empty());
    assert_eq!(sampled.get(), None);
    assert_eq!(engine.level_random().state(), 3_666_643_484_578);

    engine.seed_randoms(1, 888, 1);
    do_animate_tick(&tables, &mut engine, &ctx, [10, 70, -5], 16, true);
    let requests = engine.requested.as_mut().unwrap();
    let [(options, at, velocity)] = requests.as_slice() else {
        panic!("only the dust should pass the spawn chance");
    };
    assert_eq!(sampled.get(), Some([15, 77, 1]));
    assert_eq!(options.ty, T::Dust);
    assert_eq!(options.color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(options.scale.to_bits(), 1.5_f32.to_bits());
    assert_eq!(*velocity, [0.0; 3]);
    assert_eq!(
        at.map(f64::to_bits),
        [
            0x402f_e666_5bb6_ca7c,
            0x4053_73dd_9b3d_6ca6,
            0x3ff6_5022_567e_8d6a
        ]
    );
    requests.clear();
    assert_eq!(engine.level_random().state(), 175_523_950_812_787);

    // LegacyRandom's seventh draw (the first chance, after picking X, Y and Z) is exactly zero.
    let zero_probability = |_| Arc::from(&entries[..1]);
    let ctx = AnimateContext {
        ambient_particles: Some(&zero_probability),
        ..ctx
    };
    engine.seed_randoms(1, 252_022_095_509_006, 1);
    do_animate_tick(&tables, &mut engine, &ctx, [0, 64, 0], 16, true);
    let requests = engine.requested.take().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0.ty, T::Ash);
    assert_eq!(engine.level_random().state(), 25_707_281_917_278);
}
