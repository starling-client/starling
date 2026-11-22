//! `LocalPlayer.moveTowardsClosestSpace`: the push out of a wall the player's body is in.
pub use sunrise_client_gameplay::closest_space::closest_space_nudge;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use sunrise_client::motion::PlayerMotion;
    use sunrise_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn a_player_nudges_out_of_a_rotated_collider_as_from_stone() {
        use crate::game::{Game, in_own_process, native_entity_refs::tests::with_client_game};
        use sunrise_client_render::moving_colliders::{Placed, Prepared, in_level};
        use sunrise_core::geometry::{
            Aabb,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        };

        in_own_process(
            "game::closest_space::tests::a_player_nudges_out_of_a_rotated_collider_as_from_stone",
            || {
                with_client_game(|game| {
                    const PLAYER: u32 = 42;
                    let chunk = sunrise_client::chunk::Chunk::empty(0, 0);
                    game.world.chunks.insert((0, 0), Arc::new(chunk));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    for y in 64..=65 {
                        for (x, z) in [(2, 5), (3, 6)] {
                            game.world.set(x, y, z, stone).unwrap();
                        }
                    }
                    let mut body =
                        PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5)).unwrap();
                    body.entity_id = PLAYER as i32;
                    let nudge = |game: &Game<'_>| {
                        super::closest_space_nudge(
                            &body,
                            &game.world,
                            game.collision,
                            game.level_suffocation(),
                        )
                    };
                    let boxes = Arc::new(ColliderBoxes::new(vec![Aabb::new(
                        Vec3::new(-1.0, 0.0, -0.5),
                        Vec3::new(1.0, 2.0, 0.5),
                    )]));
                    let placed = |x, exempt| {
                        let half = std::f64::consts::FRAC_1_SQRT_2;
                        Prepared::new(vec![Placed {
                            id: 80,
                            // A quarter turn covers the player's column and the north exit.
                            collider: OrientedCollider::new(
                                Frame::from_quaternion(
                                    Vec3::new(x, 64.0, 5.0),
                                    [0.0, half, 0.0, half],
                                ),
                                boxes.clone(),
                            ),
                            exempt,
                        }])
                    };
                    let turned = in_level(placed(3.5, Vec::new()), || nudge(game));
                    assert_eq!(in_level(placed(8.5, Vec::new()), || nudge(game)), [None; 2]);
                    assert_eq!(
                        in_level(placed(3.5, vec![PLAYER]), || nudge(game)),
                        [None; 2]
                    );
                    for y in 64..=65 {
                        for z in [4, 5] {
                            game.world.set(3, y, z, stone).unwrap();
                        }
                    }
                    let from_stone = nudge(game);
                    assert_eq!(from_stone, [Some(0.1), None]);
                    assert_eq!(turned, from_stone);
                });
            },
        );
    }

    /// `suffocatesAt` counts what `isSuffocating` counts, not only cubes that render solid: a
    /// body in ice or a closed shulker box, walled in on three sides, is pushed out 0.1 a tick to
    /// the free east; one in glass (`isSuffocating` never) is not.
    #[test]
    #[ignore = "daily"]
    fn a_player_is_pushed_out_of_ice_and_a_closed_shulker_box_but_not_glass() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut chunk = sunrise_client::chunk::Chunk::empty(0, 0);
            chunk.lit = true;
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            for (block, want) in [
                ("minecraft:ice", [Some(0.1), None]),
                ("minecraft:shulker_box", [Some(0.1), None]),
                ("minecraft:glass", [None, None]),
            ] {
                let state = registry.default_state(block).unwrap();
                for y in 64..=65 {
                    game.world.set(3, y, 5, state).unwrap();
                    for (x, z) in [(2, 5), (3, 4), (3, 6)] {
                        game.world.set(x, y, z, stone).unwrap();
                    }
                }
                let body = PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5)).unwrap();
                assert_eq!(
                    super::closest_space_nudge(
                        &body,
                        &game.world,
                        game.collision,
                        game.level_suffocation(),
                    ),
                    want,
                    "{block}"
                );
            }
        });
    }
}
