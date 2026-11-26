//! Sounds and particles in a level for native server mods, through a
//! `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387).
//! Every player in range hears or sees them, as `ServerLevel.playSound` and
//! `sendParticles` send them. They return false on a client, for a handle of
//! an earlier open, or for arguments the network can't carry.
use super::api_values::TryIntoEngine;
use super::journal_level_write::PendingWrite;
use super::native_hooks::{mod_particles_ok, sound_id};
use super::{
    Game, creatures::NativeSound, lan_particles::QueuedParticle, level_sounds::LevelSound,
};
use mod_api::{LevelRef, PlayerRef, PlayerWorldRefusal, SoundSource};
use sunrise_client::particles::ParticleOptions;
use sunrise_world::registry::BlockStateId;

/// A level or entity-bound native sound request, before dispatch or queueing.
pub(super) struct NativeSoundRequest<'a> {
    pub(super) level: LevelRef,
    pub(super) sound: &'a str,
    pub(super) at: [f64; 3],
    pub(super) source: SoundSource,
    pub(super) volume: f32,
    pub(super) pitch: f32,
    pub(super) except: Option<u32>,
    pub(super) entity: Option<i32>,
    /// A direct holder, with or without a fixed broadcast radius.
    pub(super) inline_range: Option<Option<f32>>,
    /// The caller's seed (`ServerLevel.playSeededSound`), or `None` to draw the next one from
    /// the level's sound-seed stream when the sound is published (`Level.playSound`).
    pub(super) seed: Option<i64>,
}

impl Game<'_> {
    /// The level's dimension id, for a handle of this open on the server.
    fn native_level_dimension(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        Some(level.dimension_id())
    }
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(except, ...)` does, under `source`'s volume
    /// slider, leaving out `except`'s client (a fake player has none, so it
    /// leaves out no one; a stale one queues nothing). Registry entries are sent by ID, and
    /// other valid keys by name. True means queued, not heard.
    pub(super) fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
    ) -> bool {
        self.native_level_sound(level, sound, at, source, volume, pitch, except, None)
    }
    /// [`Self::level_play_sound`] with the caller's seed (`ServerLevel.playSeededSound`): the
    /// level's sound-seed stream does not move.
    pub(super) fn level_play_seeded_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: i64,
    ) -> bool {
        self.native_level_sound(level, sound, at, source, volume, pitch, except, Some(seed))
    }
    /// The next seed of the stream the level's sounds draw from
    /// (`Level.soundSeedGenerator.nextLong()`), drawn now without playing a sound: the same
    /// draw a published [`Self::level_play_sound`] makes. None where that call would refuse
    /// the level or send nothing.
    pub(super) fn level_next_sound_seed(&mut self, level: LevelRef) -> Option<i64> {
        self.native_level_dimension(level)?;
        Some(self.lan.as_ref()?.hub.next_sound_seed())
    }
    /// The connection a level sound leaves out for `except` (`Level.playSound(except, ...)`):
    /// `Some(None)` for no one, and for a fake player, who has no client to leave out; `None`
    /// refuses the sound for a stale or client-side player.
    pub(super) fn native_sound_except(&self, except: Option<PlayerRef>) -> Option<Option<u32>> {
        let Some(player) = except else {
            return Some(None);
        };
        match self.player_connection(player) {
            Ok(conn) => Some(Some(conn)),
            Err(PlayerWorldRefusal::NoConnection) => Some(None),
            Err(_) => None,
        }
    }
    /// A positional level sound leaving out `except`, with `seed` as [`NativeSoundRequest`] says.
    fn native_level_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: Option<i64>,
    ) -> bool {
        let Some(except) = self.native_sound_except(except) else {
            return false;
        };
        self.native_queue_sound(NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity: None,
            inline_range: None,
            seed,
        })
    }
    /// Accept a level or entity-bound sound (`Level.playSound`), leaving out `except` and
    /// optionally binding to `entity`. In a transaction, ask the hooks at the call and defer only
    /// the resolved effect to root commit (drawing its seed on publish unless the request gave
    /// one) and discard on abort; outside one, queue it with its seed.
    pub(super) fn native_queue_sound(&mut self, request: NativeSoundRequest<'_>) -> bool {
        let NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity,
            inline_range,
            seed,
        } = request;
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if self.lan.is_none() || !sound_id(sound) {
            return false;
        }
        self.emit_level_effect(PendingWrite::Sound {
            sound: NativeSound {
                dimension,
                sound: sound.to_owned(),
                inline_range,
                at,
                source: source.index() as i32,
                volume,
                pitch,
                seed: 0,
                except,
                entity,
                skip_hooks: false,
            },
            seed,
        });
        true
    }

    /// Queues a gameplay sound in its own dimension, even when another level is in place.
    pub(in crate::game) fn queue_level_sound(
        &mut self,
        dimension: &'static str,
        sound: LevelSound<'_>,
        except: Option<u32>,
    ) -> bool {
        if self.lan.is_none() {
            return false;
        }
        self.emit_level_effect(PendingWrite::Sound {
            sound: NativeSound {
                dimension,
                sound: sound.name.to_owned(),
                inline_range: None,
                at: sound.at,
                source: sound.source.index() as i32,
                volume: sound.volume,
                pitch: sound.pitch,
                seed: 0,
                except,
                entity: sound.emitter.map(|id| id as i32),
                skip_hooks: false,
            },
            seed: None,
        });
        true
    }
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item particles need a validated stack; trail and vibration aren't supported.
    /// True means queued, not drawn.
    pub(super) fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        self.queue_level_particles(
            dimension,
            options,
            state,
            std::slice::from_ref(&at),
            spread,
            speed,
            count,
        )
    }

    pub(super) fn level_send_particles_batch(
        &mut self,
        level: LevelRef,
        options: &mod_api::ParticleOptions,
        state: Option<mod_api::BlockState>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if positions.is_empty() {
            return true;
        }
        let items = self.items();
        let (Ok(options), Ok(state)) =
            (options.try_into_engine(items), state.try_into_engine(items))
        else {
            return false;
        };
        self.queue_level_particles(dimension, &options, state, positions, spread, speed, count)
    }

    fn queue_level_particles(
        &mut self,
        dimension: &'static str,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let registry = self.pack.assets.registry;
        let state_ok = |id| registry.state(id).is_some();
        if !mod_particles_ok(options, state, state_ok, positions, spread, speed) {
            return false;
        }
        for &at in positions {
            self.emit_level_effect(PendingWrite::Particles(QueuedParticle {
                dimension,
                options: options.clone(),
                state,
                at,
                spread,
                speed: [speed; 3],
                count,
                randomization: 0,
            }));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Dim,
        native_entity_refs::tests::{run_fresh_cases, with_client_game},
        native_guest_turn_tests::with_guests,
    };
    use super::{Game, LevelRef};
    use mod_api::{__private::particle_named_key, ParticleOptions, WorldEpoch};
    use std::sync::{Arc, mpsc};
    use sunrise_client::lan::{Out, PlayerSender};
    use sunrise_protocol::{
        buf::Reader,
        entity_data::Particle,
        packets::play::{Clientbound as Packet, LevelEvent, LevelParticles},
    };

    fn packets(rx: &mpsc::Receiver<Out>) -> Vec<Packet> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
            })
            .map(|body| {
                let mut r = Reader::new(&body);
                Packet::decode(r.varint().unwrap(), r.rest()).unwrap()
            })
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn particle_batch_is_atomic_and_follows_level_delivery_order() {
        const CASE: &str = "SUNRISE_TEST_PARTICLE_BATCH_2015";
        const A: [f64; 3] = [0.5, 64.5, 0.5];
        const B: [f64; 3] = [63.5, 64.5, 0.5];
        const C: [f64; 3] = [1.5, 64.5, 0.5];
        const E: [f64; 3] = [32.5, 64.5, 0.5];
        const SPREAD: [f64; 3] = [0.25, 0.5, 0.75];
        const SPEED: f64 = 0.125;
        const COUNT: i32 = 3;
        if run_fresh_cases(
            "game::native_level_effects::tests::particle_batch_is_atomic_and_follows_level_delivery_order",
            CASE,
            &["server", "client"],
        ) {
            return;
        }
        let heart = || ParticleOptions::simple(particle_named_key("minecraft:heart").unwrap());
        let send_at = |game: &mut Game<'_>,
                       level: LevelRef,
                       options: &ParticleOptions,
                       positions: &[[f64; 3]]| {
            level.send_particles_batch(game, options, None, positions, SPREAD, SPEED, COUNT)
        };
        if std::env::var(CASE).unwrap() == "client" {
            with_client_game(|game| {
                let epoch = game.native_lifecycle.epoch().unwrap();
                let level = LevelRef::new(epoch, game.dim.key());
                let heart = heart();
                assert!(!send_at(game, level, &heart, &[A]));
                assert!(!send_at(game, level, &heart, &[]));
            });
            return;
        }
        with_guests(|game| {
            let (tx_near, near) = mpsc::channel();
            let (tx_far, far) = mpsc::channel();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx_near, Arc::default());
                guest.position = [0.9, 64.1, 0.9];
            })
            .unwrap();
            hub.update_guest(13, |guest| {
                guest.sender = PlayerSender::new(tx_far, Arc::default());
                guest.position = [63.9, 64.1, 0.9];
            })
            .unwrap();
            let heart_id = hub
                .registry_id("minecraft:particle_type", "minecraft:heart")
                .unwrap();
            let heart = heart();
            let level = game.level("minecraft:overworld").unwrap();
            let send = |game: &mut Game<'_>, positions: &[[f64; 3]]| {
                send_at(game, level, &heart, positions)
            };
            let particle = |pos| {
                Packet::LevelParticles(LevelParticles {
                    particle: Particle {
                        id: heart_id,
                        data: None,
                    },
                    override_limiter: false,
                    always_show: false,
                    pos,
                    dist: [0.25, 0.5, 0.75],
                    max_speed: [0.125; 3],
                    count: 3,
                    randomization: 0,
                })
            };
            // E is less than 32 blocks from the first guest's position, but exactly 32
            // from the centre of that guest's block. Only the second guest receives E.
            assert!(send(game, &[B, A, E, C, A]));
            game.flush_lan_particles();
            assert_eq!(packets(&near), [particle(A), particle(C), particle(A)]);
            assert_eq!(packets(&far), [particle(B), particle(E)]);

            let mut invalid_options = heart.clone();
            invalid_options.scale = f32::NAN;
            assert!(send_at(game, level, &invalid_options, &[]));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert!(!send_at(game, stale, &heart, &[]));
            assert!(!send(game, &[A, [f64::NAN, 64.5, 0.5]]));
            let block = ParticleOptions::simple(particle_named_key("minecraft:block").unwrap());
            assert!(!send_at(game, level, &block, &[A]));
            game.flush_lan_particles();
            assert!(packets(&near).is_empty());
            assert!(packets(&far).is_empty());

            let marker_pos = [-8, 64, 0];
            let marker = |kind| {
                Packet::LevelEvent(LevelEvent {
                    kind,
                    pos: marker_pos,
                    data: 0,
                    global: false,
                })
            };
            game.storage_transaction(|game, _| {
                assert!(level.level_event(game, 1000, marker_pos, 0));
                game.storage_transaction(|game, _| {
                    assert!(send(game, &[A, C, A]));
                    Ok::<(), ()>(())
                })
                .unwrap();
                assert_eq!(
                    game.storage_transaction(|game, _| {
                        assert!(send(game, &[A, B]));
                        Err::<(), _>("nested abort")
                    }),
                    Err("nested abort")
                );
                assert!(level.level_event(game, 1001, marker_pos, 0));
                assert!(packets(&near).is_empty());
                assert!(packets(&far).is_empty());
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(
                packets(&near),
                [
                    marker(1000),
                    particle(A),
                    particle(C),
                    particle(A),
                    marker(1001)
                ]
            );
            assert!(packets(&far).is_empty());

            assert_eq!(
                game.storage_transaction(|game, _| {
                    game.storage_transaction(|game, _| {
                        assert!(send(game, &[A, B]));
                        Ok::<(), ()>(())
                    })
                    .unwrap();
                    Err::<(), _>("outer abort")
                }),
                Err("outer abort")
            );
            game.flush_lan_particles();
            assert!(packets(&near).is_empty());
            assert!(packets(&far).is_empty());

            // Both guests are now in range, but only one is in the requested dimension.
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.position = [0.9, 64.1, 0.9];
                    guest.dimension = "minecraft:the_nether".to_owned();
                })
                .unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            packets(&near);
            packets(&far);
            game.storage_transaction(|game, _| {
                assert!(send_at(game, nether, &heart, &[A, C]));
                assert!(packets(&near).is_empty());
                assert!(packets(&far).is_empty());
                Ok::<(), ()>(())
            })
            .unwrap();
            assert!(packets(&near).is_empty());
            assert_eq!(packets(&far), [particle(A), particle(C)]);
        });
    }
}
