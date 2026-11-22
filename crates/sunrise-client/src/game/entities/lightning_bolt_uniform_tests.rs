//! A summoned bolt reaches a shader pack's `lightningBoltPosition` through the frame's bolts
//! (`bolt_instances`, Sunrise's draw list): the last in the list, camera-relative, w = 1.
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::gfx::pack_uniforms::lightning_bolt_position;

#[test]
#[ignore = "daily"]
fn a_summoned_bolt_reaches_the_pack_in_player_space() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        let camera = glam::DVec3::new(0.25, 70.5, 0.75);
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [0.0; 4]
        );
        assert!(game.run_command("summon minecraft:lightning_bolt 10.5 64 -5.25"));
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [10.25, -6.5, -6.0, 1.0]
        );
        // A bolt past every draw distance (`shouldRenderAtSqrDistance`) isn't in the draw
        // list, so the listed one still counts; one summoned in range goes last and takes over.
        assert!(game.run_command("summon minecraft:lightning_bolt 1000.5 64 0.5"));
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [10.25, -6.5, -6.0, 1.0]
        );
        assert!(game.run_command("summon minecraft:lightning_bolt -3.5 66 2.75"));
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [-3.75, -4.5, 2.0, 1.0]
        );
    });
}
