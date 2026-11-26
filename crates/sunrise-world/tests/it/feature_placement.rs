use std::convert::Infallible;
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_world::worldgen::{
    anchor::WorldGenerationContext,
    placement::{FeaturePlacer, PlacementModifier},
    value_provider::IntProvider,
};

#[test]
fn long_modifier_chain_completes_in_vanilla_random_order() {
    let mut random = LegacyRandom::new(17);
    let mut expected = random.clone();
    let mut expected_x = 0_i32;
    let mut expected_z = 0_i32;
    for _ in 0..1025 {
        expected_x = expected_x.wrapping_add(expected.next_int_bound(16).unwrap());
        expected_z = expected_z.wrapping_add(expected.next_int_bound(16).unwrap());
    }
    let mut placer = FeaturePlacer::new();
    let mut seen = Vec::new();
    let modifiers = vec![PlacementModifier::InSquare; 1025];
    let result = placer.place(
        &modifiers,
        WorldGenerationContext::new(-64, 384, 63),
        &mut random,
        [0, 0, 0],
        |_, pos| {
            seen.push(pos);
            Ok::<_, Infallible>(true)
        },
    );
    assert!(result.unwrap());
    assert_eq!(seen, [[expected_x, 0, expected_z]]);
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn multiplicative_count_chain_exceeds_former_work_quota() {
    let mut random = LegacyRandom::new(17);
    let mut expected = random.clone();
    let mut placer = FeaturePlacer::new();
    let mut calls = 0_usize;
    let result = placer.place(
        &[
            PlacementModifier::Count(IntProvider::Constant(41)),
            PlacementModifier::Count(IntProvider::Constant(41)),
            PlacementModifier::Count(IntProvider::Constant(41)),
        ],
        WorldGenerationContext::new(-64, 384, 63),
        &mut random,
        [0, 0, 0],
        |_, _| {
            calls += 1;
            Ok::<_, Infallible>(true)
        },
    );
    assert!(result.unwrap());
    assert_eq!(calls, 68_921);
    assert_eq!(random.next_long(), expected.next_long());
}
