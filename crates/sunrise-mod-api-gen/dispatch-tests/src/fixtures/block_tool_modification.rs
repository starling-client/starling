mod stubs {
    pub(super) struct ToolUse;
}

#[test]
fn invalid_tool_state_does_not_discard_the_veto_or_reach_the_next_listener() {
    struct Refuse;
    #[mod_api::__subscriptions]
    impl Hooks for Refuse {
        fn block_tool_modification(
            &self,
            _: &dyn Host,
            event: &mut BlockToolModificationEvent<'_>,
        ) -> Flow {
            *event.final_state = Some(BlockState::new(BlockKey::Test, u16::MAX));
            Flow::Cancel
        }
    }
    struct Observe(Arc<Mutex<Option<(bool, Option<BlockState>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        #[listener(receive_cancelled)]
        fn block_tool_modification(
            &self,
            _: &dyn Host,
            event: &mut BlockToolModificationEvent<'_>,
        ) -> Flow {
            *self.0.lock().unwrap() = Some((event.cancelled, *event.final_state));
            Flow::Continue
        }
    }
    let seen = Arc::new(Mutex::new(None));
    install(
        vec![
            ModEntry::new("refuse", Box::new(Refuse)),
            ModEntry::new("observe", Box::new(Observe(seen.clone()))),
        ]
        .into(),
    );
    let original = Some(BlockState::new(BlockKey::Test, 0));
    let mut state = original;
    let mut cancel = false;
    let mut rejected = 0;
    dispatch_block_tool_modification(
        &TestHost,
        &ItemStack::default(),
        &ToolUse,
        &mut state,
        &mut cancel,
        &mut |value| {
            rejected += 1;
            assert_eq!(value.unwrap().ordinal(), u16::MAX);
            Err("unknown state".into())
        },
    );
    assert_eq!(rejected, 1);
    assert!(cancel);
    assert_eq!(state, original);
    assert_eq!(*seen.lock().unwrap(), Some((true, original)));
}
