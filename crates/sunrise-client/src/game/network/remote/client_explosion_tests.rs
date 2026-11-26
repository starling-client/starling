use super::super::{C, Game, Vec3, play};
use crate::game::{
    native_entity_refs::tests::with_client_game, native_hotbar_slot::hotbar_slot_tests::logged_in,
};
use sunrise_client::particles::ParticleType;

fn with_isolated_client(name: &str, body: impl FnOnce(&mut Game<'_>)) {
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    crate::game::in_own_process(&test, || with_client_game(body));
}

fn registry_id(game: &Game<'_>, registry: &str, name: &str) -> i32 {
    game.remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

fn roundtrip(packet: C) -> C {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    C::decode(reader.varint().unwrap(), reader.rest()).unwrap()
}

#[test]
#[ignore = "daily"]
fn explosion_validates_impulse_and_result_without_suppressing_effects() {
    with_isolated_client(
        "explosion_validates_impulse_and_result_without_suppressing_effects",
        |game| {
            let _reader = logged_in(game);
            game.streaming
                .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
            game.particles.particle_level = 0;
            game.particles.seed_randoms(719, 11, 12);
            game.particles.seed_particle_randoms(719);
            let center = [8.5, 65.0, 8.5];
            game.particles.camera = center;
            let particle = |key| sunrise_protocol::entity_data::Particle {
                id: registry_id(game, "minecraft:particle_type", key),
                data: None,
            };
            let explosion = play::Explode {
                center,
                radius: 2.0,
                block_count: 4,
                knockback: None,
                particle: particle("minecraft:explosion"),
                block_particles: vec![play::ExplosionParticle {
                    particle: particle("minecraft:smoke"),
                    scaling: 1.0,
                    speed: 0.25,
                    weight: 1,
                }],
                sound: (0, Some(("minecraft:entity.generic.explode".into(), None))),
                play_sound: true,
            };
            let start = [2.0_f64, -0.0, 1.5];
            let large = [f64::MAX, 1.0, -0.0];
            for (initial, impulse, expected, sync) in [
                (start, Some([f64::NAN, 1.0, -2.0]), start, false),
                (start, None, start, false),
                (start, Some([1.0, f64::INFINITY, -2.0]), start, false),
                (start, Some([1.0, 2.0, f64::NEG_INFINITY]), start, false),
                (start, Some([1.0, 0.0, -2.0]), [3.0, 0.0, -0.5], true),
                (large, Some([f64::MAX, -1.0, 1.0]), large, true),
            ] {
                game.particles.clear();
                game.audio.capture = Some(Vec::new());
                let player = game.player.as_mut().unwrap();
                let [x, y, z] = initial;
                player.velocity = Vec3::new(x, y, z);
                player.needs_sync = false;
                let packet = roundtrip(C::Explode(play::Explode {
                    knockback: impulse,
                    ..explosion.clone()
                }));
                let C::Explode(decoded) = &packet else {
                    unreachable!()
                };
                assert_eq!(
                    decoded.knockback.map(|v| v.map(f64::to_bits)),
                    impulse.map(|v| v.map(f64::to_bits))
                );
                game.handle_packet(packet).unwrap();
                let player = game.player.as_ref().unwrap();
                let Vec3 { x, y, z } = player.velocity;
                assert_eq!(
                    [x, y, z].map(f64::to_bits),
                    expected.map(f64::to_bits),
                    "impulse {impulse:?}"
                );
                assert_eq!(player.needs_sync, sync, "impulse {impulse:?}");
                let sound = &game.audio.capture.as_ref().unwrap()[..];
                assert!(
                    matches!(sound, [sound] if sound.event == "minecraft:entity.generic.explode" && sound.position == center)
                );
                assert!(game.particles.pending_len() > 0);
                game.tick_particle_engine();
                assert!(
                    game.particles
                        .live()
                        .any(|p| p.ty == ParticleType::Explosion)
                );
                assert!(game.particles.live().any(|p| p.ty == ParticleType::Smoke));
            }
        },
    );
}
