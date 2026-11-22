//! A client mod's writes to the player it moves itself, before the tick's own move.
//! Nothing is sent; the server hears of the result in the normal position report.
use super::Game;
use mod_api::{LevelWriteError, PlayerRef};
use sunrise_core::geometry::Vec3;
use sunrise_world::entity_move::MoverType;

impl Game<'_> {
    /// `Entity.setDeltaMovement` on the local player. `false`, with the velocity unchanged,
    /// with no player of its own and for a value that is not finite.
    pub(super) fn set_local_player_velocity(&mut self, velocity: [f64; 3]) -> bool {
        let velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        match self.player.as_mut() {
            Some(body) if velocity.is_finite() => {
                body.velocity = velocity;
                true
            }
            _ => false,
        }
    }

    /// The local player's on-ground flag, which this tick's jump decision reads and its move
    /// then sets again from the collision. `false` with no player of its own.
    pub(super) fn set_local_player_on_ground(&mut self, on_ground: bool) -> bool {
        match self.player.as_mut() {
            Some(body) => {
                body.ground.on_ground = on_ground;
                true
            }
            None => false,
        }
    }

    /// Whether the local player can move itself rather than being held in place.
    pub(in crate::game) fn can_move_local_player(&self) -> bool {
        self.riding.is_none() && self.sleeping.is_none() && self.screen != super::Screen::Dead
    }

    pub(super) fn player_reset_fall_distance(
        &mut self,
        player: PlayerRef,
    ) -> Result<bool, LevelWriteError> {
        if self.native_local_status(player).is_none() {
            return Ok(false);
        }
        let Some(player) = self.player.as_mut() else {
            return Ok(false);
        };
        player.fall_distance = 0.0;
        Ok(true)
    }

    pub(super) fn move_local_player_by(
        &mut self,
        delta: [f64; 3],
    ) -> Result<Option<[f64; 3]>, LevelWriteError> {
        if self.player.is_none()
            || !self.can_move_local_player()
            || !sunrise_client::move_gate::valid_mod_move_delta(delta)
        {
            return Ok(None);
        }
        let [x, y, z] = delta;
        self.in_this_levels_colliders(|game| {
            game.move_local_player(MoverType::SelfMovement, Vec3::new(x, y, z), false)
        })
        .map(|moved| moved.map(Vec3::components))
        .map_err(|_| LevelWriteError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::Game;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::HostSide;
    use std::sync::{Arc, Mutex};

    /// Writes a velocity and a ground flag before the local player's tick, and records what
    /// the tick left of them.
    struct Boots {
        velocity: [f64; 3],
        saw: Arc<Mutex<Vec<(Vec<bool>, [f64; 3])>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Boots {
        fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
            let HostSide::Client(host) = host else {
                return;
            };
            if host.local_player() != Some(event.player) {
                return;
            }
            let wrote = vec![
                host.set_local_player_velocity(self.velocity),
                host.set_local_player_on_ground(true),
                host.set_local_player_velocity([f64::NAN, 0.0, 0.0]),
            ];
            self.saw.lock().unwrap().push((wrote, self.velocity));
        }
    }

    /// A client of a server whose local player is loaded, in the air over an empty chunk.
    fn loaded_client(game: &mut Game<'_>) {
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "writes").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.profile = 456;
        remote.entity_id = 42;
        remote.loaded_sent = true;
        let mut player = sunrise_client::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap();
        player.entity_id = remote.entity_id;
        game.player = Some(player);
        game.streaming
            .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(0, 0));
    }

    #[test]
    #[ignore = "daily"]
    fn a_velocity_written_before_the_tick_is_what_the_tick_moves_by() {
        let saw = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "boots",
            Box::new(Boots {
                velocity: [0.25, 0.0, 0.0],
                saw: Arc::clone(&saw),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                loaded_client(game);
                // The write itself sets the flag, either way, before any tick can overwrite it.
                for flag in [true, false, true] {
                    game.player.as_mut().unwrap().ground.on_ground = !flag;
                    assert!(game.set_local_player_on_ground(flag));
                    assert_eq!(game.player.as_ref().unwrap().ground.on_ground, flag);
                }
                let before = game.player.as_ref().unwrap().position;
                game.tick_remote().unwrap();
                let body = game.player.as_ref().unwrap();
                let heard = saw.lock().unwrap();
                assert_eq!(heard.len(), 1, "the hook fires once a tick");
                assert_eq!(
                    heard[0].0,
                    [true, true, false],
                    "a finite velocity and the flag are written, a NaN is refused"
                );
                let moved = body.position.x - before.x;
                assert!((moved - 0.25).abs() < 1.0e-9, "moved {moved}");
                assert!(!body.ground.on_ground, "the move set the flag from the air");
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_local_displacement_stops_at_a_wall_and_reports_how_far_it_moved() {
        struct Push(Arc<Mutex<Option<[f64; 3]>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Push {
            fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
                if let HostSide::Client(host) = host
                    && host.local_player() == Some(event.player)
                {
                    *self.0.lock().unwrap() = host.move_local_player_by([3.0, 0.0, 0.0]).unwrap();
                }
            }
        }

        let moved = Arc::new(Mutex::new(None));
        let mods = vec![mod_api::ModEntry::new(
            "push",
            Box::new(Push(Arc::clone(&moved))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                loaded_client(game);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for y in 63..67 {
                    game.world.set(2, y, 0, stone).unwrap();
                }
                game.tick_remote().unwrap();
                // Standing players are 0.6 blocks wide. Their right edge stops at x = 2.
                let stopped_at = 2.0 - f64::from(0.6_f32 / 2.0);
                assert_eq!(*moved.lock().unwrap(), Some([stopped_at - 0.5, 0.0, 0.0]));
                assert_eq!(game.player.as_ref().unwrap().position.x, stopped_at);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resetting_a_local_fall_discards_the_distance_before_landing() {
        use crate::game::native_entity_refs::tests::with_client_game;
        use mod_api::ClientHost;
        use sunrise_client::{
            landing::{FallEvent, SceneSound},
            motion::MotionEvent,
        };

        with_client_game(|game| {
            loaded_client(game);
            let local = game.local_player().unwrap();
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(local.entity().epoch().get() + 1),
                    local.entity().id(),
                ),
                local.uuid(),
            );
            assert_eq!(stale.reset_fall_distance(game), Ok(false));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 57, 0, stone).unwrap();

            for reset in [true, false] {
                let player = game.player.as_mut().unwrap();
                player.client_side = true;
                player.position.y = 64.0;
                player.ground.on_ground = false;
                player.fall_distance = 0.0;
                player.events.clear();
                let health = player.health.health();
                let host: &mut dyn ClientHost = game;
                assert_eq!(
                    host.move_local_player_by([0.0, -5.0, 0.0]),
                    Ok(Some([0.0, -5.0, 0.0]))
                );
                assert_eq!(game.player.as_ref().unwrap().fall_distance, 5.0);
                if reset {
                    assert_eq!(local.reset_fall_distance(game), Ok(true));
                    assert_eq!(game.player.as_ref().unwrap().fall_distance, 0.0);
                }
                let host: &mut dyn ClientHost = game;
                assert_eq!(
                    host.move_local_player_by([0.0, -2.0, 0.0]),
                    Ok(Some([0.0, -1.0, 0.0]))
                );
                let player = game.player.as_ref().unwrap();
                assert!(player.ground.on_ground);
                assert_eq!(player.health.health(), health);
                if reset {
                    assert!(!player.events.iter().any(|event| {
                        matches!(
                            event,
                            MotionEvent::Fall(
                                FallEvent::Sound { .. } | FallEvent::BlockFallSound { .. }
                            )
                        )
                    }));
                } else {
                    assert!(player.events.iter().any(|event| {
                        matches!(
                            event,
                            MotionEvent::Fall(FallEvent::Sound {
                                sound: SceneSound::SmallFall,
                                ..
                            })
                        )
                    }));
                }
            }
            game.player = None;
            assert_eq!(local.reset_fall_distance(game), Ok(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_local_displacement_respects_collider_exemptions_when_moving_and_sneaking() {
        use sunrise_client_render::moving_colliders::{Placed, Prepared, Scope};
        use sunrise_core::geometry::{
            Aabb, Vec3,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        };

        crate::game::native_entity_refs::tests::with_client_game(|game| {
            loaded_client(game);
            let start = game.player.as_ref().unwrap().position;
            let player_id = game.own_entity_id() as u32;
            let level = game.collider_level().unwrap();
            let mut sweep = |min: Vec3, size: Vec3, sneaking: bool, exempt: bool| {
                let player = game.player.as_mut().unwrap();
                player.position = start;
                player.shift_key_down = sneaking;
                player.ground.on_ground = sneaking;
                let _scope = Scope::enter_level(
                    Some(level),
                    Prepared::new(vec![Placed {
                        id: 900,
                        collider: OrientedCollider::new(
                            Frame::translation(min),
                            Arc::new(ColliderBoxes::new(vec![Aabb::new(Vec3::ZERO, size)])),
                        ),
                        exempt: if exempt {
                            vec![900, player_id]
                        } else {
                            vec![900]
                        },
                    }]),
                );
                game.move_local_player_by([2.0, 0.0, 0.0]).unwrap().unwrap()
            };

            let wall = Vec3::new(1.5, 63.0, 0.0);
            let wall_size = Vec3::new(1.0, 4.0, 1.0);
            assert_eq!(sweep(wall, wall_size, false, true), [2.0, 0.0, 0.0]);
            let to_wall = 1.5 - 0.5 - f64::from(0.6_f32 / 2.0);
            assert_eq!(sweep(wall, wall_size, false, false), [to_wall, 0.0, 0.0]);

            // Only the collider below the destination could provide support. Sneaking
            // leaves the player in place when it doesn't count for this player.
            let platform = Vec3::new(2.0, 63.0, 0.0);
            let platform_size = Vec3::new(1.0, 0.875, 1.0);
            assert_eq!(sweep(platform, platform_size, true, true), [0.0; 3]);
            assert_eq!(sweep(platform, platform_size, true, false), [2.0, 0.0, 0.0]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_displacements_out_and_back_notify_both_section_crossings() {
        type Crossing = ([i32; 3], [i32; 3], [f64; 3], Option<bool>);
        #[derive(Default)]
        struct Walk(Arc<Mutex<Vec<Crossing>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Walk {
            fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
                if let HostSide::Client(host) = host
                    && host.local_player() == Some(event.player)
                {
                    assert!(host.set_local_player_on_ground(true));
                    assert_eq!(
                        host.move_local_player_by([16.0, 0.0, 0.0]),
                        Ok(Some([16.0, 0.0, 0.0]))
                    );
                    assert_eq!(self.0.lock().unwrap().len(), 1);
                    assert_eq!(event.player.entity().on_ground(host), Some(false));
                    assert_eq!(
                        host.move_local_player_by([-16.0, 0.0, 0.0]),
                        Ok(Some([-16.0, 0.0, 0.0]))
                    );
                    assert_eq!(self.0.lock().unwrap().len(), 2);
                }
            }

            fn entity_entering_section(
                &self,
                host: HostSide<'_>,
                event: &mut mod_api::EntityEnteringSectionEvent,
            ) {
                if let HostSide::Client(host) = host
                    && let Some(player) = host.local_player()
                    && player.entity() == event.entity
                {
                    assert_eq!(
                        player.reset_fall_distance(host),
                        Err(mod_api::LevelWriteError::Unavailable),
                    );
                    assert!(host.client_executor().is_none());
                    self.0.lock().unwrap().push((
                        event.old_section,
                        event.new_section,
                        event.entity.position(host).unwrap(),
                        event.entity.on_ground(host),
                    ));
                }
            }
        }

        let walk = Walk::default();
        let crossings = Arc::clone(&walk.0);
        let mods = vec![mod_api::ModEntry::new("walk", Box::new(walk))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                loaded_client(game);
                // Keep the tick's normal gravity away from a section boundary.
                game.player.as_mut().unwrap().position.y = 64.5;
                game.streaming
                    .insert_remote(&mut game.world, sunrise_client::chunk::Chunk::empty(1, 0));
                game.tick_remote().unwrap();
                assert_eq!(
                    *crossings.lock().unwrap(),
                    [
                        ([0, 4, 0], [1, 4, 0], [16.5, 64.5, 0.5], Some(true)),
                        ([1, 4, 0], [0, 4, 0], [0.5, 64.5, 0.5], Some(false))
                    ],
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn with_no_player_of_its_own_both_writes_answer_false() {
        with_game(|game| {
            game.player = None;
            assert!(!game.set_local_player_velocity([0.0; 3]));
            assert!(!game.set_local_player_on_ground(true));
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_collision_flags_distinguish_a_ceiling_from_the_floor_and_a_wall() {
        use crate::game::native_entity_refs::tests::{bind_local_player, with_client_game};
        use sunrise_client::{chunk::Chunk, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_world::entity_move::MoverType;

        let check = |game: &mut Game<'_>| {
            let player = game.player(456).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 66, 0, stone).unwrap();
            game.world.set(2, 64, 0, stone).unwrap();

            let mut move_and_read = |movement| {
                game.move_local_player(MoverType::SelfMovement, movement, false)
                    .unwrap();
                player.collision_flags(game).unwrap().unwrap()
            };
            let ceiling = move_and_read(Vec3::new(0.0, 1.0, 0.0));
            assert!(ceiling.vertical);
            assert!(!ceiling.vertical_below);
            assert!(!ceiling.horizontal);

            let floor = move_and_read(Vec3::new(0.0, -1.0, 0.0));
            assert!(floor.vertical);
            assert!(floor.vertical_below);
            assert!(!floor.horizontal);

            let wall = move_and_read(Vec3::new(3.0, 0.0, 0.0));
            assert!(wall.horizontal);
            assert!(!wall.minor_horizontal);
        };

        with_client_game(|game| {
            loaded_client(game);
            check(game);
        });
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            bind_local_player(game, 42, 456);
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            check(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_displacements_refuse_bad_deltas_and_players_held_in_place() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            loaded_client(game);
            let before = game.player.as_ref().unwrap().position;
            for x in [f64::NAN, f64::INFINITY, -f64::INFINITY, 64.5, -64.5] {
                assert_eq!(game.move_local_player_by([x, 0.0, 0.0]), Ok(None));
            }
            game.riding = Some(43);
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            game.riding = None;
            game.sleeping = Some([0, 64, 0]);
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            game.sleeping = None;
            game.screen = crate::game::Screen::Dead;
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            assert_eq!(game.player.as_ref().unwrap().position, before);
        });
    }
}
