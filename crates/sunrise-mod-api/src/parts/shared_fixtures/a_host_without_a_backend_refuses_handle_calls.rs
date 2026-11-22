#[test]
pub(crate) fn a_host_without_a_backend_refuses_handle_calls() {
    // NoHost implements nothing, so Host::backend keeps its None default:
    // every handle answers its typed refusal, never a no-op success.
    let level = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD);
    assert!(!level.in_place(&NoHost));
    let mut calls = 0;
    let entity = EntityRef::new(WorldEpoch::new(1), 1);
    assert_eq!(
        entity.piston_push_reaction(&NoHost),
        Err(LevelReadError::Unavailable)
    );
    assert_eq!(entity.is_hanging(&NoHost), Err(LevelReadError::Unavailable));
    assert_eq!(
        level.visit_entities_in(&NoHost, [0.0; 3], [1.0; 3], &mut |_| {
            calls += 1;
            core::ops::ControlFlow::Continue(())
        }),
        Err(crate::LevelReadError::Unavailable)
    );
    assert_eq!(calls, 0);
}
