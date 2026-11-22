//! A frame whose only particles are a pack's GPU particle records (wiki "GPU Particles"): the
//! records aren't in the native particle count, and the frame still builds their quads when no
//! renderer draws the type's shader; rain that dies under the heightmap reads heights taken
//! before those quads, from its first frame.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::hud::Rect;
use sunrise_client::particles::polytone::Registry;
use sunrise_client::particles::{ParticleEngine, ParticleOptions};

/// A game over the prepared resources that registers no mod tables of the process (the
/// particles here are the pack's): `server_game`, not the mod fixture's `with_game`.
fn with_game(test: impl FnOnce(&mut crate::game::Game<'_>)) {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    test(&mut game);
}

/// A pack with one GPU particle type, `test:rain`, drawn with a vanilla particle sprite and
/// no shader of its own; `kill_below_heightmap` when `dies_under`.
fn rain_pack(dies_under: bool) -> ResourceSource {
    let rain = serde_json::json!({
        "type": "gpu", "texture": "minecraft:textures/particle/generic_0.png",
        "render_type": "translucent", "kill_below_heightmap": dies_under,
        "initializer": {"lifetime": "40", "size": "0.5"}
    });
    ResourceSource::indexed(
        [(
            "assets/test/polytone/custom_particles/rain.json".to_owned(),
            Resource::bytes(rain.to_string().into_bytes()),
        )]
        .into_iter()
        .collect(),
    )
}

/// `game`'s particles reading `pack`, with the rain's sprite in their atlas (as a resource load
/// puts it there) and their camera at `camera`.
fn rain_particles(game: &mut crate::game::Game<'_>, pack: &ResourceSource, camera: [f64; 3]) {
    let registry = Registry::read(pack, Default::default(), game.pack.assets.registry);
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    game.particles = ParticleEngine::new(
        pack,
        &HashMap::from([("particle/generic_0".to_owned(), rect)]),
        &HashSet::new(),
    );
    game.particles.set_polytone(Some(Arc::new(registry)));
    game.particles.camera = camera;
}

#[test]
#[ignore = "daily"]
fn a_frame_of_gpu_records_only_draws_their_quads() {
    with_game(|game| {
        let at = [0.5, 80.0, 0.5];
        rain_particles(game, &rain_pack(false), at);
        game.particles
            .add(&ParticleOptions::named("test:rain"), at, [0.0; 3]);
        assert_eq!(game.client_particle_count(), 0, "not in the native count");
        assert_eq!(game.particles.polytone_gpu_records(), 1);
        let camera = glam::DVec3::new(at[0], at[1] + 2.0, at[2]);
        game.polytone_gpu_decide(camera);
        let mut out = Vec::new();
        game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
        assert_eq!(out.len(), 4, "the record's quad");
    });
}

/// In the first frame of a rain-only scene, the heights are taken before the quads: of a drop
/// above the world's surface and one under it, only the first draws.
#[test]
#[ignore = "daily"]
fn rain_under_the_surface_is_hidden_from_its_first_frame() {
    with_game(|game| {
        // The fixture's world has nothing in these columns: the surface is its bottom.
        let bottom = f64::from(game.world.bottom_y());
        let camera = [0.5, bottom + 4.0, 0.5];
        rain_particles(game, &rain_pack(true), camera);
        let rain = ParticleOptions::named("test:rain");
        game.particles
            .add(&rain, [0.5, bottom + 2.0, 0.5], [0.0; 3]);
        game.particles
            .add(&rain, [0.5, bottom - 2.0, 0.5], [0.0; 3]);
        assert_eq!(game.particles.polytone_gpu_records(), 2);
        let camera = glam::DVec3::from_array(camera);
        game.polytone_gpu_decide(camera);
        let mut out = Vec::new();
        game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
        assert_eq!(out.len(), 4, "the drop above the surface only");
    });
}

/// A mod's clear asked before the frame is applied before the heightmap's readers are counted,
/// in the same call: the cleared rain is gone when they are.
#[test]
#[ignore = "daily"]
fn a_pending_clear_goes_before_the_heightmap_is_read() {
    with_game(|game| {
        let bottom = f64::from(game.world.bottom_y());
        let camera = [0.5, bottom + 4.0, 0.5];
        rain_particles(game, &rain_pack(true), camera);
        game.particles.add(
            &ParticleOptions::named("test:rain"),
            [0.5, bottom + 2.0, 0.5],
            [0.0; 3],
        );
        game.mod_clear_particles.set(true);
        game.polytone_gpu_decide(glam::DVec3::from_array(camera));
        assert_eq!(game.particles.polytone_gpu_records(), 0, "cleared first");
    });
}
