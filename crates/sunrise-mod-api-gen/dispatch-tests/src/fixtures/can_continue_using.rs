/// A may-still-use part: records the stacks, hand and answer it was given, then answers its own.
struct ContinueProbe(bool, Arc<Mutex<Vec<(u32, u32, Option<i32>, bool)>>>);

impl ItemPart for ContinueProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::CanContinueUsing)]
    }
    fn can_continue_using(&self, _: &dyn Host, event: &mut CanContinueUsingEvent<'_>) {
        assert!(matches!(event.hand, Hand::Off));
        self.1.lock().unwrap().push((
            event.stack.count(),
            event.held.count(),
            event.entity,
            event.can_continue,
        ));
        event.can_continue = self.0;
    }
}

#[test]
fn continue_parts_start_from_vanillas_answer_and_answer_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [true, false]
            .map(|answer| leak(ContinueProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stick = |count| ItemStack {
        item: "fixture:stick".into(),
        count,
    };
    let mut can_continue = false;
    dispatch_can_continue_using(
        &TestHost,
        &stick(3),
        &stick(2),
        None,
        Hand::Off,
        &mut can_continue,
    );
    // The first part turns vanilla's `false` into `true`; the second is given
    // that, and its own `false` is the last word.
    assert!(!can_continue);
    assert_eq!(
        *heard.lock().unwrap(),
        [(3, 2, None, false), (3, 2, None, true)]
    );
    install_parts(Parts::default());
}
