//! `set_local_rotation`: a client mod turns the local player's view; the look goes out in the
//! tick's own player sync, one move packet with vanilla's clamp, never a packet of the call's own.
use crate::game::native_client_session::tests::{finish_login, logged_client};
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::LocalRotationError;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::connection::Connection;
use sunrise_protocol::packets::play::Serverbound as S;

/// The move packets the client sent before a fence, as (kind, yaw, pitch).
fn sent_moves(
    game: &crate::game::Game<'_>,
    server: &mut Connection,
) -> Vec<(&'static str, f32, f32)> {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut moves = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::MovePlayerPosRot(p) => moves.push(("pos_rot", p.yaw, p.pitch)),
            S::MovePlayerRot(p) => moves.push(("rot", p.yaw, p.pitch)),
            S::MovePlayerPos(_) => moves.push(("pos", f32::NAN, f32::NAN)),
            S::MovePlayerStatus(_) => moves.push(("status", f32::NAN, f32::NAN)),
            S::Punch(_) => return moves,
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_mod_set_look_is_clamped_and_sent_once_by_the_tick() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            assert_eq!(
                game.set_local_rotation(10.0, 5.0),
                Err(LocalRotationError::NoWorld)
            );
            let mut server = Connection::local(logged_client(game));
            server
                .reader
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut server);
            game.title = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            // The baseline: the first sync sends the position.
            game.remote_send_state(0);
            sent_moves(game, &mut server);

            // Yaw stays unwrapped, pitch is clamped; the call itself sends nothing.
            assert_eq!(game.set_local_rotation(450.0, 120.0), Ok(()));
            assert_eq!((game.yaw, game.pitch), (450.0, 90.0));
            assert!(sent_moves(game, &mut server).is_empty());
            for (yaw, pitch) in [
                (f32::NAN, 0.0),
                (0.0, f32::INFINITY),
                (f32::NEG_INFINITY, 0.0),
            ] {
                assert_eq!(
                    game.set_local_rotation(yaw, pitch),
                    Err(LocalRotationError::NotFinite)
                );
                assert_eq!((game.yaw, game.pitch), (450.0, 90.0), "unchanged");
            }
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", 450.0, 90.0)]);

            // Two calls in one tick: one packet, the last values.
            game.set_local_rotation(30.0, -10.0).unwrap();
            game.set_local_rotation(-200.0, 45.0).unwrap();
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", -200.0, 45.0)]);
            game.remote_send_state(0);
            assert!(sent_moves(game, &mut server).is_empty(), "nothing new");

            // Riding: the passenger's rotation-only packet.
            game.riding = Some(99);
            game.set_local_rotation(12.0, 3.0).unwrap();
            game.remote_send_state(0);
            assert_eq!(sent_moves(game, &mut server), [("rot", 12.0, 3.0)]);
            game.riding = None;

            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                game.set_local_rotation(0.0, 0.0),
                Err(LocalRotationError::NoWorld)
            );
        });
    });
}

/// How many packets of any kind the client sent before a fence.
fn sent_packets(game: &crate::game::Game<'_>, server: &mut Connection) -> usize {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut count = 0;
    loop {
        let (id, payload) = server.recv().unwrap();
        if matches!(S::decode(id, &payload).unwrap(), S::Punch(_)) {
            return count;
        }
        count += 1;
    }
}

/// `LocalPlayer.tickDeath` removes the player at deathTime 20: from then on nothing goes out, not
/// a changed input or look, until the respawn's new player (deathTime back at 0).
#[test]
#[ignore = "daily"]
fn a_player_dead_for_twenty_ticks_sends_nothing_until_it_respawns() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let mut server = Connection::local(logged_client(game));
            server
                .reader
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut server);
            game.title = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.remote_send_state(0);
            sent_packets(game, &mut server);

            game.screen = crate::game::Screen::Dead;
            game.ui.death_time = 19;
            game.yaw = 10.0;
            game.remote_send_state(1);
            assert!(sent_packets(game, &mut server) > 0, "still sent at 19");

            game.ui.death_time = 20;
            for call in 0..25 {
                game.yaw = 20.0 + call as f32;
                game.remote_send_state(call as u8 & 1);
                assert_eq!(sent_packets(game, &mut server), 0, "call {call}");
            }

            // The respawn's new player starts its death clock again.
            game.screen = crate::game::Screen::Playing;
            game.ui.death_time = 0;
            game.yaw = 90.0;
            game.remote_send_state(1);
            assert!(sent_packets(game, &mut server) > 0, "sent after respawn");
        });
    });
}
