#![cfg(test)]
use crate::game::{
    Game,
    execution_role::ExecutionRole,
    native_client_session::tests::logged_client,
    native_entity_refs::tests::{fixture_root, run_fresh_cases, with_game_role_and_facts},
};
use mod_api::{
    AddDestroyEffectsEvent, AddHitEffectsEvent, BlockPart, BlockPos, BlockShapeKind, ClientHost,
    Direction, Flow, Hooks, ModEntry, ParticleKey, ParticleOptions, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{
    chunk::Chunk,
    motion::PlayerMotion,
    particles::{ParticleLevel, requested_count},
};
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource as _},
};

type Calls = Arc<Mutex<Vec<(String, BlockPos, Option<Direction>)>>>;
struct Effects {
    calls: Calls,
    marker: ParticleKey,
}
#[mod_api::sunrise_part]
impl BlockPart for Effects {
    fn add_hit_effects(
        &self,
        host: &mut dyn ClientHost,
        event: &mut AddHitEffectsEvent<'_>,
    ) -> Flow {
        let id = event.state.block().id(&*host).unwrap().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), event.pos, Some(event.face)));
        if id != "minecraft:glass" {
            return Flow::Continue;
        }
        assert_eq!(
            event
                .level
                .block_shape(&*host, event.state, [8, 64, 4], BlockShapeKind::Outline),
            Some(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]])
        );
        event.random.next_int();
        assert_eq!(
            event
                .level
                .add_breaking_particles(host, [8, 64, 4], event.state, event.face),
            Ok(true)
        );
        Flow::Cancel
    }

    fn add_destroy_effects(
        &self,
        host: &mut dyn ClientHost,
        event: &mut AddDestroyEffectsEvent<'_>,
    ) -> Flow {
        let id = event.state.block().id(&*host).unwrap().to_owned();
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), event.pos, None));
        if id == "minecraft:glass" {
            // The sound capture records how many of these requests have already run.
            assert!(host.add_particle(
                &ParticleOptions::simple(self.marker),
                [4.5, 64.5, 4.5],
                [0.0; 3]
            ));
            assert_eq!(
                event.level.add_terrain_particle(
                    host,
                    event.state,
                    [4, 64, 4],
                    [8.5, 64.5, 4.5],
                    [0.0; 3]
                ),
                Ok(true)
            );
        }
        Flow::Cancel
    }
}

struct Register(Calls);
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let marker = registrar.particle_key("minecraft:heart");
        for name in [
            "minecraft:glass",
            "minecraft:dirt",
            "minecraft:barrier",
            "minecraft:air",
        ] {
            let block = registrar.block_key(name);
            registrar.block_part(
                block,
                Effects {
                    calls: self.0.clone(),
                    marker,
                },
            );
        }
    }
}

fn with_effects(test: impl FnOnce(&mut Game<'_>, &Calls)) {
    let calls = Calls::default();
    let mods = vec![ModEntry::new("debris", Box::new(Register(calls.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game_role_and_facts(
            fixture_root(),
            ExecutionRole::Client,
            Some(|registry, facts| {
                // A visible full cube with a sprite, but no subscribed destroy Part.
                let state = registry.default_state("minecraft:cobblestone").unwrap();
                facts.states[state.0 as usize].spawn_terrain_particles = false;
            }),
            |game| {
                let _server = logged_client(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 63.0, 4.5)).unwrap());
                game.audio.capture = Some(Vec::new());
                test(game, &calls);
            },
        );
    });
}

fn admit(game: &mut Game<'_>) -> usize {
    game.particles.tick(&ParticleLevel {
        world: &game.world,
        facts: game.res.facts(),
        collision: game.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    });
    game.particles.len()
}

fn fresh(name: &str) -> bool {
    run_fresh_cases(
        &format!("{}::{name}", module_path!().split_once("::").unwrap().1),
        "SUNRISE_BLOCK_PARTICLE_EFFECT_CHILD",
        &["run"],
    )
}

#[test]
#[ignore = "daily"]
fn hit_effect_replaces_the_crumb_but_not_the_sound_or_level_random() {
    if fresh("hit_effect_replaces_the_crumb_but_not_the_sound_or_level_random") {
        return;
    }
    with_effects(|game, calls| {
        for (name, expected) in [
            ("glass", 1),
            ("dirt", 1),
            ("stone", 1),
            ("barrier", 0),
            ("air", 0),
        ] {
            let state = game
                .pack
                .assets
                .registry
                .default_state(&format!("minecraft:{name}"))
                .unwrap();
            game.world.set(4, 64, 4, state).unwrap();
            game.particles.clear();
            game.audio.heard.clear();
            game.particles.seed_randoms(10, 11, 12);
            game.breaking_block_effects([4, 64, 4], [-1, 0, 0], true);
            assert_eq!(admit(game), expected, "{name}");
            if name == "glass" {
                // The client-only hit sound intentionally bypasses the server's effect capture.
                assert_eq!(
                    game.audio.heard,
                    [("minecraft:block.glass.hit".into(), 1)].into()
                );
                let mut expected_random = LegacyRandom::new(11);
                expected_random.next_int();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert_eq!(
                    game.particles.level_random().next_long(),
                    expected_random.next_long()
                );
                let mut quads = Vec::new();
                game.particles.render(
                    glam::DVec3::ZERO,
                    glam::Quat::IDENTITY,
                    0.0,
                    &|_| 0xff,
                    &mut quads,
                );
                assert_eq!(quads.len(), 4);
                assert!(quads.iter().all(|vertex| vertex.pos[0] > 7.0));
            } else if name == "air" {
                assert!(game.audio.heard.is_empty());
            }
        }
        let pos = BlockPos::new(4, 64, 4);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                ("minecraft:glass".into(), pos, Some(Direction::West)),
                ("minecraft:dirt".into(), pos, Some(Direction::West))
            ]
        );

        let level = game.native_client_level().unwrap();
        let grass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:short_grass")
            .unwrap();
        game.streaming.biome_blend = 2;
        let biomes = world_biomes!(game);
        let tint = biomes.grass(biomes.id("minecraft:plains"), 0.0, 0.0);
        let c = |shift: u32| 0.6 * ((tint >> shift) & 255) as f32 / 255.0;
        for edge in [[i32::MIN, 64, i32::MAX], [i32::MAX, 64, i32::MIN]] {
            game.particles.clear();
            assert_eq!(
                game.level_add_breaking_particles(
                    level,
                    edge,
                    grass,
                    sunrise_core::geometry::Direction::West
                ),
                Ok(true)
            );
            assert_eq!(
                game.level_add_terrain_particle(level, grass, edge, [4.5, 64.5, 4.5], [0.0; 3]),
                Ok(true)
            );
            assert_eq!(admit(game), 2);
            for particle in game.particles.live() {
                assert_eq!(particle.rgba, [c(16), c(8), c(0), 1.0]);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn break_event_uses_the_destroyed_state_and_keeps_its_sound() {
    if fresh("break_event_uses_the_destroyed_state_and_keeps_its_sound") {
        return;
    }
    with_effects(|game, calls| {
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let barrier = game
            .pack
            .assets
            .registry
            .default_state("minecraft:barrier")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        assert!(game.res.facts().get(stone).spawn_terrain_particles);
        assert!(!game.res.facts().get(barrier).spawn_terrain_particles);
        game.world.set(4, 64, 4, air).unwrap();
        game.particles.requested = Some(Vec::new());
        let before_part = requested_count();
        game.level_event_at(2001, [4, 64, 4], glass.0 as i32);
        assert_eq!(game.particles.requested.take().unwrap().len(), 1);
        assert_eq!(admit(game), 1, "the part replaces the entire vanilla burst");
        let [sound] = game.audio.capture.as_deref().unwrap() else {
            panic!("expected exactly one break sound");
        };
        assert_eq!(sound.event, "minecraft:block.glass.break");
        assert_eq!(
            sound.particles_before, before_part,
            "the sound precedes the Part's request"
        );
        assert_eq!(game.world.block(4, 64, 4), air);
        game.particles.clear();
        game.level_event_at(2001, [4, 64, 4], stone.0 as i32);
        assert_eq!(
            admit(game),
            64,
            "an unhandled full block gives the vanilla burst"
        );
        game.particles.clear();
        let cobblestone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobblestone")
            .unwrap();
        assert!(!game.res.facts().get(cobblestone).spawn_terrain_particles);
        assert!(game.particle_sprite(cobblestone, [4, 64, 4]).is_some());
        assert!(
            game.res
                .facts()
                .get(cobblestone)
                .outline
                .iter()
                .any(|b| b[4] > b[1])
        );
        game.destroy_particles([4, 64, 4], cobblestone);
        assert_eq!(
            admit(game),
            0,
            "an unhandled false flag suppresses debris even on a drawable state"
        );
        game.destroy_particles([4, 64, 4], barrier);
        assert_eq!(
            admit(game),
            0,
            "an invisible non-air state can handle destruction"
        );
        game.destroy_particles([4, 64, 4], air);
        assert_eq!(admit(game), 0);
        let pos = BlockPos::new(4, 64, 4);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                ("minecraft:glass".into(), pos, None),
                ("minecraft:barrier".into(), pos, None)
            ]
        );
    });
}
