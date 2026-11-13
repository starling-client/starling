//! A summoned drowned (`summon minecraft:drowned x y z`, no NBT, so
//! `Drowned.finalizeSpawn` runs) holds a nautilus shell in its offhand with a
//! 0.03 chance, as a guaranteed drop (`setGuaranteedDrop(OFFHAND)`). Over 400
//! drowned, the live offhand, the saved offhand and the saved chance agree,
//! and the count is in the original probe's wide range (1..30), not an exact
//! random result.
use crate::game::*;
fn game(resources: &mut Option<std::sync::Arc<Resources>>) -> Game<'_> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    // The fixture does not start streaming: make every forced column resident.
    for x in -1..=0 {
        for z in -1..=0 {
            game.world.chunks.insert(
                (x, z),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(x, z)),
            );
        }
    }
    for command in [
        "difficulty easy",
        "gamerule spawn_mobs false",
        "forceload add -16 -16 15 15",
        "fill -16 99 -16 15 99 15 stone",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn summoned_drowned_hold_shells_as_guaranteed_drops() {
    let mut game_resources = None;
    let mut game = game(&mut game_resources);
    for index in 0..400 {
        let (x, z) = (f64::from(index % 20) - 9.5, f64::from(index / 20) - 9.5);
        assert!(game.run_command(&format!("summon drowned {x} 100 {z}")));
    }
    let drowned: Vec<u32> = game
        .mobs
        .iter()
        .filter(|m| m.species() == horizon_client::mobs::Species::Drowned)
        .map(|m| m.id)
        .collect();
    assert_eq!(drowned.len(), 400);
    let (mut live, mut saved, mut guaranteed, mut offhands) = (0, 0, 0, Vec::new());
    for id in drowned {
        let m = game.mobs.iter().find(|m| m.id == id).unwrap();
        offhands.push(m.extra.off_hand_id().map(str::to_owned));
        if m.extra.off_hand_id() == Some("minecraft:nautilus_shell") {
            live += 1;
            if horizon_client::mobs::drops::drop_chance(m, "offhand") == 2.0 {
                guaranteed += 1;
            }
        }
        let tag = game
            .command_target_nbt(crate::game::commands::Target::Mob(id))
            .unwrap();
        let body = horizon_client::snbt::pretty(&tag).plain();
        if body.contains("offhand: {count: 1, id: \"minecraft:nautilus_shell\"}") {
            saved += 1;
        }
    }
    let other: Vec<_> = offhands.iter().flatten().collect();
    eprintln!("live {live} saved {saved} guaranteed {guaranteed}; other offhands {other:?}");
    assert!((1..=30).contains(&live), "live shells {live}");
    assert_eq!(saved, live, "the saved offhand");
    assert_eq!(guaranteed, live, "every shell is a guaranteed drop");
}
