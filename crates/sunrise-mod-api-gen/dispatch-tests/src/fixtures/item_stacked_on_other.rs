#[test]
fn item_stacked_on_other_dispatches_global_and_item_parts() {
    struct GlobalListener(Arc<Mutex<Vec<&'static str>>>, bool);
    #[__subscriptions]
    impl Hooks for GlobalListener {
        fn item_stacked_on_other(
            &self,
            _: HostSide<'_>,
            event: &mut ItemStackedOnOtherEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push("global");
            if self.1 {
                event.cancelled = true;
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    struct PartListener {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        cancel_flow: bool,
        cancel_flag: bool,
    }
    impl ItemPart for PartListener {
        fn item_subscribed(&self) -> &'static [Option<HookId>] {
            &[hook_ids::item_stacked_on_other]
        }
        fn item_stacked_on_other(
            &self,
            _: HostSide<'_>,
            event: &mut ItemStackedOnOtherEvent<'_>,
        ) -> Flow {
            self.log.lock().unwrap().push(self.label);
            event.cancelled |= self.cancel_flag;
            if self.cancel_flow {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let first: &dyn ItemPart = leak(PartListener {
        label: "part1",
        log: log.clone(),
        cancel_flow: true,
        cancel_flag: false,
    });
    let second: &dyn ItemPart = leak(PartListener {
        label: "part2",
        log: log.clone(),
        cancel_flow: false,
        cancel_flag: false,
    });
    let flag_canceller: &dyn ItemPart = leak(PartListener {
        label: "flag",
        log: log.clone(),
        cancel_flow: false,
        cancel_flag: true,
    });

    // Global listener cancelling skips part dispatch when `*canceled` is already true.
    install(Arc::from([ModEntry::new(
        "g",
        Box::new(GlobalListener(log.clone(), true)),
    )]));
    install_parts(Parts {
        items: vec![first, second],
        ..Parts::default()
    });
    let mut host = TestHost;
    let click = mod_api::mod_api::item::menu::StackedOnOther;
    let mut result = mod_api::mod_api::item::menu::StackedOnOtherResult;
    let mut canceled = false;
    dispatch_item_stacked_on_other(
        HostSide::Server(&mut host),
        &click,
        &mut result,
        &mut canceled,
    );
    assert!(canceled);
    dispatch_item_stacked_on_other_part(
        HostSide::Server(&mut host),
        "test:pouch",
        &click,
        &mut result,
        &mut canceled,
    );
    assert_eq!(*log.lock().unwrap(), ["global"]);

    // Uncancelled global lets parts run and stop at the first Flow::Cancel.
    log.lock().unwrap().clear();
    install(Arc::from([ModEntry::new(
        "g",
        Box::new(GlobalListener(log.clone(), false)),
    )]));
    canceled = false;
    dispatch_item_stacked_on_other(
        HostSide::Server(&mut host),
        &click,
        &mut result,
        &mut canceled,
    );
    assert!(!canceled);
    dispatch_item_stacked_on_other_part(
        HostSide::Server(&mut host),
        "test:pouch",
        &click,
        &mut result,
        &mut canceled,
    );
    assert!(canceled);
    assert_eq!(*log.lock().unwrap(), ["global", "part1"]);

    // Setting `event.cancelled = true` on a part also stops later parts.
    log.lock().unwrap().clear();
    install_parts(Parts {
        items: vec![flag_canceller, second],
        ..Parts::default()
    });
    canceled = false;
    dispatch_item_stacked_on_other_part(
        HostSide::Server(&mut host),
        "test:pouch",
        &click,
        &mut result,
        &mut canceled,
    );
    assert!(canceled);
    assert_eq!(*log.lock().unwrap(), ["flag"]);
}
