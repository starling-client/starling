//! A client rider's own move runs inside `tick_local_player`: its look is this tick's and its
//! ground and collision flags are what the move left, after the seat is taken.
use super::*;
use crate::game::boats::Boat;
use crate::game::native_entity_refs::tests::with_client_game;
use sunrise_client::motion::PlayerMotion;
use winit::keyboard::KeyCode;

#[test]
#[ignore = "daily"]
fn a_boat_clamps_the_riders_yaw_without_changing_the_previous_yaw() {
    with_client_game(|game| {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "boat-clamp").unwrap();
        game.remote.as_mut().unwrap().first_passenger = false;
        game.boats
            .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 64.0, 2.5), 0.0));
        game.riding = Some(71);
        game.yaw = 200.0;
        game.rope_rot_old.0 = 180.0;

        assert!(game.remote_ride(Keys::default()));
        assert_eq!(game.yaw, 255.0);
        assert_eq!(game.rope_rot_old.0, 180.0);
        server.shutdown();
    });
}

#[test]
#[ignore = "daily"]
fn a_riders_move_takes_the_look_of_the_tick_and_leaves_it_off_the_ground() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "rider-tick").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.loaded_sent = true;
            remote.joined = true;
            remote.first_passenger = true;
            remote.entity_id = 42;
            remote.level_load.finish();
            game.screen = Screen::Playing;
            game.streaming
                .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // A floor under the rider and a wall a hair beyond it at +z.
            for x in 6..=11 {
                for z in 6..=11 {
                    game.world.set(x, 4, z, stone).unwrap();
                }
            }
            for y in 5..=7 {
                game.world.set(8, y, 9, stone).unwrap();
            }
            game.boats
                .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 5.0, 2.5), 0.0));
            game.riding = Some(71);

            let mut collided = Vec::new();
            for yaw in [0.0, 180.0] {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(8.5, 5.0, 8.69)).unwrap();
                player.client_side = true;
                // The player's own copy of the look is the opposite one: only this tick's look,
                // synced before the rider moves, turns it to the wall or away from it.
                player.yaw = 180.0 - yaw;
                game.player = Some(player);
                game.yaw = yaw;
                game.clear_device_keys();
                game.insert_device_key(KeyCode::KeyW);
                game.tick_local_player(Vec::new(), 7).unwrap();
                let player = game.player.as_ref().unwrap();
                collided.push(player.collision.horizontal);
                // `Entity.rideTick` zeroes the rider's motion and `LivingEntity.travelInAir` adds
                // gravity only after the move, so a move over a floor has no downward step to
                // find it: the rider is off the ground, and the seating and the pose tick that
                // follow do not change what the move left.
                assert!(
                    !player.ground.on_ground,
                    "a rider's own move leaves it off the ground, yaw {yaw}"
                );
            }
            assert_eq!(collided, [true, false]);
            server.shutdown();
        });
    });
}
