use sunrise_world::worldgen::feature_sort::{
    FeatureIdentity, FeatureSortError, build_features_per_step,
};

#[test]
fn long_dependency_chains_use_bounded_iterative_traversal() {
    let features = (0..50000).map(FeatureIdentity).collect::<Vec<_>>();
    let result = build_features_per_step(&[vec![features.clone()]]).unwrap();
    assert_eq!(result[0].features(), features);
    assert_eq!(result[0].index(FeatureIdentity(49999)), Some(49999));
    assert_eq!(result[0].index(FeatureIdentity(50000)), None);
}

#[test]
fn opposite_biome_orders_and_repeated_identity_form_cycles() {
    let a = FeatureIdentity(91);
    let b = FeatureIdentity(7);
    assert!(matches!(
        build_features_per_step(&[vec![vec![a, b]], vec![vec![b, a]]]),
        Err(FeatureSortError::Cycle)
    ));
    assert!(matches!(
        build_features_per_step(&[vec![vec![a, a]]]),
        Err(FeatureSortError::Cycle)
    ));
    let result = build_features_per_step(&[vec![vec![a], vec![], vec![a, b]]]).unwrap();
    assert_eq!(result[0].features(), [a]);
    assert!(result[1].features().is_empty());
    assert_eq!(result[2].features(), [a, b]);
}

#[test]
fn many_biomes_order_within_raised_limit() {
    const BIOMES: usize = 650;
    const STEPS: usize = 11;
    const FEATURES: usize = 9;
    let biomes = (0..BIOMES)
        .map(|biome| {
            (0..STEPS)
                .map(|step| {
                    (0..FEATURES)
                        .map(|slot| {
                            FeatureIdentity(
                                (biome * STEPS * FEATURES + step * FEATURES + slot) as u32,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let result = build_features_per_step(&biomes).unwrap();
    assert_eq!(result.len(), STEPS);
    assert_eq!(result[0].features().len(), BIOMES * FEATURES);
    assert_eq!(result[10].features().len(), BIOMES * FEATURES);
    assert_eq!(
        result[0].features()[0],
        FeatureIdentity(((BIOMES - 1) * STEPS * FEATURES) as u32)
    );
    assert_eq!(
        result[0].features()[FEATURES - 1],
        FeatureIdentity(((BIOMES - 1) * STEPS * FEATURES + FEATURES - 1) as u32)
    );
    assert_eq!(
        result[0].features()[FEATURES],
        FeatureIdentity(((BIOMES - 2) * STEPS * FEATURES) as u32)
    );
    assert_eq!(
        result[10].features().last().copied(),
        Some(FeatureIdentity(98))
    );
}

#[test]
fn replacement_cap_still_rejects_excessive_input() {
    assert!(matches!(
        build_features_per_step(&[vec![vec![FeatureIdentity(0); 1_048_577]]]),
        Err(FeatureSortError::WorkLimit)
    ));
}
