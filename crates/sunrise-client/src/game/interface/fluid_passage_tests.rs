//! `fluid_passage` reaches mods from the server's fluid ticks and closes the sides it answers false for.
use mod_api::{FluidPassageEvent, Hooks, LevelHost, Registrar};
use sunrise_core::position::BlockPos;

/// Closes every horizontal step out of the cells at x 5.
struct Levee;
#[mod_api::__subscriptions]
impl Hooks for Levee {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}
    fn fluid_passage(&self, _: &dyn LevelHost, event: &mut FluidPassageEvent) {
        if event.from.x == 5 && event.to.y == event.from.y {
            event.passes = false;
        }
    }
}

/// Two water sources on a stone floor, at x 5 and x 10, ticked once. Returns whether water
/// reached the cells beside each. Each source has floor for `FlowingFluid.getSpread`'s four-cell
/// search on every side, so with no hole in reach it spreads all four ways.
fn spread_beside_both_sources(game: &mut super::Game<'_>) -> [bool; 2] {
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
    );
    let registry = &game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let source = registry.default_state("minecraft:water").unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    for x in 0..=15 {
        for z in 0..=15 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    for x in [5, 10] {
        game.world.set(x, 64, 5, source).unwrap();
    }
    game.with_level(|behaviours, level, ticks| {
        for x in [5, 10] {
            behaviours.scheduled_fluid_tick(level, ticks, BlockPos::new(x, 64, 5));
        }
    });
    [5, 10].map(|x| game.world.block(x + 1, 64, 5) != air && game.world.block(x - 1, 64, 5) != air)
}

#[test]
#[ignore = "daily"]
fn a_mod_closes_a_side_to_flowing_water() {
    super::native_guest_turn_tests::with_guests(|game| {
        assert_eq!(
            spread_beside_both_sources(game),
            [true, true],
            "no listener: water flows as before"
        );
    });
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("levee", Box::new(Levee))].into();
    mod_platform::testing::with_mods(mods, || {
        super::native_guest_turn_tests::with_guests(|game| {
            assert_eq!(
                spread_beside_both_sources(game),
                [false, true],
                "water leaves the open source and not the closed one"
            );
        });
    });
}
