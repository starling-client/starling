//! `AnimateContext::block_parts` and [`AnimateSampling`]: the sampling stops in each loaded pick
//! whose block has a part, after the block's own vanilla effects and before the rest of the
//! pick and the next pick, and stepping it changes no vanilla effect or draw.
use super::*;

/// A plane of torches around `[8, 64, 8]` in a loaded chunk.
fn torches(facts: &BlockFacts, torch: BlockStateId) -> World {
    let mut world = World::new(facts.air, 0);
    world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
    );
    for x in 0..16 {
        for z in 0..16 {
            world.set(x, 64, z, torch).unwrap();
        }
    }
    world
}

#[test]
#[ignore = "daily"]
fn each_loaded_pick_is_told_to_the_block_parts_after_its_vanilla_effects() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let assets = crate::assets::Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let collision = crate::collisions::load(&root, assets.registry).unwrap();
    let tables = AnimateTables::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        root.clone(),
    ))
    .unwrap();
    let torch = assets.registry.default_state("minecraft:torch").unwrap();
    let world = torches(&facts, torch);
    let has_part = |state: BlockStateId| state == torch;
    let ctx = AnimateContext {
        enchant_power: &EnchantPower::default(),
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
        ambient_particles: None,
        model_offset: &|_, _, _| None,
        gateway_particles: &|_, _| 0,
        thundering: false,
        ultrawarm: false,
        block_parts: Some(&has_part),
    };
    let engine = || {
        let mut engine = ParticleEngine::new(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            &Default::default(),
            &Default::default(),
        );
        engine.seed_randoms(5, 9, 0);
        engine.requested = Some(Vec::new());
        engine.camera = [8.5, 64.5, 8.5];
        engine
    };
    // Stepped: each stop is a torch pick whose own smoke and flame are the last requests; the
    // "part" adds a heart there, and the next run ends that pick and goes on.
    let mut stepped = engine();
    let mut sampling = AnimateSampling::new([8, 64, 8]);
    assert!(!sampling.started());
    let mut stops = 0;
    while let Some(step) = sampling.run(&tables, &mut stepped, &ctx) {
        let AnimateStep::BlockPart(state, pos) = step else {
            panic!("the context has no ambient sources");
        };
        assert!(sampling.started());
        assert_eq!(state, torch);
        assert_eq!(world.block(pos[0], pos[1], pos[2]), torch);
        let made = stepped.requested.as_ref().unwrap();
        let tail: Vec<_> = made[made.len() - 2..].iter().map(|r| r.0.ty).collect();
        assert_eq!(
            tail,
            [T::Smoke, T::Flame],
            "the torch's own effects come first"
        );
        let at = pos.map(|v| f64::from(v) + 0.5);
        stepped.add(&ParticleOptions::simple(T::Heart), at, [0.0; 3]);
        stops += 1;
    }
    assert!(stops > 0, "the tick picked a torch");
    // Unstepped (no part asked): the same vanilla requests and draws, without the hearts.
    let mut plain = engine();
    let quiet = AnimateContext {
        block_parts: None,
        ..ctx
    };
    animate_tick(&tables, &mut plain, &quiet, [8, 64, 8]);
    let hearts = |r: &&(ParticleOptions, _, _)| r.0.ty == T::Heart;
    let stepped_made = stepped.requested.take().unwrap();
    assert_eq!(stepped_made.iter().filter(hearts).count(), stops);
    let without: Vec<_> = stepped_made.iter().filter(|r| !hearts(r)).collect();
    let plain_made = plain.requested.take().unwrap();
    assert_eq!(without, plain_made.iter().collect::<Vec<_>>());
    assert_eq!(stepped.random().next_long(), plain.random().next_long());
    assert_eq!(
        stepped.level_random().next_long(),
        plain.level_random().next_long()
    );
    // A pick in a chunk that isn't loaded draws nothing and asks nobody.
    let mut far = engine();
    let mut sampling = AnimateSampling::new([400, 64, 400]);
    assert!(sampling.run(&tables, &mut far, &ctx).is_none());
    assert!(far.requested.take().unwrap().is_empty());
}
