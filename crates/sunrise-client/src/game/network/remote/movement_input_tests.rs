//! Exercise the real local-player movement caller with and without a native listener.
use super::*;
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, Hooks, MovementInputUpdateEvent};
use sunrise_client::motion::PlayerMotion;
use winit::keyboard::KeyCode;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn movement_input_update(&self, _: &dyn ClientHost, event: &mut MovementInputUpdateEvent<'_>) {
        let input = &mut *event.input;
        let keys = &mut input.key_presses;
        if keys.right {
            keys.forward = true;
            keys.jump = false;
        } else if keys.left {
            input.move_vector = [0.25, -0.5];
        } else if keys.backward {
            keys.forward = true;
            keys.left = true;
            keys.right = true;
            keys.sprint = true;
            input.move_vector = [0.5, 0.25];
        } else if keys.jump {
            input.move_vector = [f32::MAX, 0.25];
            keys.sneak = true;
        } else if !keys.forward {
            keys.forward = true;
            keys.sprint = true;
            input.move_vector = [0.0, 1.0];
        }
    }
}

fn reset_player(game: &mut Game<'_>, food: i32) {
    let mut player = PlayerMotion::new(game.motion, Vec3::new(8.0, 64.0, 8.0)).unwrap();
    player.client_side = true;
    player.health.food.food = food;
    player.ground.on_ground = true;
    game.player = Some(player);
    game.clear_device_keys();
    game.using = None;
    let remote = game.remote.as_mut().unwrap();
    remote.health = (20.0, food, 5.0);
    remote.was_sprinting = false;
    remote.last_input = 0;
}

fn connect(game: &mut Game<'_>) -> sunrise_protocol::connection::Pipe {
    let (client, server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "movement-client").unwrap();
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.loaded_sent = true;
    remote.entity_id = 42;
    remote.level_load.finish();
    game.screen = Screen::Playing;
    game.streaming
        .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
    server
}

fn tick(game: &mut Game<'_>) {
    if let Some(input) = game.tick_local_player(Vec::new(), 7, None).unwrap() {
        game.remote_send_state(input);
    }
}

#[test]
#[ignore = "daily"]
fn native_movement_input_keys_axes_and_sprint() {
    let mods = vec![mod_api::ModEntry::new("movement", Box::new(Native))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            // Key-only edits don't replace the sampled controller vector.
            let keys = game.update_movement_input(Keys {
                right: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert!(keys.forward && keys.right);
            assert_eq!(keys.direction(), [0.2, 0.8]);

            // Edited vectors (including large ones whose squared length overflows) apply directly
            // without reverting to the sampled vector, alongside independent key edits.
            let keys = game.update_movement_input(Keys {
                jump: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [f32::MAX, 0.25]);
            assert!(keys.jump && keys.shift);

            // Consumer-visible acceptance: no physical input still moves the
            // local player; the final keys and eligible sprint reach the server.
            for food in [0, 6, 7, 20] {
                reset_player(game, food);
                let mut replayed = game.player.as_ref().unwrap().clone();
                tick(game);
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.move_vector, [0.0, 1.0]);
                assert!(
                    player.position.z > 8.0,
                    "injected forward must move, food={food}"
                );
                assert_eq!(player.sprinting, food > 6, "food={food}");
                assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 64);
                assert_eq!(game.remote.as_ref().unwrap().was_sprinting, food > 6);
                // What the client sent is enough to play its tick again: the input packet's keys
                // and the sprint state, through `MotionWorld::replayed_tick` (the tick a mod's
                // `native_simulate_player_move` plays), without asking the listener again, move
                // the player as the client's own tick did.
                let position = player.position;
                let sprinting = player.sprinting;
                let sent_keys =
                    crate::game::lan_vehicles::input_keys(game.remote.as_ref().unwrap().last_input);
                let side = game_side!(game);
                MotionWorld {
                    side: &side,
                    entities: &[],
                    player: &mut replayed,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                }
                .replayed_tick(sent_keys, sprinting)
                .unwrap();
                assert_eq!(replayed.move_vector, [0.0, 1.0]);
                assert_eq!(replayed.sprinting, sprinting);
                assert!((replayed.position.z - position.z).abs() < 1.0e-10);
            }

            // Both edits can stand at once, including all four direction flags being held.
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyS);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.5, 0.25]);
            assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 2 | 4 | 8 | 64);
            // Vector-only edits don't affect what gets sent, even if the vector points backward.
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyA);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.25, -0.5]);
            assert_eq!(game.remote.as_ref().unwrap().last_input, 4);

            // A mod's sprint request still uses the actual player's vanilla
            // crouching and slow-item eligibility, not only the hunger check.
            reset_player(game, 20);
            game.player.as_mut().unwrap().shift_key_down = true;
            game.insert_device_key(KeyCode::ShiftLeft);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.inventory[0] = Some(sunrise_client::items::ItemStack::new("minecraft:apple", 1));
            assert!(game.start_using_item());
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);

            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_passive_listener_preserves_sprint() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct Observe(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn movement_input_update(
            &self,
            _: &dyn ClientHost,
            event: &mut MovementInputUpdateEvent<'_>,
        ) {
            if event.input.key_presses.forward && !event.input.key_presses.sprint {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let saw_released_sprint = Arc::new(AtomicBool::new(false));
    let mods = vec![mod_api::ModEntry::new(
        "observer",
        Box::new(Observe(saw_released_sprint.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            game.insert_device_key(KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting);
            game.remove_device_key(&KeyCode::ControlLeft);
            tick(game);
            assert!(saw_released_sprint.load(Ordering::Relaxed));
            assert!(game.player.as_ref().unwrap().sprinting, "vanilla latch");
            assert_eq!(game.remote.as_ref().unwrap().last_input, 1);
            assert!(game.remote.as_ref().unwrap().was_sprinting);

            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            game.clear_device_keys();
            tick(game);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(
                game.player.as_ref().unwrap().sprinting,
                "vanilla double tap"
            );
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_sends_auto_jump_with_the_edited_keys() {
    let mods = vec![mod_api::ModEntry::new("movement", Box::new(Native))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            // Right selects the listener's edits: add forward and clear jump.
            game.insert_device_key(KeyCode::KeyD);
            game.insert_device_key(KeyCode::Space);
            game.player.as_mut().unwrap().auto_jump_time = 1;
            tick(game);
            let player = game.player.as_ref().unwrap();
            assert!(player.was_auto_jump);
            assert_eq!(player.move_vector, [-1.0, 0.0]);
            assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 8 | 16);
            tick(game);
            assert!(!game.player.as_ref().unwrap().was_auto_jump);
            assert_eq!(game.remote.as_ref().unwrap().last_input, 1 | 8);
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_without_listeners_keeps_vanilla_latch_double_tap_and_controller() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            let keys = game.update_movement_input(Keys {
                forward: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.2, 0.8]);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            game.insert_device_key(KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting);
            game.remove_device_key(&KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting, "vanilla latch");
            game.clear_device_keys();
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            game.clear_device_keys();
            tick(game);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(
                game.player.as_ref().unwrap().sprinting,
                "vanilla double tap"
            );
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_disabled_listener_keeps_vanilla_keys() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("movement", Box::new(Native))].into(),
        || {
            assert!(mod_api::__private::set_mod_enabled("movement", false));
            with_client_game(|game| {
                let keys = game.update_movement_input(Keys {
                    forward: true,
                    analog: Some([0.2, 0.8]),
                    ..Keys::default()
                });
                assert_eq!(keys.direction(), [0.2, 0.8]);
                assert!(!keys.sprint);
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint);
                assert!(mod_api::__private::set_mod_enabled("movement", true));
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 1.0]);
                assert!(keys.forward && keys.sprint);
                assert!(mod_api::__private::set_mod_enabled("movement", false));
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint);
            });
        },
    );
}

#[path = "local_rotation_tests.rs"]
mod local_rotation_tests;

/// LocalPlayer.isShiftKeyDown is the input's sneak while the player rides too, so the
/// sneak held to get off already counts on the first tick out of the vehicle.
#[test]
#[ignore = "daily"]
fn the_sneak_key_counts_while_riding() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            let boat =
                crate::game::boats::Boat::new(5, "oak".into(), Vec3::new(8.0, 64.0, 8.0), 0.0);
            game.boats.push(boat);
            game.riding = Some(5);
            game.remote.as_mut().unwrap().first_passenger = false;
            game.insert_device_key(KeyCode::ShiftLeft);
            tick(game);
            assert_eq!(game.riding, Some(5));
            assert!(game.player.as_ref().unwrap().shift_key_down);
            server.shutdown();
        });
    });
}
