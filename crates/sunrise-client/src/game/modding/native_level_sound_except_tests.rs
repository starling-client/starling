//! `LevelRef::play_sound` leaving one player out: the player handle is translated to the
//! connection inside the game.

use super::native_guest_turn_tests::with_guests;
use mod_api::SoundSource;
use std::sync::{Arc, mpsc};
use sunrise_client::lan::{Out, PlayerSender};
use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};

fn decode_packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).ok()
        })
        .collect()
}

fn sound_packets(rx: &mpsc::Receiver<Out>) -> Vec<sunrise_protocol::packets::play::Sound> {
    decode_packets(rx)
        .into_iter()
        .filter_map(|packet| match packet {
            C::Sound(sound) => Some(sound),
            _ => None,
        })
        .collect()
}

/// Run a `with_guests` test in its own child process so loading prepared resources never replaces
/// the shared test process's language table or registries.
fn in_isolated_process(test_name: &str, body: impl FnOnce()) {
    crate::game::in_own_process(
        &format!("game::native_level_sound_except_tests::{test_name}"),
        body,
    );
}

/// A sound queued with `except` names that player's connection; without it, no one.
#[test]
#[ignore = "daily"]
fn a_level_sound_leaves_out_the_player_asked() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        let player = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let at = [0.0, 64.0, 0.0];
        let sound = "minecraft:block.bell.use";
        let play = |game: &mut super::Game<'_>, except| {
            game.level_play_sound(level, sound, at, SoundSource::Blocks, 1.0, 1.0, except)
        };
        let queued = |game: &super::Game<'_>| game.creatures.lan_sounds.len();
        let before = queued(game);
        assert!(play(game, Some(player)));
        assert_eq!(game.creatures.lan_sounds.last().unwrap().except, Some(13));
        assert!(play(game, None));
        assert_eq!(game.creatures.lan_sounds.last().unwrap().except, None);
        assert_eq!(queued(game), before + 2);
        let gone = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(player.entity().epoch().get() + 1),
                player.entity().id(),
            ),
            player.uuid(),
        );
        assert!(!play(game, Some(gone)), "a stale player queues nothing");
        assert_eq!(queued(game), before + 2);
    });
}

/// Sounds played through `LevelRef::play_sound` inside an open transaction are held until root
/// commit and discarded on rollback, including nested savepoints, while preserving `except` and
/// argument validation.
#[test]
#[ignore = "daily"]
fn a_level_sound_inside_a_transaction_waits_for_commit_and_discards_on_rollback() {
    in_isolated_process(
        "a_level_sound_inside_a_transaction_waits_for_commit_and_discards_on_rollback",
        || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let (tx12, rx12) = mpsc::channel();
                let (tx13, rx13) = mpsc::channel();
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender = PlayerSender::new(tx12, Arc::default());
                });
                game.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                    g.sender = PlayerSender::new(tx13, Arc::default());
                });
                let player13 = game.player(13).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let at = [3.0, 64.0, 5.0];
                let bell = "minecraft:block.bell.use";
                let anvil = "minecraft:block.anvil.use";
                let gone = mod_api::__private::game::player_ref(
                    mod_api::EntityRef::new(
                        mod_api::WorldEpoch::new(player13.entity().epoch().get() + 1),
                        player13.entity().id(),
                    ),
                    player13.uuid(),
                );
                let bell_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", bell)
                    .unwrap();
                let anvil_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", anvil)
                    .unwrap();

                // Rolled-back transaction: nothing is queued while open and nothing publishes on abort.
                let rolled = mod_api::run_transaction(game, |game, _| {
                    assert!(game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    assert!(!game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        Some(gone)
                    ));
                    assert!(
                        game.creatures.lan_sounds.is_empty(),
                        "deferred sound must not queue before commit"
                    );
                    assert!(sound_packets(&rx12).is_empty());
                    assert!(sound_packets(&rx13).is_empty());
                    Err::<(), ()>(())
                });
                assert!(rolled.is_err());
                assert!(game.creatures.lan_sounds.is_empty());
                assert!(sound_packets(&rx12).is_empty());
                assert!(sound_packets(&rx13).is_empty());

                // Nested transactions: an aborted child drops its sound; a committed child waits for root
                // commit and respects `except`.
                let committed = mod_api::run_transaction(game, |game, _| {
                    let inner_abort = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            anvil,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Err::<(), ()>(())
                    });
                    assert!(inner_abort.is_err());

                    let inner_commit = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            Some(player13)
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(inner_commit.is_ok());
                    // Still inside the outer transaction: neither queued nor sent yet.
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert!(sound_packets(&rx12).is_empty());
                    assert!(sound_packets(&rx13).is_empty());
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());

                let sounds12 = sound_packets(&rx12);
                let sounds13 = sound_packets(&rx13);
                assert_eq!(sounds12.len(), 1, "only the committed bell sound publishes");
                assert_eq!(sounds12[0].sound.0, bell_id);
                assert_ne!(sounds12[0].sound.0, anvil_id);
                assert!(
                    sounds13.is_empty(),
                    "committed deferred sound still leaves out `except`"
                );

                // Outside a transaction, `level_play_sound` keeps queueing immediately as before.
                let before = game.creatures.lan_sounds.len();
                assert!(game.level_play_sound(
                    level,
                    anvil,
                    at,
                    SoundSource::Blocks,
                    1.0,
                    1.0,
                    None
                ));
                assert_eq!(game.creatures.lan_sounds.len(), before + 1);
            });
        },
    );
}

/// Entity sounds (`EntityRef::play_sound`), entity-bound sounds (`EntityRef::play_bound_sound`),
/// player sounds/stops (`PlayerRef::play_sound` / `stop_sound`), entity events
/// (`EntityRef::broadcast_event`), particles (`LevelRef::send_particles`), level events
/// (`LevelRef::level_event`), and global level events (`LevelRef::global_level_event`, with
/// `global_sound_events` both on and off) all route through the shared deferred `PendingWrite`
/// path inside open transactions, discarding on rollback or aborted savepoint and publishing only
/// on root commit.
#[test]
#[ignore = "daily"]
fn entity_sounds_bound_sounds_particles_and_level_events_defer_inside_transactions() {
    in_isolated_process(
        "entity_sounds_bound_sounds_particles_and_level_events_defer_inside_transactions",
        || {
            use sunrise_client::gamerules::Value;
            use sunrise_client::particles::{ParticleOptions, ParticleType};
            use sunrise_core::random::{LegacyRandom, RandomSource};

            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.audio.capture = Some(Vec::new());
                let (tx12, rx12) = mpsc::channel();
                let (tx13, rx13) = mpsc::channel();
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender = PlayerSender::new(tx12, Arc::default());
                });
                game.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                    g.sender = PlayerSender::new(tx13, Arc::default());
                });
                let player13 = game.player(13).unwrap();
                let entity13 = player13.entity();
                let entity13_id = i32::try_from(entity13.id()).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let at = [3.0, 64.0, 5.0];
                let pos = [3, 64, 5];
                game.player = Some(
                    sunrise_client::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(at[0], at[1], at[2]),
                    )
                    .unwrap(),
                );
                game.audio.set_listener(at, 0.0, 0.0);
                let end_portal_spawn = "minecraft:block.end_portal.spawn";
                let portal_plays = |game: &super::Game<'_>| {
                    game.audio.heard.get(end_portal_spawn).copied().unwrap_or(0)
                };
                let burp = "minecraft:entity.player.burp";
                let bell = "minecraft:block.bell.use";
                let anvil = "minecraft:block.anvil.use";
                let burp_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", burp)
                    .unwrap();
                let bell_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", bell)
                    .unwrap();
                let anvil_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", anvil)
                    .unwrap();
                let heart = ParticleOptions::simple(ParticleType::Heart);
                let flame = ParticleOptions::simple(ParticleType::Flame);

                let clear_queues = |game: &mut super::Game<'_>| {
                    game.creatures.lan_sounds.clear();
                    game.creatures.lan_particles.clear();
                    game.level_events.clear();
                    game.lan_events.clear();
                    game.lan_global_events.clear();
                    game.lan_entity_events.clear();
                    if let Some(captured) = game.audio.capture.as_mut() {
                        captured.clear();
                    }
                };
                clear_queues(game);
                decode_packets(&rx12);
                decode_packets(&rx13);

                // Snapshot both RNG streams (hub sound_seeds and level world.random) so we can verify
                // that rolled-back transactions and aborted nested savepoints advance neither stream.
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .set_sound_seeds(LegacyRandom::new(0x5A17_CAFE));
                let mut expected_sound_seeds = LegacyRandom::new(0x5A17_CAFE);
                let mut expected_level_random = game.world.random.clone();

                // 1. Rolled-back transaction discards every deferred effect path (with
                // global_sound_events = true) and leaves both RNG streams and End-portal playback count
                // untouched.
                let portal_before_abort = portal_plays(game);
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                let rolled = mod_api::run_transaction(game, |game, _| {
                    assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                    assert!(game.entity_play_bound_sound(
                        entity13,
                        bell,
                        SoundSource::Players,
                        1.0,
                        1.0
                    ));
                    assert!(game.player_play_sound(
                        player13,
                        anvil,
                        SoundSource::Neutral,
                        1.0,
                        1.0
                    ));
                    assert!(game.player_stop_sound(
                        player13,
                        Some(anvil),
                        Some(SoundSource::Neutral)
                    ));
                    assert!(game.entity_broadcast_event(entity13, 35));
                    assert!(game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 2));
                    assert!(game.level_level_event(level, 1500, pos, 0));
                    assert!(game.level_global_level_event(level, 1038, pos, 0));
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert!(game.creatures.lan_particles.is_empty());
                    assert!(game.lan_events.is_empty());
                    assert!(game.lan_global_events.is_empty());
                    assert!(game.lan_entity_events.is_empty());
                    assert_eq!(portal_plays(game), portal_before_abort);
                    assert!(decode_packets(&rx12).is_empty());
                    assert!(decode_packets(&rx13).is_empty());
                    Err::<(), ()>(())
                });
                assert!(rolled.is_err());
                assert!(game.creatures.lan_sounds.is_empty());
                assert!(game.creatures.lan_particles.is_empty());
                assert!(game.lan_events.is_empty());
                assert!(game.lan_global_events.is_empty());
                assert!(game.lan_entity_events.is_empty());
                assert_eq!(
                    portal_plays(game),
                    portal_before_abort,
                    "rolled-back global_level_event must not increase local End-portal playback count"
                );
                assert!(decode_packets(&rx12).is_empty());
                assert!(decode_packets(&rx13).is_empty());
                assert_eq!(
                    game.world.random.clone().next_long(),
                    expected_level_random.clone().next_long(),
                    "rolled-back transaction must not advance world.random"
                );

                // 2. Nested savepoints (global_sound_events = true): aborted child drops its effects
                // without advancing RNG; committed child waits for root commit, then publishes every
                // effect directly in call order.
                let portal_before_commit = portal_plays(game);
                let committed = mod_api::run_transaction(game, |game, _| {
                    let aborted_child = mod_api::run_transaction(game, |game, _| {
                        assert!(game.entity_play_sound(entity13, anvil, 1.0, 1.0));
                        assert!(game.entity_play_bound_sound(
                            entity13,
                            anvil,
                            SoundSource::Players,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_play_sound(
                            player13,
                            burp,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_stop_sound(
                            player13,
                            Some(burp),
                            Some(SoundSource::Neutral)
                        ));
                        assert!(game.entity_broadcast_event(entity13, 2));
                        assert!(
                            game.level_send_particles(level, &flame, None, at, [0.0; 3], 0.0, 4)
                        );
                        assert!(game.level_level_event(level, 1000, pos, 0));
                        assert!(game.level_global_level_event(level, 1028, pos, 0));
                        Err::<(), ()>(())
                    });
                    assert!(aborted_child.is_err());

                    let kept_child = mod_api::run_transaction(game, |game, _| {
                        assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                        assert!(game.entity_play_bound_sound(
                            entity13,
                            bell,
                            SoundSource::Players,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_play_sound(
                            player13,
                            anvil,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_stop_sound(
                            player13,
                            Some(anvil),
                            Some(SoundSource::Neutral)
                        ));
                        assert!(game.entity_broadcast_event(entity13, 35));
                        assert!(
                            game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 3)
                        );
                        assert!(game.level_level_event(level, 1500, pos, 0));
                        assert!(game.level_global_level_event(level, 1038, pos, 0));
                        Ok::<(), ()>(())
                    });
                    assert!(kept_child.is_ok());

                    // Nothing published before outermost commit.
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert!(game.creatures.lan_particles.is_empty());
                    assert!(game.lan_events.is_empty());
                    assert!(game.lan_global_events.is_empty());
                    assert!(game.lan_entity_events.is_empty());
                    assert_eq!(portal_plays(game), portal_before_commit);
                    assert!(decode_packets(&rx12).is_empty());
                    assert!(decode_packets(&rx13).is_empty());
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());

                // Committed effects deliver directly without waiting for `lan_flush_effects`.
                assert!(game.lan_events.is_empty());
                assert!(game.lan_global_events.is_empty());
                assert!(game.lan_entity_events.is_empty());
                assert_eq!(
                    portal_plays(game),
                    portal_before_commit + 1,
                    "global_level_event plays its local global sound once on root commit"
                );

                let packets12 = decode_packets(&rx12);
                let packets13 = decode_packets(&rx13);
                let expected_burp_seed = expected_sound_seeds.next_long();
                let expected_bell_seed = expected_sound_seeds.next_long();
                let expected_anvil_seed = expected_level_random.next_long();

                let sounds12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::Sound(s) => Some((s.sound.0, s.seed)),
                        _ => None,
                    })
                    .collect();
                let sounds13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::Sound(s) => Some((s.sound.0, s.seed)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    sounds12,
                    vec![(burp_id, expected_burp_seed)],
                    "entity_play_sound reaches nearby guest 12 only with the first sound seed"
                );
                assert_eq!(
                    sounds13,
                    vec![(anvil_id, expected_anvil_seed)],
                    "guest 13 receives only player_play_sound with the first world.random seed"
                );
                let stops12: Vec<_> = packets12
                    .iter()
                    .filter(|p| matches!(p, C::StopSound(_)))
                    .collect();
                let stops13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::StopSound(s) => Some(s.sound.as_deref()),
                        _ => None,
                    })
                    .collect();
                assert!(stops12.is_empty());
                assert_eq!(stops13, vec![Some(anvil)]);
                let bound12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::SoundEntity(s) => Some((s.sound.0, s.id, s.seed)),
                        _ => None,
                    })
                    .collect();
                let bound13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::SoundEntity(s) => Some((s.sound.0, s.id, s.seed)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(bound12, vec![(bell_id, entity13_id, expected_bell_seed)]);
                assert_eq!(bound13, vec![(bell_id, entity13_id, expected_bell_seed)]);
                let particles12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelParticles(p) => Some(p.count),
                        _ => None,
                    })
                    .collect();
                let particles13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelParticles(p) => Some(p.count),
                        _ => None,
                    })
                    .collect();
                assert_eq!(particles12, vec![3]);
                assert_eq!(particles13, vec![3]);
                let level_events12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                let level_events13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(level_events12, vec![(1500, false), (1038, true)]);
                assert_eq!(level_events13, vec![(1500, false), (1038, true)]);
                let entity_events13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::EntityEvent(e) => Some((e.id, e.event)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(entity_events13, vec![(entity13_id, 35)]);

                // 3. `LevelRef::global_level_event` with `global_sound_events = false` defers its plain
                // `level_event_at` fallback on rollback and emits it directly as a non-global LevelEvent
                // on commit.
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
                clear_queues(game);
                decode_packets(&rx12);
                decode_packets(&rx13);
                let rolled_plain = mod_api::run_transaction(game, |game, _| {
                    assert!(game.level_global_level_event(level, 1023, pos, 0));
                    assert!(game.lan_events.is_empty());
                    assert!(game.lan_global_events.is_empty());
                    assert!(game.audio.capture.as_ref().unwrap().is_empty());
                    Err::<(), ()>(())
                });
                assert!(rolled_plain.is_err());
                assert!(game.lan_events.is_empty());
                assert!(game.lan_global_events.is_empty());
                assert!(game.audio.capture.as_ref().unwrap().is_empty());

                let committed_plain = mod_api::run_transaction(game, |game, _| {
                    assert!(game.level_global_level_event(level, 1023, pos, 0));
                    assert!(game.lan_events.is_empty());
                    Ok::<(), ()>(())
                });
                assert!(committed_plain.is_ok());
                assert!(game.lan_global_events.is_empty());
                assert!(game.lan_events.is_empty());
                assert!(game.audio.capture.as_ref().unwrap().is_empty());
                let plain12: Vec<_> = decode_packets(&rx12)
                    .into_iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(plain12, vec![(1023, false)]);
                decode_packets(&rx13);

                // 4. Outside a transaction, every effect path continues queueing/sending immediately.
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                clear_queues(game);
                assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                assert!(game.entity_play_bound_sound(
                    entity13,
                    bell,
                    SoundSource::Players,
                    1.0,
                    1.0
                ));
                assert!(game.player_play_sound(player13, anvil, SoundSource::Neutral, 1.0, 1.0));
                assert!(game.player_stop_sound(player13, Some(anvil), Some(SoundSource::Neutral)));
                assert!(game.entity_broadcast_event(entity13, 35));
                assert!(game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 1));
                assert!(game.level_level_event(level, 1500, pos, 0));
                assert!(game.level_global_level_event(level, 1038, pos, 0));
                assert_eq!(game.creatures.lan_sounds.len(), 2);
                assert_eq!(game.creatures.lan_sounds[0].entity, None);
                assert_eq!(game.creatures.lan_sounds[1].entity, Some(entity13_id));
                assert_eq!(game.creatures.lan_particles.len(), 1);
                assert_eq!(game.lan_events.len(), 1);
                assert_eq!(game.lan_global_events.len(), 1);
                assert_eq!(game.lan_entity_events, vec![(entity13_id, 35)]);
                let direct13 = decode_packets(&rx13);
                assert!(direct13.iter().any(|p| matches!(p, C::Sound(_))));
                assert!(direct13.iter().any(|p| matches!(p, C::StopSound(_))));
            });
        },
    );
}

/// A transaction that emits Nether effects and commits while the Overworld is active replays each
/// effect in its recorded `Dim` (so Nether events/particles/sounds never leak to Overworld guests
/// and `PlayerRef::play_sound` advances Nether's `world.random` rather than the Overworld's) and
/// delivers packets on a single connection in exact call order across all effect kinds (including
/// `GlobalEvent(1038)` followed by `StopSound`, both outside and inside `as_guest`).
#[test]
#[ignore = "daily"]
fn cross_level_transaction_replays_effects_in_request_level_and_preserves_packet_order() {
    in_isolated_process(
        "cross_level_transaction_replays_effects_in_request_level_and_preserves_packet_order",
        || {
            use sunrise_client::gamerules::Value;
            use sunrise_client::particles::{ParticleOptions, ParticleType};
            use sunrise_core::random::RandomSource;

            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.audio.capture = Some(Vec::new());
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                let (tx12, rx12) = mpsc::channel();
                let (tx13, rx13) = mpsc::channel();
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender = PlayerSender::new(tx12, Arc::default());
                });
                game.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                    g.sender = PlayerSender::new(tx13, Arc::default());
                    g.dimension = "minecraft:the_nether".to_owned();
                });
                // Load Nether once so it sits in `levels.parked` when Overworld is active.
                game.enter_level(super::Dim::Nether).unwrap();
                let mut expected_nether_random = game.world.random.clone();
                game.enter_level(super::Dim::Overworld).unwrap();
                let mut expected_overworld_random = game.world.random.clone();

                let player13 = game.player(13).unwrap();
                let entity13 = player13.entity();
                let entity13_id = i32::try_from(entity13.id()).unwrap();
                let at = [3.0, 64.0, 5.0];
                let pos = [3, 64, 5];
                let anvil = "minecraft:block.anvil.use";
                let bell = "minecraft:block.bell.use";
                let end_portal_spawn = "minecraft:block.end_portal.spawn";
                let anvil_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", anvil)
                    .unwrap();
                let bell_id = game
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", bell)
                    .unwrap();
                let heart = ParticleOptions::simple(ParticleType::Heart);
                decode_packets(&rx12);
                decode_packets(&rx13);

                // Open an outer transaction in Overworld, switch to Nether and open a Nether
                // transaction to issue Nether effects across all effect kinds, commit the Nether
                // transaction, and switch back to Overworld before closing the outer transaction.
                let committed = mod_api::run_transaction(game, |game, _| {
                    game.enter_level(super::Dim::Nether).unwrap();
                    let nether = game.level("minecraft:the_nether").unwrap();
                    let nether_committed = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_global_level_event(nether, 1038, pos, 0));
                        assert!(game.player_stop_sound(player13, Some(end_portal_spawn), None));
                        assert!(game.player_play_sound(
                            player13,
                            anvil,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.level_level_event(nether, 1500, pos, 0));
                        assert!(game.entity_broadcast_event(entity13, 35));
                        assert!(
                            game.level_send_particles(nether, &heart, None, at, [0.0; 3], 0.0, 2)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(nether_committed.is_ok());
                    game.enter_level(super::Dim::Overworld).unwrap();
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());
                assert_eq!(game.dim, super::Dim::Overworld);

                // Overworld guest 12 receives ONLY the global level event (1038); no Nether-local
                // level event, entity event, particles, or sounds leak into the Overworld.
                let ow_packets = decode_packets(&rx12);
                assert_eq!(
                    ow_packets.len(),
                    1,
                    "only global event 1038 reaches Overworld guest 12"
                );
                assert!(matches!(
                    ow_packets[0],
                    C::LevelEvent(ref e) if e.kind == 1038 && e.global
                ));

                // Nether guest 13 receives every packet on one connection in exact call order, and
                // `PlayerRef::play_sound` drew its seed from Nether's `world.random`.
                let expected_nether_seed = expected_nether_random.next_long();
                let nether_packets = decode_packets(&rx13);
                let tags: Vec<&'static str> = nether_packets
                    .iter()
                    .map(|p| match p {
                        C::LevelEvent(e) if e.kind == 1038 && e.global => "global_1038",
                        C::StopSound(s) if s.sound.as_deref() == Some(end_portal_spawn) => {
                            "stop_portal"
                        }
                        C::Sound(s) if s.sound.0 == anvil_id && s.seed == expected_nether_seed => {
                            "player_anvil"
                        }
                        C::LevelEvent(e) if e.kind == 1500 && !e.global => "level_1500",
                        C::EntityEvent(e) if e.id == entity13_id && e.event == 35 => "entity_35",
                        C::LevelParticles(p) if p.count == 2 => "particles_2",
                        C::Sound(s) if s.sound.0 == bell_id => "level_bell",
                        other => panic!("unexpected Nether packet: {other:?}"),
                    })
                    .collect();
                assert_eq!(
                    tags,
                    vec![
                        "global_1038",
                        "stop_portal",
                        "player_anvil",
                        "level_1500",
                        "entity_35",
                        "particles_2",
                        "level_bell",
                    ]
                );

                // Parked-level effects issued while Overworld is active and an Overworld
                // transaction is open replay into the Nether on Overworld commit in exact order
                // and stay discarded on abort.
                let nether_handle = game.level("minecraft:the_nether").unwrap();
                let cross_abort = mod_api::run_transaction(game, |game, _| {
                    assert!(game.level_send_particles(
                        nether_handle,
                        &heart,
                        None,
                        at,
                        [0.0; 3],
                        0.0,
                        2
                    ));
                    assert!(game.level_play_sound(
                        nether_handle,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    Err::<(), ()>(())
                });
                assert!(cross_abort.is_err());
                assert!(decode_packets(&rx12).is_empty());
                assert!(decode_packets(&rx13).is_empty());

                let cross_commit = mod_api::run_transaction(game, |game, _| {
                    assert!(game.level_send_particles(
                        nether_handle,
                        &heart,
                        None,
                        at,
                        [0.0; 3],
                        0.0,
                        2
                    ));
                    assert!(game.level_play_sound(
                        nether_handle,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    Ok::<(), ()>(())
                });
                assert!(cross_commit.is_ok());
                assert_eq!(game.dim, super::Dim::Overworld);
                assert!(decode_packets(&rx12).is_empty());
                let cross_tags: Vec<&'static str> = decode_packets(&rx13)
                    .iter()
                    .map(|p| match p {
                        C::LevelParticles(p) if p.count == 2 => "particles_2",
                        C::Sound(s) if s.sound.0 == bell_id => "level_bell",
                        other => panic!("unexpected parked-Nether packet: {other:?}"),
                    })
                    .collect();
                assert_eq!(cross_tags, vec!["particles_2", "level_bell"]);

                // Overworld's `world.random` must remain completely untouched by Nether's `play_sound`.
                assert_eq!(
                    game.world.random.clone().next_long(),
                    expected_overworld_random.next_long(),
                    "Nether PlayerRef::play_sound must not advance Overworld world.random"
                );

                // Packet order also holds inside an `as_guest` turn in BOTH directions
                // (`GlobalEvent(1038)` -> `StopSound` and `StopSound` -> `GlobalEvent(1038)`).
                game.enter_level(super::Dim::Nether).unwrap();
                let nether = game.level("minecraft:the_nether").unwrap();
                decode_packets(&rx13);
                game.as_guest(13, |game| {
                    let acting = game.player(13).unwrap();
                    let res = mod_api::run_transaction(game, |game, _| {
                        assert!(game.player_stop_sound(acting, Some(anvil), None));
                        assert!(game.level_global_level_event(nether, 1038, pos, 0));
                        assert!(game.player_stop_sound(acting, Some(end_portal_spawn), None));
                        Ok::<(), ()>(())
                    });
                    assert!(res.is_ok());
                })
                .expect("guest turn runs");
                let turn_tags: Vec<&'static str> = decode_packets(&rx13)
                    .into_iter()
                    .filter_map(|p| match p {
                        C::StopSound(s) if s.sound.as_deref() == Some(anvil) => Some("stop_anvil"),
                        C::LevelEvent(e) if e.kind == 1038 && e.global => Some("global_1038"),
                        C::StopSound(s) if s.sound.as_deref() == Some(end_portal_spawn) => {
                            Some("stop_portal")
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(turn_tags, vec!["stop_anvil", "global_1038", "stop_portal"]);
            });
        },
    );
}

/// Committed effects must not drain unrelated queued `lan_sounds`/`lan_particles`/event queues,
/// `play_level_sound_at_position` runs synchronously at the call inside a transaction (so commit
/// replay executes no mod code and leaves nothing piling up in any queue), and non-player
/// `living_hurt_and_break` keeps its equipment break event immediate so an aborted transaction
/// does not leave equipment broken without its break notification.
#[test]
#[ignore = "daily"]
fn committed_effects_leave_unrelated_queues_intact_prevent_sound_recursion_and_keep_mob_break_immediate()
 {
    in_isolated_process(
        "committed_effects_leave_unrelated_queues_intact_prevent_sound_recursion_and_keep_mob_break_immediate",
        || {
            use mod_api::{EquipmentSlot, Flow, HostSide, HostTransaction, LivingRef};
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            use sunrise_client::gamerules::Value;
            use sunrise_client::items::ItemStack;
            use sunrise_client::particles::{ParticleOptions, ParticleType};

            struct ReentrantSoundHook {
                calls: Arc<AtomicUsize>,
                triggered: Arc<AtomicBool>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for ReentrantSoundHook {
                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if e.sound.to_string() == "minecraft:block.bell.use"
                        && !self.triggered.swap(true, Ordering::SeqCst)
                        && let HostSide::Server(server) = host
                    {
                        let res: Result<(), ()> = server.transaction(|server, _| {
                            assert!(e.level.play_sound(
                                server,
                                "minecraft:block.anvil.use",
                                e.position,
                                SoundSource::Blocks,
                                1.0,
                                1.0,
                                None,
                            ));
                            Ok(())
                        });
                        assert!(res.is_ok());
                    }
                    Flow::Continue
                }
            }

            let calls = Arc::new(AtomicUsize::new(0));
            let triggered = Arc::new(AtomicBool::new(false));
            let mods = vec![mod_api::ModEntry::new(
                "reentrant_sound_test",
                Box::new(ReentrantSoundHook {
                    calls: Arc::clone(&calls),
                    triggered: Arc::clone(&triggered),
                }),
            )]
            .into();

            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.audio.capture = Some(Vec::new());
                    assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                    let (tx12, rx12) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = PlayerSender::new(tx12, Arc::default());
                    });
                    let level = game.level(game.dim.id()).unwrap();
                    let player13 = game.player(13).unwrap();
                    let entity13 = player13.entity();
                    let entity13_id = i32::try_from(entity13.id()).unwrap();
                    let at = [3.0, 64.0, 5.0];
                    let pos = [3, 64, 5];
                    let burp = "minecraft:entity.player.burp";
                    let bell = "minecraft:block.bell.use";
                    let anvil = "minecraft:block.anvil.use";
                    let bell_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", bell)
                        .unwrap();
                    let anvil_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", anvil)
                        .unwrap();
                    let heart = ParticleOptions::simple(ParticleType::Heart);
                    let flame = ParticleOptions::simple(ParticleType::Flame);

                    // Part A: Queue unrelated effects outside any transaction first.
                    assert!(game.level_play_sound(
                        level,
                        burp,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    assert!(game.level_send_particles(level, &flame, None, at, [0.0; 3], 0.0, 4));
                    assert!(game.level_level_event(level, 1000, pos, 0));
                    assert!(game.level_global_level_event(level, 1028, pos, 0));
                    assert!(game.entity_broadcast_event(entity13, 2));
                    assert_eq!(game.creatures.lan_sounds.len(), 1);
                    assert_eq!(game.creatures.lan_particles.len(), 1);
                    assert_eq!(game.lan_events.len(), 1);
                    assert_eq!(game.lan_global_events.len(), 1);
                    assert_eq!(game.lan_entity_events, vec![(entity13_id, 2)]);
                    decode_packets(&rx12);

                    // Part B: Inside a transaction, `play_level_sound_at_position` runs synchronously at
                    // the call (observing `bell`, emitting `anvil`, and observing `anvil`), while commit
                    // performs pure delivery with zero mod callbacks and leaves unrelated queues intact.
                    let mut calls_before_commit = 0;
                    let committed = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        assert!(
                            game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 2)
                        );
                        assert!(game.level_level_event(level, 1500, pos, 0));
                        assert!(game.level_global_level_event(level, 1038, pos, 0));
                        assert!(game.entity_broadcast_event(entity13, 35));
                        calls_before_commit = calls.load(Ordering::SeqCst);
                        assert_eq!(
                            calls_before_commit, 1,
                            "bell ran the listener at the call; nested anvil bypassed re-dispatch"
                        );
                        assert!(
                            sound_packets(&rx12).is_empty(),
                            "no packets are sent before outermost commit"
                        );
                        Ok::<(), ()>(())
                    });
                    assert!(committed.is_ok());

                    // Unrelated queued effects must remain untouched on the normal flush path!
                    assert_eq!(game.creatures.lan_sounds.len(), 1);
                    assert_eq!(game.creatures.lan_sounds[0].sound, burp);
                    assert_eq!(game.creatures.lan_particles.len(), 1);
                    assert_eq!(game.creatures.lan_particles[0].count, 4);
                    assert_eq!(game.lan_events.len(), 1);
                    assert_eq!(game.lan_events[0].kind, 1000);
                    assert_eq!(game.lan_global_events.len(), 1);
                    assert_eq!(game.lan_global_events[0].1, 1028);
                    assert_eq!(game.lan_entity_events, vec![(entity13_id, 2)]);

                    // Commit replay ran zero mod callbacks and delivered both sounds in journal order.
                    assert!(triggered.load(Ordering::SeqCst));
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        calls_before_commit,
                        "commit replay runs no play_level_sound_at_* listeners"
                    );
                    let sounds12: Vec<i32> = sound_packets(&rx12)
                        .into_iter()
                        .map(|s| s.sound.0)
                        .collect();
                    assert_eq!(sounds12, vec![anvil_id, bell_id]);

                    // Part C: Non-player `living_hurt_and_break` inside an aborted transaction mutates the
                    // mob's equipment immediately (unjournaled), so its break event (49 for Head) must
                    // also emit immediately rather than being discarded on rollback.
                    game.lan_entity_events.clear();
                    let zombie_id = game
                        .spawn_mob(
                            "minecraft:zombie",
                            [3.5, 64.0, 5.5],
                            true,
                            super::creatures::SpawnSite::Raw,
                        )
                        .unwrap();
                    let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
                    let max_damage = game.items().stack_max_damage(&helmet);
                    assert!(max_damage > 0, "iron helmet has durability");
                    helmet.damage = max_damage - 1;
                    game.mobs
                        .iter_mut()
                        .find(|m| m.id == zombie_id)
                        .unwrap()
                        .set_equipment_stack(EquipmentSlot::Head as usize, Some(helmet));
                    let zombie_living = LivingRef::new(game.entity(zombie_id).unwrap());

                    let aborted = mod_api::run_transaction(game, |game, _| {
                        let broken =
                            game.living_hurt_and_break(zombie_living, EquipmentSlot::Head, 1);
                        assert!(broken.is_some(), "helmet breaks on final durability point");
                        Err::<(), ()>(())
                    });
                    assert!(aborted.is_err());
                    assert!(
                        game.living_equipment(zombie_living, EquipmentSlot::Head, &mut |_| ())
                            .is_none(),
                        "non-player equipment write stays broken after abort"
                    );
                    assert_eq!(
                        game.lan_entity_events,
                        vec![(zombie_id as i32, 49)],
                        "coupled non-player break event stays on the immediate side of the rollback boundary"
                    );
                });
            });
        },
    );
}

/// When a `play_level_sound_at_position` listener commits a transaction with mixed effect kinds
/// (playing a level sound and then stopping it for a player) or emits multiple follow-up sounds
/// from inside its hook, listeners run synchronously at the call, commit replay runs no mod code
/// with nothing piling up, mixed play/stop packet order holds on one connection, and rollback or
/// cancellation leaves both packets and `hub.sound_seeds` untouched.
#[test]
#[ignore = "daily"]
fn sound_listener_transaction_preserves_mixed_play_and_stop_packet_order() {
    in_isolated_process(
        "sound_listener_transaction_preserves_mixed_play_and_stop_packet_order",
        || {
            use mod_api::{EntityRef, Flow, HitHost, HostSide, HostTransaction, PlayerRef};
            use std::sync::{
                Mutex,
                atomic::{AtomicBool, AtomicUsize, Ordering},
            };
            use sunrise_core::random::{LegacyRandom, RandomSource};

            const HIT_SOUND: &str = "minecraft:entity.player.attack.strong";

            struct PlayThenStopHook {
                target: Arc<Mutex<Option<PlayerRef>>>,
                hurt_on_bell: Arc<Mutex<Option<EntityRef>>>,
                cancel_bell: Arc<AtomicBool>,
                emit_extra_burp: Arc<AtomicBool>,
                calls: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for PlayThenStopHook {
                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if e.sound.to_string() == "minecraft:block.bell.use"
                        && let HostSide::Server(server) = host
                    {
                        let player = self.target.lock().unwrap().expect("target player set");
                        let res: Result<(), ()> = server.transaction(|server, _| {
                            assert!(e.level.play_sound(
                                server,
                                "minecraft:block.anvil.use",
                                e.position,
                                SoundSource::Blocks,
                                1.0,
                                1.0,
                                None,
                            ));
                            assert!(player.stop_sound(
                                server,
                                Some("minecraft:block.anvil.use"),
                                Some(SoundSource::Blocks),
                            ));
                            if self.emit_extra_burp.load(Ordering::SeqCst) {
                                assert!(e.level.play_sound(
                                    server,
                                    "minecraft:entity.player.burp",
                                    e.position,
                                    SoundSource::Players,
                                    1.0,
                                    1.0,
                                    None,
                                ));
                            }
                            Ok(())
                        });
                        assert!(res.is_ok());
                        if let Some(victim) = self.hurt_on_bell.lock().unwrap().take() {
                            assert_eq!(
                                victim.hurt(server, "minecraft:generic", 1.0, None, None),
                                Some(false)
                            );
                        }
                        if self.cancel_bell.load(Ordering::SeqCst) {
                            return Flow::Cancel;
                        }
                    }
                    Flow::Continue
                }

                fn living_incoming_damage(
                    &self,
                    hit: &mut dyn HitHost,
                    _: &mut mod_api::LivingIncomingDamageEvent<'_>,
                ) -> Flow {
                    assert!(hit.play_sound_at(
                        HIT_SOUND,
                        [3.0, 64.0, 5.0],
                        SoundSource::Players,
                        1.0,
                        1.0
                    ));
                    Flow::Cancel
                }
            }

            let target = Arc::new(Mutex::new(None));
            let hurt_on_bell = Arc::new(Mutex::new(None));
            let cancel_bell = Arc::new(AtomicBool::new(false));
            let emit_extra_burp = Arc::new(AtomicBool::new(false));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods = vec![mod_api::ModEntry::new(
                "play_then_stop_sound_test",
                Box::new(PlayThenStopHook {
                    target: Arc::clone(&target),
                    hurt_on_bell: Arc::clone(&hurt_on_bell),
                    cancel_bell: Arc::clone(&cancel_bell),
                    emit_extra_burp: Arc::clone(&emit_extra_burp),
                    calls: Arc::clone(&calls),
                }),
            )]
            .into();

            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let (tx12, rx12) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = PlayerSender::new(tx12, Arc::default());
                    });
                    let player12 = game.player(12).unwrap();
                    *target.lock().unwrap() = Some(player12);
                    let level = game.level(game.dim.id()).unwrap();
                    let at = [3.0, 64.0, 5.0];
                    let bell = "minecraft:block.bell.use";
                    let anvil = "minecraft:block.anvil.use";
                    let burp = "minecraft:entity.player.burp";
                    let bell_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", bell)
                        .unwrap();
                    let anvil_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", anvil)
                        .unwrap();
                    let burp_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", burp)
                        .unwrap();
                    let hit_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", HIT_SOUND)
                        .unwrap();
                    decode_packets(&rx12);

                    let classify = |packets: Vec<C>| -> Vec<&'static str> {
                        packets
                            .into_iter()
                            .map(|p| match p {
                                C::Sound(s) if s.sound.0 == bell_id => "sound_bell",
                                C::Sound(s) if s.sound.0 == anvil_id => "sound_anvil",
                                C::Sound(s) if s.sound.0 == burp_id => "sound_burp",
                                C::Sound(s) if s.sound.0 == hit_id => "hit_sound",
                                C::StopSound(s) if s.sound.as_deref() == Some(anvil) => {
                                    "stop_anvil"
                                }
                                other => panic!("unexpected packet: {other:?}"),
                            })
                            .collect()
                    };

                    // 1. Rolled-back outer transaction discards both the listener's committed nested
                    // effects and the outer sound without advancing `hub.sound_seeds`.
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .set_sound_seeds(LegacyRandom::new(0xBEEF_1234));
                    let mut expected_seeds = LegacyRandom::new(0xBEEF_1234);
                    let aborted = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Err::<(), ()>(())
                    });
                    assert!(aborted.is_err());
                    assert!(decode_packets(&rx12).is_empty());
                    assert!(game.creatures.lan_sounds.is_empty());

                    // 2. Committed transaction: listener plays anvil, stops anvil, and plays burp inside
                    // its callback; all resolve at the call, commit runs no mod callbacks and sends
                    // `[sound_anvil, stop_anvil, sound_burp, sound_bell]` on one connection with nothing
                    // left queued.
                    emit_extra_burp.store(true, Ordering::SeqCst);
                    calls.store(0, Ordering::SeqCst);
                    let mut calls_at_closure_end = 0;
                    let committed = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        calls_at_closure_end = calls.load(Ordering::SeqCst);
                        assert_eq!(
                            calls_at_closure_end, 1,
                            "bell ran the listener once; nested anvil and burp bypassed re-dispatch"
                        );
                        Ok::<(), ()>(())
                    });
                    assert!(committed.is_ok());
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        calls_at_closure_end,
                        "commit replay runs no mod listeners"
                    );
                    assert!(game.creatures.lan_sounds.is_empty());
                    let packets = decode_packets(&rx12);
                    let sound_seeds: Vec<i64> = packets
                        .iter()
                        .filter_map(|p| match p {
                            C::Sound(s) => Some(s.seed),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        classify(packets),
                        vec!["sound_anvil", "stop_anvil", "sound_burp", "sound_bell"]
                    );
                    assert_eq!(
                        sound_seeds,
                        vec![
                            expected_seeds.next_long(),
                            expected_seeds.next_long(),
                            expected_seeds.next_long(),
                        ],
                        "rollback left hub.sound_seeds untouched; committed sounds take draws 1..=3"
                    );

                    // 3. Listener canceling the outer bell sound suppresses bell (and draws no seed for
                    // it) while keeping its own committed `[sound_anvil, stop_anvil]` in order.
                    emit_extra_burp.store(false, Ordering::SeqCst);
                    cancel_bell.store(true, Ordering::SeqCst);
                    let replaced = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(replaced.is_ok());
                    let replaced_packets = decode_packets(&rx12);
                    let replaced_seeds: Vec<i64> = replaced_packets
                        .iter()
                        .filter_map(|p| match p {
                            C::Sound(s) => Some(s.seed),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        classify(replaced_packets),
                        vec!["sound_anvil", "stop_anvil"]
                    );
                    assert_eq!(
                        replaced_seeds,
                        vec![expected_seeds.next_long()],
                        "canceled outer bell draws no sound seed"
                    );

                    // 4. Non-transactional tick flush (`lan_sounds`): listener's committed transaction
                    // delivers `[sound_anvil, stop_anvil]` before the outer bell sound, but the
                    // raw hit sound appended by the callback belongs to the next flush.
                    let victim = game
                        .spawn_mob(
                            "minecraft:zombie",
                            at,
                            true,
                            super::creatures::SpawnSite::Raw,
                        )
                        .unwrap();
                    *hurt_on_bell.lock().unwrap() = Some(game.entity(victim).unwrap());
                    cancel_bell.store(false, Ordering::SeqCst);
                    calls.store(0, Ordering::SeqCst);
                    assert!(game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    game.flush_native_sounds();
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        ["sound_anvil", "stop_anvil", "sound_bell"]
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    game.flush_native_sounds();
                    assert_eq!(classify(decode_packets(&rx12)), ["hit_sound"]);
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                });
            });
        },
    );
}

/// Unconditional `play_level_sound_at_position` and `play_level_sound_at_entity` listeners that
/// play another sound every time they are called must not recurse infinitely or stall the tick —
/// neither inside nor outside a transaction. Each original sound dispatches its hook once
/// (`max_depth == 1`), produces nested sounds resolved without re-entering either level-sound
/// hook, and delivers packets in exact call order within the same tick/commit. A panicking
/// listener also releases the RAII guard so subsequent sounds still dispatch hooks.
#[test]
#[ignore = "daily"]
fn always_producing_sound_listener_stops_at_one_nested_sound_in_and_out_of_transactions() {
    in_isolated_process(
        "always_producing_sound_listener_stops_at_one_nested_sound_in_and_out_of_transactions",
        || {
            use mod_api::{Flow, HostSide};
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

            struct AlwaysEmitHook {
                active_depth: Arc<AtomicUsize>,
                max_depth: Arc<AtomicUsize>,
                pos_calls: Arc<AtomicUsize>,
                entity_calls: Arc<AtomicUsize>,
                panic_once: Arc<AtomicBool>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for AlwaysEmitHook {
                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    let depth = self.active_depth.fetch_add(1, Ordering::SeqCst) + 1;
                    self.max_depth.fetch_max(depth, Ordering::SeqCst);
                    self.pos_calls.fetch_add(1, Ordering::SeqCst);
                    if self.panic_once.swap(false, Ordering::SeqCst) {
                        self.active_depth.fetch_sub(1, Ordering::SeqCst);
                        panic!("simulated level-sound listener panic");
                    }
                    if let HostSide::Server(server) = host {
                        assert!(e.level.play_sound(
                            server,
                            "minecraft:block.anvil.use",
                            e.position,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                    }
                    self.active_depth.fetch_sub(1, Ordering::SeqCst);
                    Flow::Continue
                }

                fn play_level_sound_at_entity(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtEntityEvent,
                ) -> Flow {
                    let depth = self.active_depth.fetch_add(1, Ordering::SeqCst) + 1;
                    self.max_depth.fetch_max(depth, Ordering::SeqCst);
                    self.entity_calls.fetch_add(1, Ordering::SeqCst);
                    if let HostSide::Server(server) = host {
                        assert!(e.entity.play_sound(
                            server,
                            "minecraft:entity.player.burp",
                            1.0,
                            1.0,
                        ));
                        assert!(e.entity.play_bound_sound(
                            server,
                            "minecraft:block.anvil.use",
                            SoundSource::Players,
                            1.0,
                            1.0,
                        ));
                    }
                    self.active_depth.fetch_sub(1, Ordering::SeqCst);
                    Flow::Continue
                }
            }

            struct AfterPanicHook {
                active: Arc<AtomicBool>,
                calls: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for AfterPanicHook {
                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    if !self.active.load(Ordering::SeqCst) {
                        return Flow::Continue;
                    }
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if let HostSide::Server(server) = host {
                        assert!(e.level.play_sound(
                            server,
                            "minecraft:block.anvil.use",
                            e.position,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                    }
                    Flow::Continue
                }
            }

            let active_depth = Arc::new(AtomicUsize::new(0));
            let max_depth = Arc::new(AtomicUsize::new(0));
            let pos_calls = Arc::new(AtomicUsize::new(0));
            let entity_calls = Arc::new(AtomicUsize::new(0));
            let panic_once = Arc::new(AtomicBool::new(false));
            let after_panic_active = Arc::new(AtomicBool::new(false));
            let after_panic_calls = Arc::new(AtomicUsize::new(0));
            let mods = vec![
                mod_api::ModEntry::new(
                    "always_emit_sound_test",
                    Box::new(AlwaysEmitHook {
                        active_depth: Arc::clone(&active_depth),
                        max_depth: Arc::clone(&max_depth),
                        pos_calls: Arc::clone(&pos_calls),
                        entity_calls: Arc::clone(&entity_calls),
                        panic_once: Arc::clone(&panic_once),
                    }),
                ),
                mod_api::ModEntry::new(
                    "after_panic_sound_test",
                    Box::new(AfterPanicHook {
                        active: Arc::clone(&after_panic_active),
                        calls: Arc::clone(&after_panic_calls),
                    }),
                ),
            ]
            .into();

            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let (tx12, rx12) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender = PlayerSender::new(tx12, Arc::default());
                    });
                    let level = game.level(game.dim.id()).unwrap();
                    let entity13 = game.player(13).unwrap().entity();
                    let at = [3.0, 64.0, 5.0];
                    let bell = "minecraft:block.bell.use";
                    let anvil = "minecraft:block.anvil.use";
                    let burp = "minecraft:entity.player.burp";
                    let bell_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", bell)
                        .unwrap();
                    let anvil_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", anvil)
                        .unwrap();
                    let burp_id = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .registry_id("minecraft:sound_event", burp)
                        .unwrap();
                    let classify = |packets: Vec<C>| -> Vec<&'static str> {
                        packets
                            .into_iter()
                            .filter_map(|p| match p {
                                C::Sound(s) if s.sound.0 == bell_id => Some("pos_bell"),
                                C::Sound(s) if s.sound.0 == anvil_id => Some("pos_anvil"),
                                C::Sound(s) if s.sound.0 == burp_id => Some("pos_burp"),
                                C::SoundEntity(s) if s.sound.0 == bell_id => Some("ent_bell"),
                                C::SoundEntity(s) if s.sound.0 == anvil_id => Some("ent_anvil"),
                                _ => None,
                            })
                            .collect()
                    };
                    decode_packets(&rx12);

                    // 1. Positional sound inside a transaction: `bell` dispatches `at_position` once,
                    // which plays `anvil` without re-entering the hook; commit sends `[pos_anvil, pos_bell]`.
                    let committed = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            level,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(committed.is_ok());
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 1);
                    assert_eq!(entity_calls.load(Ordering::SeqCst), 0);
                    assert_eq!(active_depth.load(Ordering::SeqCst), 0);
                    assert_eq!(max_depth.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        vec!["pos_anvil", "pos_bell"]
                    );

                    // 2. Positional sound outside a transaction: one `flush_native_sounds` finishes the
                    // tick, dispatches `at_position` once, delivers `[pos_anvil, pos_bell]` in call order,
                    // and leaves `lan_sounds` empty.
                    assert!(game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    game.flush_native_sounds();
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 2);
                    assert_eq!(entity_calls.load(Ordering::SeqCst), 0);
                    assert_eq!(active_depth.load(Ordering::SeqCst), 0);
                    assert_eq!(max_depth.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        vec!["pos_anvil", "pos_bell"]
                    );

                    // 3. Entity-bound sound inside a transaction: dispatches `at_entity` once; its nested
                    // positional (`burp`) and bound (`anvil`) sounds bypass BOTH level-sound hooks and
                    // commit in call order `[pos_burp, ent_anvil, ent_bell]`.
                    let entity_tx = mod_api::run_transaction(game, |game, _| {
                        assert!(game.entity_play_bound_sound(
                            entity13,
                            bell,
                            SoundSource::Players,
                            1.0,
                            1.0
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(entity_tx.is_ok());
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 2);
                    assert_eq!(entity_calls.load(Ordering::SeqCst), 1);
                    assert_eq!(active_depth.load(Ordering::SeqCst), 0);
                    assert_eq!(max_depth.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        vec!["pos_burp", "ent_anvil", "ent_bell"]
                    );

                    // 4. Entity-bound sound outside a transaction: one flush delivers the exact same
                    // `[pos_burp, ent_anvil, ent_bell]` sequence with `lan_sounds` empty.
                    assert!(game.entity_play_bound_sound(
                        entity13,
                        bell,
                        SoundSource::Players,
                        1.0,
                        1.0
                    ));
                    game.flush_native_sounds();
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 2);
                    assert_eq!(entity_calls.load(Ordering::SeqCst), 2);
                    assert_eq!(active_depth.load(Ordering::SeqCst), 0);
                    assert_eq!(max_depth.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        vec!["pos_burp", "ent_anvil", "ent_bell"]
                    );

                    // 5. Panic safety: the hook panic fence contains a panicking listener,
                    // records its panic report, disables only that listener, and releases the RAII
                    // guard so subsequent sounds still dispatch hooks normally to other mods.
                    let _ = mod_api::__private::take_hook_panics();
                    panic_once.store(true, Ordering::SeqCst);
                    assert!(game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    game.flush_native_sounds();
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        vec![mod_api::__private::HookPanicReport {
                            mod_id: "always_emit_sound_test",
                            hook: "play_level_sound_at_position",
                        }]
                    );
                    assert!(!game.creatures.asking_level_sound);
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 3);
                    decode_packets(&rx12);

                    after_panic_active.store(true, Ordering::SeqCst);
                    assert!(game.level_play_sound(
                        level,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    game.flush_native_sounds();
                    assert_eq!(pos_calls.load(Ordering::SeqCst), 3);
                    assert_eq!(after_panic_calls.load(Ordering::SeqCst), 1);
                    assert_eq!(max_depth.load(Ordering::SeqCst), 1);
                    assert!(game.creatures.lan_sounds.is_empty());
                    assert_eq!(
                        classify(decode_packets(&rx12)),
                        vec!["pos_anvil", "pos_bell"]
                    );
                });
            });
        },
    );
}

/// Regression for cross-level transactional writes during synchronous level-sound dispatch:
/// an Overworld transaction plays a sound in the Nether, whose listener writes a block at the
/// exact same coordinates in the Nether (directly or via a nested transaction merged into the
/// Overworld parent). Abort must restore both dimensions untouched; commit must apply only each
/// dimension's own writes without cross-dimension corruption or deduplication.
#[test]
#[ignore = "daily"]
fn cross_level_sound_listener_block_writes_journal_by_dimension_on_abort_commit_and_nested_merge() {
    in_isolated_process(
        "cross_level_sound_listener_block_writes_journal_by_dimension_on_abort_commit_and_nested_merge",
        || {
            use super::api_values::TryIntoApi;
            use mod_api::{BlockUpdateFlags, Flow, HostSide};
            use std::sync::atomic::{AtomicUsize, Ordering};
            use sunrise_client::chunk::Chunk;

            struct NetherBlockHook {
                pos: [i32; 3],
                write_state: mod_api::BlockState,
                second_write_state: mod_api::BlockState,
                /// 0 = direct (no Nether tx), 1 = nested Nether tx commits, 2 = nested Nether tx
                /// aborts, 3 = raw Nether tx left open, 4 = two-deep raw Nether txs left open.
                tx_mode: Arc<AtomicUsize>,
                calls: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for NetherBlockHook {
                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    if e.level.dimension_id() != "minecraft:the_nether" {
                        return Flow::Continue;
                    }
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if let HostSide::Server(server) = host {
                        let write_state = |server: &mut dyn mod_api::ServerHost, state| {
                            assert_eq!(
                                e.level.set_block(
                                    server,
                                    self.pos,
                                    state,
                                    BlockUpdateFlags::NEIGHBORS,
                                ),
                                Ok(true)
                            );
                        };
                        match self.tx_mode.load(Ordering::SeqCst) {
                            1 => {
                                let committed = mod_api::run_transaction(server, |server, _| {
                                    write_state(server, self.write_state);
                                    Ok::<(), ()>(())
                                });
                                assert!(committed.is_ok());
                            }
                            2 => {
                                let aborted = mod_api::run_transaction(server, |server, _| {
                                    write_state(server, self.write_state);
                                    Err::<(), ()>(())
                                });
                                assert!(aborted.is_err());
                            }
                            3 => {
                                let _abandoned = server.transaction_open();
                                write_state(server, self.write_state);
                            }
                            4 => {
                                let _outer_abandoned = server.transaction_open();
                                write_state(server, self.write_state);
                                let _inner_abandoned = server.transaction_open();
                                write_state(server, self.second_write_state);
                            }
                            _ => write_state(server, self.write_state),
                        }
                    }
                    Flow::Continue
                }
            }

            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let pos = [3, 64, 5];
                let at = [3.0, 64.0, 5.0];
                let bell = "minecraft:block.bell.use";
                let state = |game: &super::Game<'_>, name: &str| {
                    game.pack
                        .assets
                        .registry
                        .default_state(name)
                        .unwrap_or_else(|| panic!("missing block state {name}"))
                };
                let diamond = state(game, "minecraft:diamond_block");
                let netherrack = state(game, "minecraft:netherrack");
                let emerald = state(game, "minecraft:emerald_block");
                let iron = state(game, "minecraft:iron_block");
                let gold = state(game, "minecraft:gold_block");
                let emerald_api = emerald.try_into_api().unwrap();
                let iron_api = iron.try_into_api().unwrap();

                // Populate chunk (0, 0) in both dimensions with distinct blocks at the exact same coords.
                if !game.world.chunks.contains_key(&(0, 0)) {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                }
                game.world.set(pos[0], pos[1], pos[2], diamond).unwrap();

                game.enter_level(super::Dim::Nether).unwrap();
                if !game.world.chunks.contains_key(&(0, 0)) {
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                }
                game.world.set(pos[0], pos[1], pos[2], netherrack).unwrap();
                game.enter_level(super::Dim::Overworld).unwrap();

                let overworld = game.level("minecraft:overworld").unwrap();
                let nether = game.level("minecraft:the_nether").unwrap();

                let block_in = |game: &mut super::Game<'_>, dim: super::Dim| {
                    game.with_effect_level(dim, |g| g.world.block(pos[0], pos[1], pos[2]))
                        .unwrap()
                };
                assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                let tx_mode = Arc::new(AtomicUsize::new(0));
                let calls = Arc::new(AtomicUsize::new(0));
                let mods = vec![mod_api::ModEntry::new(
                    "cross_level_sound_block_journal_test",
                    Box::new(NetherBlockHook {
                        pos,
                        write_state: emerald_api,
                        second_write_state: iron_api,
                        tx_mode: Arc::clone(&tx_mode),
                        calls: Arc::clone(&calls),
                    }),
                )]
                .into();

                mod_platform::testing::with_mods(mods, || {
                    // 1. Direct listener write in Nether during an Overworld transaction that ABORTS:
                    // level-owned journals are per-dimension, so Nether's direct write does not
                    // enter Overworld's journal and Overworld's abort never overwrites Overworld's
                    // diamond_block with Nether's original netherrack.
                    let aborted = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(game.dim, super::Dim::Overworld);
                        assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                        assert_eq!(block_in(game, super::Dim::Nether), emerald);
                        Err::<(), ()>(())
                    });
                    assert!(aborted.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);

                    // 2. Direct listener write in Nether when the Overworld transaction ALSO edits the
                    // same coordinates, then ABORTS: Overworld restores diamond_block cleanly while
                    // Nether keeps its unjournaled emerald_block write.
                    game.with_effect_level(super::Dim::Nether, |g| {
                        g.world.set(pos[0], pos[1], pos[2], netherrack).unwrap();
                    })
                    .unwrap();
                    let aborted_both = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), emerald);
                        Err::<(), ()>(())
                    });
                    assert!(aborted_both.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 2);
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);

                    // 3. Nested Nether transaction that ABORTS during an Overworld transaction:
                    // rolls back Nether without disturbing Overworld's tentative write.
                    game.with_effect_level(super::Dim::Nether, |g| {
                        g.world.set(pos[0], pos[1], pos[2], netherrack).unwrap();
                    })
                    .unwrap();
                    tx_mode.store(2, Ordering::SeqCst);
                    let nether_abort_overworld_commit =
                        mod_api::run_transaction(game, |game, _| {
                            assert_eq!(
                                game.level_set_block(
                                    overworld,
                                    pos,
                                    gold,
                                    BlockUpdateFlags::NEIGHBORS
                                ),
                                Ok(true)
                            );
                            assert!(game.level_play_sound(
                                nether,
                                bell,
                                at,
                                SoundSource::Blocks,
                                1.0,
                                1.0,
                                None,
                            ));
                            assert_eq!(block_in(game, super::Dim::Overworld), gold);
                            assert_eq!(block_in(game, super::Dim::Nether), netherrack);
                            Ok::<(), ()>(())
                        });
                    assert!(nether_abort_overworld_commit.is_ok());
                    assert_eq!(calls.load(Ordering::SeqCst), 3);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_eq!(block_in(game, super::Dim::Overworld), gold);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // Reset Overworld back to diamond_block for the independent Nether commit cases.
                    game.world.set(pos[0], pos[1], pos[2], diamond).unwrap();

                    // 4. Nested transaction in Nether COMMITS its own dimension's root independently;
                    // when the outer Overworld transaction later ABORTS (even while Overworld also
                    // wrote to `pos`), Overworld restores diamond_block and Nether stays emerald_block.
                    tx_mode.store(1, Ordering::SeqCst);
                    let nested_abort = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), emerald);
                        Err::<(), ()>(())
                    });
                    assert!(nested_abort.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 4);
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);

                    // 5. Nested Nether commit + same-coordinate Overworld write both commit without
                    // deduplicating or clobbering each other across dimensions.
                    game.with_effect_level(super::Dim::Nether, |g| {
                        g.world.set(pos[0], pos[1], pos[2], netherrack).unwrap();
                    })
                    .unwrap();
                    let both_commit = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(both_commit.is_ok());
                    assert_eq!(calls.load(Ordering::SeqCst), 5);
                    assert_eq!(block_in(game, super::Dim::Overworld), gold);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);

                    let defers_retirement_in = |game: &mut super::Game<'_>, dim: super::Dim| {
                        game.with_effect_level(dim, |g| g.world.block_entities.defers_retirement())
                            .unwrap()
                    };

                    // 6. Abandoned raw Nether transaction during an Overworld transaction that
                    // ABORTS: the abandoned Nether frame must close in the Nether (restoring
                    // netherrack and publishing Nether retirement) before Overworld restores
                    // diamond_block and the Overworld item's fields, never restoring Nether's
                    // snapshot into Overworld.
                    game.world.set(pos[0], pos[1], pos[2], diamond).unwrap();
                    game.with_effect_level(super::Dim::Nether, |g| {
                        g.world.set(pos[0], pos[1], pos[2], netherrack).unwrap();
                    })
                    .unwrap();
                    let stack = sunrise_client::items::ItemStack::new("minecraft:stone", 1);
                    game.spawn_item(stack, at, [0.0; 3], 10);
                    let entity = game.entity(game.dropped.last().unwrap().id).unwrap();
                    let item = game.item_entity(entity).unwrap();
                    tx_mode.store(3, Ordering::SeqCst);
                    let abandoned_abort = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(game.dim, super::Dim::Overworld);
                        assert_eq!(game.transactions.len(), 2);
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), emerald);
                        // These writes are to the Overworld item even with Nether's frame on top.
                        assert!(item.set_no_pickup_delay(game));
                        let mut stack = item
                            .with_stack(game, |stack| stack.project())
                            .flatten()
                            .unwrap();
                        stack.set_count(2).unwrap();
                        assert_eq!(item.set_stack(game, stack), Ok(true));
                        assert_eq!(item.pickup_delay(game), Some(0));
                        assert_eq!(item.with_stack(game, |stack| stack.count()), Some(2));
                        Err::<(), ()>(())
                    });
                    assert!(abandoned_abort.is_err());
                    assert_eq!(item.pickup_delay(game), Some(10));
                    assert_eq!(item.with_stack(game, |stack| stack.count()), Some(1));
                    assert_eq!(calls.load(Ordering::SeqCst), 6);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // 7. Abandoned raw Nether transaction during an Overworld transaction that
                    // COMMITS: the abandoned Nether child aborts in the Nether (restoring
                    // netherrack) while the Overworld parent commits its own gold_block write.
                    let abandoned_commit = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(game.dim, super::Dim::Overworld);
                        assert_eq!(game.transactions.len(), 2);
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), emerald);
                        Ok::<(), ()>(())
                    });
                    assert!(abandoned_commit.is_ok());
                    assert_eq!(calls.load(Ordering::SeqCst), 7);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), gold);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // 8. Two-deep abandoned raw Nether children during an Overworld transaction
                    // that ABORTS: both Nether frames unwind in the Nether back to netherrack and
                    // Overworld restores diamond_block.
                    game.world.set(pos[0], pos[1], pos[2], diamond).unwrap();
                    tx_mode.store(4, Ordering::SeqCst);
                    let two_deep_abort = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(game.dim, super::Dim::Overworld);
                        assert_eq!(game.transactions.len(), 3);
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), iron);
                        Err::<(), ()>(())
                    });
                    assert!(two_deep_abort.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 8);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // 9. Two-deep abandoned raw Nether children during an Overworld transaction
                    // that COMMITS: both Nether frames abort in the Nether back to netherrack
                    // while Overworld keeps its committed gold_block write.
                    let two_deep_commit = mod_api::run_transaction(game, |game, _| {
                        assert_eq!(
                            game.level_set_block(overworld, pos, gold, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        assert_eq!(game.dim, super::Dim::Overworld);
                        assert_eq!(game.transactions.len(), 3);
                        assert_eq!(block_in(game, super::Dim::Overworld), gold);
                        assert_eq!(block_in(game, super::Dim::Nether), iron);
                        Ok::<(), ()>(())
                    });
                    assert!(two_deep_commit.is_ok());
                    assert_eq!(calls.load(Ordering::SeqCst), 9);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), gold);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // 10. Abandoned raw Nether root opened during `flush_native_sounds` outside
                    // any Overworld transaction: `abort_open_transactions` from Overworld must
                    // close the Nether frame in the Nether and keep Overworld untouched.
                    game.world.set(pos[0], pos[1], pos[2], diamond).unwrap();
                    tx_mode.store(3, Ordering::SeqCst);
                    assert!(game.level_play_sound(
                        nether,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None,
                    ));
                    game.flush_native_sounds();
                    assert_eq!(calls.load(Ordering::SeqCst), 10);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_eq!(game.transactions.len(), 1);
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);
                    game.abort_open_transactions();
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), netherrack);

                    // 11. Explicit cross-level commit of a Nether seal while Overworld is active:
                    // commits in the Nether, publishes Nether retirement, and restores Overworld.
                    let nether_seal = game
                        .with_effect_level(super::Dim::Nether, |g| {
                            let seal = g.transaction_open();
                            assert_eq!(
                                g.level_set_block(
                                    nether,
                                    pos,
                                    emerald,
                                    BlockUpdateFlags::NEIGHBORS
                                ),
                                Ok(true)
                            );
                            seal
                        })
                        .unwrap();
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_eq!(game.transactions.len(), 1);
                    game.transaction_close(&nether_seal, true);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert!(game.transactions.is_empty());
                    assert!(!defers_retirement_in(game, super::Dim::Overworld));
                    assert!(!defers_retirement_in(game, super::Dim::Nether));
                    assert_eq!(block_in(game, super::Dim::Overworld), diamond);
                    assert_eq!(block_in(game, super::Dim::Nether), emerald);
                });
            });
        },
    );
}

/// Regression for cross-level rollback of player-owned attachments: `Holder::Player(uuid)` is
/// backed by one per-player attachment map regardless of which dimension is swapped in. Writing an
/// attachment on the acting player during an Overworld guest turn and then again from a Nether
/// sound listener (directly or via a nested transaction merged into the Overworld parent) must
/// keep a single original snapshot per player UUID so abort deterministically restores the
/// original value, commit keeps the final write, and rolling back after `as_guest` exits restores
/// the parked guest's map even when the write happened while swapped into the Nether.
#[test]
#[ignore = "daily"]
fn cross_level_sound_listener_player_attachment_writes_rollback_to_original_and_preserve_nested_merge()
 {
    in_isolated_process(
        "cross_level_sound_listener_player_attachment_writes_rollback_to_original_and_preserve_nested_merge",
        || {
            use mod_api::{Flow, HostSide, ServerHost};
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            use sunrise_core::nbt::{Compound, Tag};
            use sunrise_mod_runtime::attachments::{
                AttachmentCodec, AttachmentError, AttachmentType, Holder,
            };

            const ORIGINAL: i32 = 10;
            const TENTATIVE_OVERWORLD: i32 = 20;
            const TENTATIVE_NETHER: i32 = 30;
            const MARK: AttachmentType<i32> = AttachmentType {
                namespace: "cross_level_sound_test",
                name: "mark",
                default: || 0,
                codec: Some(AttachmentCodec {
                    encode: |value| Ok(Compound::from([("n".into(), Tag::Int(*value))])),
                    decode: |data| match data.get(&"n".into()) {
                        Some(Tag::Int(n)) => Ok(*n),
                        _ => Err(AttachmentError::Decode),
                    },
                }),
                copy_on_death: true,
                sync: false,
            };

            struct NetherPlayerAttachmentHook {
                player_uuid: u128,
                expect_before: Arc<std::sync::atomic::AtomicI32>,
                nested_tx: Arc<AtomicBool>,
                calls: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for NetherPlayerAttachmentHook {
                fn register(
                    &self,
                    r: &mut dyn mod_api::Registrar,
                    _: &mod_api::RegistrationContext,
                ) {
                    r.register_attachment_type("cross_level_sound_test:mark", false)
                        .unwrap();
                }

                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    if e.level.dimension_id() != "minecraft:the_nether" {
                        return Flow::Continue;
                    }
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if let HostSide::Server(server) = host {
                        let holder = Holder::Player(self.player_uuid);
                        assert_eq!(
                            MARK.get_existing(server, holder),
                            Ok(Some(self.expect_before.load(Ordering::SeqCst))),
                            "Nether listener reaches the same live player-owned attachment map"
                        );
                        let write = |server: &mut dyn ServerHost| {
                            MARK.set(server, holder, TENTATIVE_NETHER).unwrap();
                        };
                        if self.nested_tx.load(Ordering::SeqCst) {
                            let merged = mod_api::run_transaction(server, |server, _| {
                                write(server);
                                Ok::<(), ()>(())
                            });
                            assert!(merged.is_ok());
                        } else {
                            write(server);
                        }
                    }
                    Flow::Continue
                }
            }

            let expect_before = Arc::new(std::sync::atomic::AtomicI32::new(TENTATIVE_OVERWORLD));
            let nested_tx = Arc::new(AtomicBool::new(false));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods = vec![mod_api::ModEntry::new(
                "cross_level_sound_test",
                Box::new(NetherPlayerAttachmentHook {
                    player_uuid: 12,
                    expect_before: Arc::clone(&expect_before),
                    nested_tx: Arc::clone(&nested_tx),
                    calls: Arc::clone(&calls),
                }),
            )]
            .into();

            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    // Load Nether once so it sits in `levels.parked` while Overworld is active.
                    game.enter_level(super::Dim::Nether).unwrap();
                    game.enter_level(super::Dim::Overworld).unwrap();

                    let nether = game.level("minecraft:the_nether").unwrap();
                    let at = [3.0, 64.0, 5.0];
                    let bell = "minecraft:block.bell.use";
                    let holder = Holder::Player(12);

                    {
                        let host: &mut dyn ServerHost = game;
                        MARK.set(host, holder, ORIGINAL).unwrap();
                        assert_eq!(MARK.get_existing(host, holder), Ok(Some(ORIGINAL)));
                    }

                    game.as_guest(12, |game| {
                        // 1. Direct write in Overworld guest turn + write in Nether sound
                        // listener + abort: repeat across multiple fresh HashMap instances so
                        // hash-order randomization cannot hide duplicate-dimension snapshots.
                        for round in 0..16 {
                            let aborted = mod_api::run_transaction(game, |game, _| {
                                {
                                    let host: &mut dyn ServerHost = game;
                                    MARK.set(host, holder, TENTATIVE_OVERWORLD).unwrap();
                                }
                                assert!(game.level_play_sound(
                                    nether,
                                    bell,
                                    at,
                                    SoundSource::Blocks,
                                    1.0,
                                    1.0,
                                    None,
                                ));
                                assert_eq!(game.dim, super::Dim::Overworld);
                                {
                                    let host: &mut dyn ServerHost = game;
                                    assert_eq!(
                                        MARK.get_existing(host, holder),
                                        Ok(Some(TENTATIVE_NETHER))
                                    );
                                }
                                Err::<(), ()>(())
                            });
                            assert!(aborted.is_err());
                            let host: &mut dyn ServerHost = game;
                            assert_eq!(
                                MARK.get_existing(host, holder),
                                Ok(Some(ORIGINAL)),
                                "round {round}: direct cross-level abort must restore ORIGINAL"
                            );
                        }
                        assert_eq!(calls.load(Ordering::SeqCst), 16);

                        // 2. Nested-merge variant: Overworld write, then Nether listener commits a
                        // nested transaction writing TENTATIVE_NETHER, then Overworld parent
                        // aborts -> parent keeps its ORIGINAL snapshot through nested merge.
                        nested_tx.store(true, Ordering::SeqCst);
                        for round in 0..16 {
                            let aborted = mod_api::run_transaction(game, |game, _| {
                                {
                                    let host: &mut dyn ServerHost = game;
                                    MARK.set(host, holder, TENTATIVE_OVERWORLD).unwrap();
                                }
                                assert!(game.level_play_sound(
                                    nether,
                                    bell,
                                    at,
                                    SoundSource::Blocks,
                                    1.0,
                                    1.0,
                                    None,
                                ));
                                {
                                    let host: &mut dyn ServerHost = game;
                                    assert_eq!(
                                        MARK.get_existing(host, holder),
                                        Ok(Some(TENTATIVE_NETHER))
                                    );
                                }
                                Err::<(), ()>(())
                            });
                            assert!(aborted.is_err());
                            let host: &mut dyn ServerHost = game;
                            assert_eq!(
                                MARK.get_existing(host, holder),
                                Ok(Some(ORIGINAL)),
                                "round {round}: nested-merge cross-level abort must restore ORIGINAL"
                            );
                        }
                        assert_eq!(calls.load(Ordering::SeqCst), 32);

                        // 3. Nested-merge commit persists the final Nether write.
                        let committed = mod_api::run_transaction(game, |game, _| {
                            {
                                let host: &mut dyn ServerHost = game;
                                MARK.set(host, holder, TENTATIVE_OVERWORLD).unwrap();
                            }
                            assert!(game.level_play_sound(
                                nether,
                                bell,
                                at,
                                SoundSource::Blocks,
                                1.0,
                                1.0,
                                None,
                            ));
                            Ok::<(), ()>(())
                        });
                        assert!(committed.is_ok());
                        assert_eq!(calls.load(Ordering::SeqCst), 33);
                        {
                            let host: &mut dyn ServerHost = game;
                            assert_eq!(
                                MARK.get_existing(host, holder),
                                Ok(Some(TENTATIVE_NETHER))
                            );
                        }
                    })
                    .expect("guest 12 turn runs");

                    // 4. Outer transaction opened outside `as_guest`, written only from a Nether
                    // listener during `as_guest(12)`, and aborted after `as_guest(12)` exits:
                    // rollback still restores the parked guest's attachment map.
                    {
                        let host: &mut dyn ServerHost = game;
                        assert_eq!(MARK.get_existing(host, holder), Ok(Some(TENTATIVE_NETHER)));
                        MARK.set(host, holder, ORIGINAL).unwrap();
                    }
                    expect_before.store(ORIGINAL, Ordering::SeqCst);
                    let parked_abort = mod_api::run_transaction(game, |game, _| {
                        game.as_guest(12, |game| {
                            assert!(game.level_play_sound(
                                nether,
                                bell,
                                at,
                                SoundSource::Blocks,
                                1.0,
                                1.0,
                                None,
                            ));
                        })
                        .expect("guest 12 turn runs");
                        Err::<(), ()>(())
                    });
                    assert!(parked_abort.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 34);
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(
                        MARK.get_existing(host, holder),
                        Ok(Some(ORIGINAL)),
                        "parked guest attachment restores even when first written in Nether"
                    );
                });
            });
        },
    );
}

/// Same-UUID fake players in the Overworld and the Nether own separate bodies (`Guest::conn`):
/// writing attachment, health, and inventory on the Nether fake only (directly in the Nether,
/// through a nested committed transaction, or from a Nether sound listener during an Overworld
/// transaction) and aborting must restore the Nether fake's original state without touching the
/// earlier Overworld fake sharing the same UUID.
#[test]
#[ignore = "daily"]
fn same_uuid_fake_players_across_levels_rollback_attachment_health_and_inventory_independently() {
    in_isolated_process(
        "same_uuid_fake_players_across_levels_rollback_attachment_health_and_inventory_independently",
        || {
            use mod_api::{Flow, HostSide, ItemPlace, ItemResource, LivingRef, ServerHost};
            use std::sync::Mutex;
            use std::sync::atomic::{AtomicUsize, Ordering};
            use sunrise_client::items::ItemStack;
            use sunrise_core::nbt::{Compound, Tag};
            use sunrise_mod_runtime::attachments::{
                AttachmentCodec, AttachmentError, AttachmentType, Holder,
            };

            const FAKE_UUID: u128 = 2345;
            const OW_MARK: i32 = 11;
            const OW_HEALTH: f32 = 14.0;
            const NETHER_MARK: i32 = 22;
            const NETHER_HEALTH: f32 = 18.0;

            const MARK: AttachmentType<i32> = AttachmentType {
                namespace: "cross_level_fake_test",
                name: "mark",
                default: || 0,
                codec: Some(AttachmentCodec {
                    encode: |value| Ok(Compound::from([("n".into(), Tag::Int(*value))])),
                    decode: |data| match data.get(&"n".into()) {
                        Some(Tag::Int(n)) => Ok(*n),
                        _ => Err(AttachmentError::Decode),
                    },
                }),
                copy_on_death: false,
                sync: false,
            };

            #[derive(Clone, Copy)]
            struct FakeWritePlan {
                living: LivingRef,
                mark: i32,
                health: f32,
                item: &'static str,
                count: u32,
                nested: bool,
            }

            fn item_resource(id: &str) -> ItemResource {
                ItemResource::of(&crate::game::api_values::test_snapshot(&ItemStack::new(
                    id, 1,
                )))
            }

            fn replace_fake_slot0(
                server: &mut dyn ServerHost,
                tx: &mod_api::Transaction,
                item: &str,
                count: u32,
            ) {
                let place = ItemPlace::PlayerSlot {
                    player: FAKE_UUID,
                    slot: 0,
                };
                if let Some((res, existing_count)) = place
                    .with_stack(server, |existing| {
                        existing
                            .project()
                            .map(|snap| (ItemResource::of(&snap), snap.count()))
                    })
                    .flatten()
                {
                    assert_eq!(
                        place.extract(server, tx, &res, existing_count),
                        existing_count
                    );
                }
                let next = item_resource(item);
                assert_eq!(place.insert(server, tx, &next, count), count);
            }

            struct NetherFakeWriteHook {
                plan: Arc<Mutex<Option<FakeWritePlan>>>,
                calls: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for NetherFakeWriteHook {
                fn register(
                    &self,
                    r: &mut dyn mod_api::Registrar,
                    _: &mod_api::RegistrationContext,
                ) {
                    r.register_attachment_type("cross_level_fake_test:mark", false)
                        .unwrap();
                }

                fn play_level_sound_at_position(
                    &self,
                    host: HostSide<'_>,
                    e: &mut mod_api::PlayLevelSoundAtPositionEvent,
                ) -> Flow {
                    if e.level.dimension_id() != "minecraft:the_nether" {
                        return Flow::Continue;
                    }
                    let Some(plan) = *self.plan.lock().unwrap() else {
                        return Flow::Continue;
                    };
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    if let HostSide::Server(server) = host {
                        let apply = |server: &mut dyn ServerHost, tx: &mod_api::Transaction| {
                            MARK.set(server, Holder::Player(FAKE_UUID), plan.mark)
                                .unwrap();
                            replace_fake_slot0(server, tx, plan.item, plan.count);
                            assert!(plan.living.set_health(server, plan.health));
                        };
                        if plan.nested {
                            let merged = mod_api::run_transaction(server, |server, tx| {
                                apply(server, tx);
                                Ok::<(), ()>(())
                            });
                            assert!(merged.is_ok());
                        } else {
                            let tx = server.transaction_current().expect("active transaction");
                            apply(server, &tx);
                        }
                    }
                    Flow::Continue
                }
            }

            let plan = Arc::new(Mutex::new(None));
            let calls = Arc::new(AtomicUsize::new(0));
            let mods = vec![mod_api::ModEntry::new(
                "cross_level_fake_test",
                Box::new(NetherFakeWriteHook {
                    plan: Arc::clone(&plan),
                    calls: Arc::clone(&calls),
                }),
            )]
            .into();

            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let holder = Holder::Player(FAKE_UUID);
                    let place = ItemPlace::PlayerSlot {
                        player: FAKE_UUID,
                        slot: 0,
                    };

                    // Create Overworld fake and seed distinct attachment, health, and slot 0.
                    let overworld = game.level("minecraft:overworld").unwrap();
                    let ow_fake = game.fake_player(overworld, FAKE_UUID, "parked").unwrap();
                    let ow_living = LivingRef::new(ow_fake.entity());
                    {
                        let host: &mut dyn ServerHost = game;
                        MARK.set(host, holder, OW_MARK).unwrap();
                        assert!(ow_living.set_health(host, OW_HEALTH));
                        let seeded = mod_api::run_transaction(host, |host, tx| {
                            replace_fake_slot0(host, tx, "minecraft:apple", 2);
                            Ok::<(), ()>(())
                        });
                        assert!(seeded.is_ok());
                    }

                    // Enter Nether and create a second fake with the EXACT same UUID.
                    assert!(game.enter_command_level(super::Dim::Nether).is_some());
                    let nether = game.level("minecraft:the_nether").unwrap();
                    let nether_fake = game.fake_player(nether, FAKE_UUID, "parked").unwrap();
                    assert_ne!(ow_fake, nether_fake);
                    let nether_living = LivingRef::new(nether_fake.entity());
                    {
                        let host: &mut dyn ServerHost = game;
                        MARK.set(host, holder, NETHER_MARK).unwrap();
                        assert!(nether_living.set_health(host, NETHER_HEALTH));
                        let seeded = mod_api::run_transaction(host, |host, tx| {
                            replace_fake_slot0(host, tx, "minecraft:emerald", 5);
                            Ok::<(), ()>(())
                        });
                        assert!(seeded.is_ok());
                    }

                    let ow_conn = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == ow_fake.entity().id())
                        .unwrap()
                        .conn;
                    let nether_conn = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == nether_fake.entity().id())
                        .unwrap()
                        .conn;
                    assert_ne!(ow_conn, nether_conn);
                    {
                        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
                        let ow_i = guests.connection_index(ow_conn).unwrap();
                        guests.update(ow_i, |g| {
                            g.ender_items[0] = Some(ItemStack::new("minecraft:diamond", 1));
                        });
                        let nether_i = guests.connection_index(nether_conn).unwrap();
                        guests.update(nether_i, |g| {
                            g.ender_items[0] = Some(ItemStack::new("minecraft:gold_ingot", 3));
                        });
                    }

                    let assert_fake_state =
                        |game: &mut super::Game<'_>,
                         conn: u32,
                         living: LivingRef,
                         expected_mark: i32,
                         expected_health: f32,
                         expected_item: &str,
                         expected_count: u32,
                         label: &str| {
                            let mark = {
                                let map = game
                                    .lan
                                    .as_mut()
                                    .unwrap()
                                    .hub
                                    .guests
                                    .attachments_mut(conn)
                                    .unwrap();
                                MARK.get_existing(map, Holder::Level)
                            };
                            assert_eq!(mark, Ok(Some(expected_mark)), "{label}: attachment");
                            assert_eq!(
                                living.health(game),
                                Some(expected_health),
                                "{label}: health"
                            );
                            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                            let slot = guest.inventory[0].as_ref().expect("slot 0 occupied");
                            assert_eq!(slot.item.as_ref(), expected_item, "{label}: slot 0 item");
                            assert_eq!(slot.count, expected_count, "{label}: slot 0 count");
                            let (expected_ender, expected_ender_count) = if conn == ow_conn {
                                ("minecraft:diamond", 1)
                            } else {
                                ("minecraft:gold_ingot", 3)
                            };
                            let ender = guest.ender_items[0]
                                .as_ref()
                                .expect("ender slot 0 occupied");
                            assert_eq!(
                                ender.item.as_ref(),
                                expected_ender,
                                "{label}: ender slot 0 item"
                            );
                            assert_eq!(
                                ender.count, expected_ender_count,
                                "{label}: ender slot 0 count"
                            );
                        };

                    // 1. Direct write to Nether fake only while in Nether, then abort.
                    {
                        let host: &mut dyn ServerHost = game;
                        let aborted: Result<(), ()> = mod_api::run_transaction(host, |host, tx| {
                            MARK.set(host, holder, 99).unwrap();
                            assert!(nether_living.set_health(host, 4.0));
                            replace_fake_slot0(host, tx, "minecraft:stick", 1);
                            assert_eq!(MARK.get_existing(host, holder), Ok(Some(99)));
                            assert_eq!(nether_living.health(host), Some(4.0));
                            assert_eq!(place.with_stack(host, |s| s.count()), Some(1));
                            Err(())
                        });
                        assert!(aborted.is_err());
                    }
                    assert_fake_state(
                        game,
                        ow_conn,
                        ow_living,
                        OW_MARK,
                        OW_HEALTH,
                        "minecraft:apple",
                        2,
                        "direct in-Nether abort (Overworld fake)",
                    );
                    assert_fake_state(
                        game,
                        nether_conn,
                        nether_living,
                        NETHER_MARK,
                        NETHER_HEALTH,
                        "minecraft:emerald",
                        5,
                        "direct in-Nether abort (Nether fake)",
                    );

                    // 2. Nested-merge abort in Nether: outer writes Nether fake, inner commits a
                    // second write (including zero health), then outer aborts.
                    {
                        let host: &mut dyn ServerHost = game;
                        let aborted: Result<(), ()> = mod_api::run_transaction(host, |host, tx| {
                            MARK.set(host, holder, 50).unwrap();
                            assert!(nether_living.set_health(host, 12.0));
                            replace_fake_slot0(host, tx, "minecraft:coal", 3);
                            let inner = mod_api::run_transaction(host, |host, inner_tx| {
                                MARK.set(host, holder, 99).unwrap();
                                replace_fake_slot0(host, inner_tx, "minecraft:stick", 1);
                                assert!(nether_living.set_health(host, 0.0));
                                Ok::<(), ()>(())
                            });
                            assert!(inner.is_ok());
                            assert_eq!(MARK.get_existing(host, holder), Ok(Some(99)));
                            assert_eq!(nether_living.health(host), Some(0.0));
                            Err(())
                        });
                        assert!(aborted.is_err());
                    }
                    assert_fake_state(
                        game,
                        ow_conn,
                        ow_living,
                        OW_MARK,
                        OW_HEALTH,
                        "minecraft:apple",
                        2,
                        "nested-merge in-Nether abort (Overworld fake)",
                    );
                    assert_fake_state(
                        game,
                        nether_conn,
                        nether_living,
                        NETHER_MARK,
                        NETHER_HEALTH,
                        "minecraft:emerald",
                        5,
                        "nested-merge in-Nether abort (Nether fake)",
                    );

                    // 3. Cross-level abort from Overworld: Nether sound listener writes attachment,
                    // health, and inventory on the Nether fake only (direct + nested-merge), then
                    // the outer Overworld transaction aborts while `self.dim == Overworld`.
                    game.enter_level(super::Dim::Overworld).unwrap();
                    let at = [3.0, 64.0, 5.0];
                    let bell = "minecraft:block.bell.use";

                    *plan.lock().unwrap() = Some(FakeWritePlan {
                        living: nether_living,
                        mark: 88,
                        health: 6.0,
                        item: "minecraft:stick",
                        count: 1,
                        nested: false,
                    });
                    let cross_direct = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        Err::<(), ()>(())
                    });
                    assert!(cross_direct.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_fake_state(
                        game,
                        ow_conn,
                        ow_living,
                        OW_MARK,
                        OW_HEALTH,
                        "minecraft:apple",
                        2,
                        "cross-level direct abort (Overworld fake)",
                    );
                    assert_fake_state(
                        game,
                        nether_conn,
                        nether_living,
                        NETHER_MARK,
                        NETHER_HEALTH,
                        "minecraft:emerald",
                        5,
                        "cross-level direct abort (Nether fake)",
                    );

                    // Cross-level nested-merge abort: first listener write direct, second listener
                    // write inside a committed nested transaction (including zero health), then
                    // outer Overworld transaction aborts.
                    let cross_nested = mod_api::run_transaction(game, |game, _| {
                        *plan.lock().unwrap() = Some(FakeWritePlan {
                            living: nether_living,
                            mark: 55,
                            health: 10.0,
                            item: "minecraft:coal",
                            count: 3,
                            nested: false,
                        });
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        *plan.lock().unwrap() = Some(FakeWritePlan {
                            living: nether_living,
                            mark: 99,
                            health: 0.0,
                            item: "minecraft:stick",
                            count: 1,
                            nested: true,
                        });
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        Err::<(), ()>(())
                    });
                    assert!(cross_nested.is_err());
                    assert_eq!(calls.load(Ordering::SeqCst), 3);
                    assert_eq!(game.dim, super::Dim::Overworld);
                    assert_fake_state(
                        game,
                        ow_conn,
                        ow_living,
                        OW_MARK,
                        OW_HEALTH,
                        "minecraft:apple",
                        2,
                        "cross-level nested-merge abort (Overworld fake)",
                    );
                    assert_fake_state(
                        game,
                        nether_conn,
                        nether_living,
                        NETHER_MARK,
                        NETHER_HEALTH,
                        "minecraft:emerald",
                        5,
                        "cross-level nested-merge abort (Nether fake)",
                    );

                    // Cross-level nested-merge commit: persists only on the Nether fake while
                    // leaving the Overworld fake intact.
                    *plan.lock().unwrap() = Some(FakeWritePlan {
                        living: nether_living,
                        mark: 77,
                        health: 9.0,
                        item: "minecraft:stick",
                        count: 4,
                        nested: true,
                    });
                    let cross_commit = mod_api::run_transaction(game, |game, _| {
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None,
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(cross_commit.is_ok());
                    assert_eq!(calls.load(Ordering::SeqCst), 4);
                    assert_fake_state(
                        game,
                        ow_conn,
                        ow_living,
                        OW_MARK,
                        OW_HEALTH,
                        "minecraft:apple",
                        2,
                        "cross-level nested-merge commit (Overworld fake)",
                    );
                    assert_fake_state(
                        game,
                        nether_conn,
                        nether_living,
                        77,
                        9.0,
                        "minecraft:stick",
                        4,
                        "cross-level nested-merge commit (Nether fake)",
                    );
                });
            });
        },
    );
}
