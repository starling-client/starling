/// A stop-using part: records the side, stack, hand and time left it heard, under its name.
struct StopProbe(
    &'static str,
    Arc<Mutex<Vec<(&'static str, bool, u32, Option<i32>, i32)>>>,
);

impl ItemPart for StopProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::OnStopUsing)]
    }
    fn on_stop_using(&self, host: HostSide, event: &mut OnStopUsingEvent<'_>) {
        assert!(matches!(event.hand, Hand::Main));
        self.1.lock().unwrap().push((
            self.0,
            matches!(host, HostSide::Server(_)),
            event.stack.count(),
            event.entity,
            event.time_left,
        ));
    }
}

#[test]
fn stop_using_parts_hear_the_stop_in_turn_on_either_side() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: ["first", "second"]
            .map(|name| leak(StopProbe(name, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stick = ItemStack {
        item: "fixture:stick".into(),
        count: 3,
    };
    dispatch_on_stop_using(
        HostSide::Server(&mut TestHost),
        &stick,
        Some(7),
        Hand::Main,
        12,
    );
    dispatch_on_stop_using(HostSide::Client(&mut TestHost), &stick, None, Hand::Main, 0);
    assert_eq!(
        *heard.lock().unwrap(),
        [
            ("first", true, 3, Some(7), 12),
            ("second", true, 3, Some(7), 12),
            ("first", false, 3, None, 0),
            ("second", false, 3, None, 0),
        ]
    );
    install_parts(Parts::default());
}
