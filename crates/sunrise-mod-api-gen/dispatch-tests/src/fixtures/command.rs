impl LevelTicksHost for TestLevel {}

#[test]
fn disabled_update_shape_owner_admits_the_input_state_to_the_engine() {
    let owner: &dyn BlockPart = leak(Mod {
        id: 0,
        calls: Arc::default(),
    });
    DISABLED_BLOCK_PART.set(Some(std::ptr::from_ref(owner).cast()));
    install_parts(Parts {
        blocks: vec![(BlockKey::Test, vec![owner])],
        owner_slot: Some((BlockKey::Test, 0)),
        ..Parts::default()
    });
    let state = BlockState::new(BlockKey::Test, 1);
    let mut engine_output = None;
    dispatch_update_shape(
        &mut TestLevel,
        calls::UpdateShapeCall {
            dimension: LevelDimension,
            pos: BlockPos,
            state,
            direction: Direction,
            neighbour_pos: BlockPos,
            neighbour: BlockState::new(BlockKey::Other, 0),
            new: &mut None,
        },
        // Like the native crossing, only the accept callback writes the engine output.
        &mut |proposal| {
            assert_eq!(*proposal, Some(state));
            engine_output = Some(BlockStateId);
            Ok(())
        },
    );
    DISABLED_BLOCK_PART.set(None);
    assert_eq!(engine_output, Some(BlockStateId));
}
