use super::*;
fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn files() -> FileToIdConverter {
    FileToIdConverter::json("machines").unwrap()
}

#[test]
fn registry_views_and_explicit_order_before_vanilla() {
    let mut listeners = ClientListenerRegistry::new();
    let model_manager = VanillaClientListeners::model_manager();
    assert_eq!(
        listeners.get_registry(),
        std::slice::from_ref(&model_manager)
    );
    assert!(listeners.get_graph().is_empty());

    let first = Arc::new(Boundary);
    let second = Arc::new(Boundary);
    let after = Arc::new(Boundary);
    listeners
        .add_listener(id("test_mod:first"), files(), first)
        .unwrap();
    listeners
        .add_listener(id("test_mod:second"), files(), second)
        .unwrap();
    listeners
        .add_listener(id("test_mod:after"), files(), after)
        .unwrap();
    assert_eq!(listeners.get_last_vanilla_listener(), model_manager);
    assert_eq!(
        listeners.get_registry(),
        [
            model_manager.clone(),
            id("test_mod:first"),
            id("test_mod:second"),
            id("test_mod:after"),
        ]
    );
    listeners
        .add_dependency(
            id("test_mod:second"),
            VanillaClientListeners::model_manager(),
        )
        .unwrap();
    listeners
        .add_dependency(
            VanillaClientListeners::model_manager(),
            id("test_mod:after"),
        )
        .unwrap();
    assert_eq!(
        listeners.get_graph(),
        [
            (id("test_mod:second"), model_manager.clone()),
            (model_manager.clone(), id("test_mod:after")),
        ]
    );
    let sorted = listeners.finish().unwrap();
    assert_eq!(
        sorted.into_iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [
            id("test_mod:second"),
            model_manager,
            id("test_mod:first"),
            id("test_mod:after"),
        ]
    );
}

#[test]
#[ignore = "daily"]
fn reload_registration_reports_each_refusal_without_latching_but_a_cycle_fails() {
    // Install the process-wide logger only in an exact-selected child.
    const TEST: &str = "client::client_reload::tests::reload_registration_reports_each_refusal_without_latching_but_a_cycle_fails";
    const CHILD: &str = "SUNRISE_RELOAD_REFUSAL_DIAGNOSTIC_TEST";
    if !std::env::var_os(CHILD).is_some_and(|value| value == TEST) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([TEST, "--exact", "--ignored"])
            .env(CHILD, TEST)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{TEST} did not pass in its own process:\n{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    thread_local! {
        static WARNINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    struct RefusalLog;
    impl log::Log for RefusalLog {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.level() == log::Level::Warn
                && record
                    .args()
                    .to_string()
                    .contains("mod test_mod hook add_client_reload_listeners: unknown reload listener test_mod:repeated_missing")
            {
                WARNINGS.set(WARNINGS.get() + 1);
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&RefusalLog).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let model = VanillaClientListeners::model_manager();
    let missing = id("test_mod:repeated_missing");
    let valid = id("test_mod:valid");
    let mut listeners = ClientListenerRegistry::new();
    let _owner = crate::dispatch_scope::ModGuard::fixture_owner(crate::content::crash::ModCall {
        id: "test_mod",
        hook: "add_client_reload_listeners",
        part: None,
    });
    for _ in 0..2 {
        assert_eq!(
            listeners.add_dependency(missing.clone(), model.clone()),
            Err(ReloadOrderError::UnknownListener {
                id: missing.clone()
            })
        );
    }
    assert_eq!(WARNINGS.get(), 2);
    listeners
        .add_listener(valid.clone(), files(), Arc::new(Boundary))
        .unwrap();
    listeners
        .add_dependency(model.clone(), valid.clone())
        .unwrap();
    let ordered = listeners.finish().unwrap();
    let mod_entry = ordered.iter().find(|entry| entry.id == valid).unwrap();
    let apply = mod_entry
        .listener
        .prepare(&ClientReloadContext::new(ReloadFiles::new(
            Default::default(),
        )))
        .unwrap();
    apply();

    let mut cyclic = ClientListenerRegistry::new();
    cyclic
        .add_listener(valid.clone(), files(), Arc::new(Boundary))
        .unwrap();
    cyclic.add_dependency(model.clone(), valid.clone()).unwrap();
    cyclic.add_dependency(valid, model).unwrap();
    assert!(matches!(
        cyclic.finish(),
        Err(ReloadOrderError::Cycle { .. })
    ));
}

#[test]
fn duplicate_objects_cannot_make_the_reverse_lookup_ambiguous() {
    let mut listeners = ClientListenerRegistry::new();
    let listener = Arc::new(Boundary);
    listeners
        .add_listener(id("one:listener"), files(), listener.clone())
        .unwrap();
    assert_eq!(
        listeners.add_listener(id("two:listener"), files(), listener),
        Err(ReloadOrderError::DuplicateObject {
            id: id("two:listener"),
            registered_as: id("one:listener"),
        })
    );
}

#[test]
fn registration_panic_discards_its_listeners_and_edges_before_later_subscribers() {
    struct Context;
    impl crate::ClientReloadListenerContext for Context {
        fn math_profile(&self) -> crate::MathProfile {
            crate::MathProfile::HotSpotX86_64
        }
    }

    struct Broken;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Broken {
        fn add_client_reload_listeners(
            &self,
            _: &dyn crate::ClientReloadListenerContext,
            event: &mut crate::AddClientReloadListenersEvent<'_>,
        ) {
            let key = id("registration_probe:discarded");
            event
                .listeners
                .add_listener(key.clone(), files(), Arc::new(Boundary))
                .unwrap();
            event
                .listeners
                .add_dependency(key, VanillaClientListeners::model_manager())
                .unwrap();
            panic!("registration probe");
        }
    }
    struct Healthy;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Healthy {
        fn add_client_reload_listeners(
            &self,
            _: &dyn crate::ClientReloadListenerContext,
            event: &mut crate::AddClientReloadListenersEvent<'_>,
        ) {
            assert!(event.listeners.get_graph().is_empty());
            event
                .listeners
                .add_listener(id("registration_probe:kept"), files(), Arc::new(Boundary))
                .unwrap();
        }
    }
    crate::testing::with_dispatch(
        vec![
            crate::ModEntry::new("registration_probe", Box::new(Broken)),
            crate::ModEntry::new("registration_healthy", Box::new(Healthy)),
        ]
        .into(),
        || {
            for load in 0..2 {
                let mut registry = ClientListenerRegistry::new();
                crate::dispatch_add_client_reload_listeners(&Context, |listen| {
                    registry.register_listener(listen)
                });
                assert!(registry.get_graph().is_empty());
                assert_eq!(
                    registry.get_registry(),
                    [
                        VanillaClientListeners::model_manager(),
                        id("registration_probe:kept")
                    ]
                );
                let reports = crate::__private::take_hook_panics();
                if load == 0 {
                    assert_eq!(
                        reports,
                        [crate::__private::HookPanicReport {
                            mod_id: "registration_probe",
                            hook: "add_client_reload_listeners",
                        }]
                    );
                } else {
                    assert!(reports.is_empty());
                }
            }
        },
    );
}
