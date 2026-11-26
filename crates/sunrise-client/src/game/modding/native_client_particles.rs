//! Local particle and firework requests from native client hooks.
use super::Game;
use mod_api::{
    Direction, FireworksRequest, LevelRef, LevelWriteError, client::particles::PositionSource,
};
use sunrise_client::particles::{Particle, ParticleLevel, ParticleOptions};
use sunrise_world::registry::BlockStateId;

fn finite_options(options: &ParticleOptions) -> bool {
    options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
        && options
            .trail
            .as_ref()
            .is_none_or(|(at, _)| at.iter().all(|v| v.is_finite()))
        && options
            .geyser
            .is_none_or(|(_, impulse)| impulse.is_finite())
        && options.vibration.as_ref().is_none_or(|(source, at, _)| {
            at.iter().all(|v| v.is_finite())
                && match source {
                    PositionSource::Block(_) => true,
                    PositionSource::Entity { y_offset, .. } => y_offset.is_finite(),
                    _ => false,
                }
        })
}

impl Game<'_> {
    pub(in crate::game) fn particle_block_states() -> Option<sunrise_client::particles::BlockStates>
    {
        super::api_values::block_state::current()
            .ok()
            .map(|states| -> sunrise_client::particles::BlockStates { states })
    }

    /// `ClientLevel.addParticle`: request one particle in this client's loaded world.
    /// Options and provider input slots pass through unchanged, including encoded
    /// color or size values in `velocity`. Normal distance, settings and population
    /// limits still apply. True means submitted, not necessarily visible: an unknown
    /// provider or missing sprite set draws nothing. False on a server, without a
    /// client player/level, or for non-finite numeric input. API @API:client_particles@.
    pub(super) fn add_particle(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> bool {
        if !self.native_window()
            || self.player.is_none()
            || !at.into_iter().all(f64::is_finite)
            || !velocity.into_iter().all(f64::is_finite)
            || !finite_options(options)
        {
            return false;
        }
        // The limiter's roll draws the level random: inside a part's `animate_tick`, the one
        // the part holds as `event.random`.
        self.with_lent_level_random(|game| {
            let level = ParticleLevel::new(&game.world, game.res.facts(), game.collision);
            game.particles
                .add_with(Some(&level), options, at, velocity, false, false);
        });
        true
    }
    pub(super) fn level_add_breaking_particles(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        face: Direction,
    ) -> Result<bool, LevelWriteError> {
        if self.native_client_level() != Some(level) {
            return Err(LevelWriteError::Unavailable);
        }
        Ok(self.add_breaking_particle(pos, state, face))
    }

    pub(super) fn level_add_terrain_particle(
        &mut self,
        level: LevelRef,
        state: BlockStateId,
        source_pos: [i32; 3],
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> Result<bool, LevelWriteError> {
        if self.native_client_level() != Some(level) {
            return Err(LevelWriteError::Unavailable);
        }
        if !at.into_iter().chain(velocity).all(f64::is_finite) {
            return Ok(false);
        }
        let Some((sprite, tint)) = self.particle_sprite(state, source_pos) else {
            return Ok(false);
        };
        let particle = Particle::terrain(self.particles.random(), at, velocity, sprite, tint);
        self.particles.push(particle);
        Ok(true)
    }

    /// `ClientLevel.createFireworks`: queue a firework's explosions in this client's loaded
    /// world through the starter a bursting rocket uses, its blast and twinkle sounds gated by
    /// `audible` as a rocket's by its silence. No entity, no damage, nothing on the server.
    /// True means queued; false on a server, without a client player/level or for non-finite
    /// input. API @API:client_fireworks@.
    pub(super) fn create_fireworks(&mut self, request: &FireworksRequest) -> bool {
        use super::api_values::TryIntoEngine;
        if !self.native_window()
            || self.player.is_none()
            || !request
                .at
                .into_iter()
                .chain(request.velocity)
                .all(f64::is_finite)
        {
            return false;
        }
        let Ok(explosions) = request
            .explosions
            .iter()
            .map(|explosion| explosion.try_into_engine(&()))
            .collect();
        let camera = self.eye_position().unwrap_or(request.at);
        // Without explosions its poofs are added at once, each with the limiter's roll.
        self.with_lent_level_random(|game| {
            game.particles.fireworks(
                request.at,
                request.velocity,
                explosions,
                camera,
                request.audible,
            );
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{ClientHost, FireworkExplosion, FireworkShape};
    use sunrise_client::particles::ParticleType;

    #[test]
    fn nonfinite_nested_particle_parameters_are_rejected() {
        let good = ParticleOptions::simple(ParticleType::Smoke);
        assert!(finite_options(&good));
        let mut bad = good.clone();
        bad.to_color[0] = f32::NAN;
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.trail = Some(([f64::INFINITY, 0.0, 0.0], 3));
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.geyser = Some((1, f32::NAN));
        assert!(!finite_options(&bad));
        bad = good;
        bad.vibration = Some((
            PositionSource::Entity {
                id: 1,
                y_offset: f32::NAN,
            },
            [0.0; 3],
            3,
        ));
        assert!(!finite_options(&bad));
        bad.vibration = Some((PositionSource::Block([0; 3]), [0.0, f64::NAN, 0.0], 3));
        assert!(!finite_options(&bad));
    }

    /// A client mod's fireworks burst either way, and blast and twinkle only when audible,
    /// as a rocket that isn't silent; a server draws nothing.
    #[test]
    #[ignore = "daily"]
    fn a_client_mods_fireworks_sound_only_when_audible() {
        use crate::game::native_entity_refs::tests::with_client_game;
        fn tick_particles(game: &mut Game<'_>) {
            let level = ParticleLevel {
                world: &game.world,
                facts: game.res.facts(),
                collision: game.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            };
            game.particles.tick(&level);
        }
        with_client_game(|game| {
            let eye = sunrise_core::geometry::Vec3::new(0.5, 200.0, 0.5);
            game.player =
                Some(sunrise_client::motion::PlayerMotion::new(game.motion, eye).unwrap());
            let at = [6.5, 201.5, 10.5];
            let request = |audible| FireworksRequest {
                at,
                velocity: [0.0, 0.05, 0.0],
                explosions: vec![FireworkExplosion {
                    shape: FireworkShape::Star,
                    colors: vec![0xFF5555, 0xFFFF55],
                    fade_colors: vec![0xFFFFFF],
                    trail: true,
                    twinkle: true,
                }],
                audible,
            };
            for (audible, expected) in [
                (
                    true,
                    vec![
                        "minecraft:entity.firework_rocket.blast",
                        "minecraft:entity.firework_rocket.twinkle",
                    ],
                ),
                (false, vec![]),
            ] {
                game.particles.clear();
                game.particles.sounds.clear();
                let host: &mut dyn ClientHost = game;
                assert!(host.create_fireworks(&request(audible)));
                for _ in 0..20 {
                    tick_particles(game);
                }
                let heard: Vec<_> = game
                    .particles
                    .sounds
                    .iter()
                    .filter(|s| s.0.starts_with("minecraft:entity.firework_rocket."))
                    .map(|s| (s.0, s.1))
                    .collect();
                let expected: Vec<_> = expected.into_iter().map(|s| (s, at)).collect();
                assert_eq!(heard, expected, "audible {audible}");
                assert!(
                    !game.particles.is_empty(),
                    "the sparks drew, audible {audible}"
                );
            }
            let host: &mut dyn ClientHost = game;
            let mut bad = request(true);
            bad.velocity[1] = f64::NAN;
            assert!(!host.create_fireworks(&bad), "non-finite input");
            game.particles.clear();
            game.particles.sounds.clear();
            let mut many = request(true);
            many.explosions = vec![
                FireworkExplosion {
                    shape: FireworkShape::Burst,
                    colors: vec![0xFF5555],
                    fade_colors: Vec::new(),
                    trail: false,
                    twinkle: false,
                };
                257
            ];
            many.explosions[256].twinkle = true;
            let host: &mut dyn ClientHost = game;
            assert!(
                host.create_fireworks(&many),
                "more than 256 explosions are queued without an item-component limit"
            );
            tick_particles(game);
            assert_eq!(
                game.particles.group_count(),
                1,
                "starter admitted on first tick"
            );
            assert!(game.particles.sounds.is_empty());
            tick_particles(game);
            assert!(
                game.particles
                    .sounds
                    .iter()
                    .any(|s| s.0 == "minecraft:entity.firework_rocket.large_blast" && s.1 == at),
                "257-explosion starter played its launch blast"
            );
            assert!(
                !game.particles.is_empty(),
                "257-explosion starter emitted sparks"
            );
            // 257 explosions with twinkle live for 257 * 2 - 1 + 15 = 528 ticks after admission:
            // life reaches 528 on tick 529 (still alive) and finishes on tick 530 with the twinkle.
            for _ in 2..529 {
                tick_particles(game);
            }
            assert!(
                !game
                    .particles
                    .sounds
                    .iter()
                    .any(|s| s.0 == "minecraft:entity.firework_rocket.twinkle"),
                "257-explosion starter still running before tick 530"
            );
            tick_particles(game);
            let heard: Vec<_> = game
                .particles
                .sounds
                .iter()
                .filter(|s| s.0.starts_with("minecraft:entity.firework_rocket."))
                .map(|s| (s.0, s.1))
                .collect();
            assert_eq!(
                heard,
                vec![
                    ("minecraft:entity.firework_rocket.large_blast", at),
                    ("minecraft:entity.firework_rocket.twinkle", at),
                ],
                "257th explosion's twinkle completes the starter's lifecycle"
            );
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::IntegratedServer;
            let host: &mut dyn ClientHost = game;
            assert!(
                !host.create_fireworks(&request(true)),
                "servers draw nothing"
            );
        });
    }
}
