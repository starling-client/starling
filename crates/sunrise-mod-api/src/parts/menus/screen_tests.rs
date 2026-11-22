use super::*;

#[test]
fn menu_screen_factories_key_by_menu_type_and_refuse_builtins_and_duplicates() {
    const CUSTOM: &str = "alpha:pack";
    const CROSS_MOD: &str = "alpha:shared";
    const MINECRAFT_CUSTOM: &str = "minecraft:custom_pack";
    let factory = || -> Arc<dyn crate::MenuScreenFactory> {
        Arc::new(|_: &dyn ClientHost, _: crate::MenuScreenOpen<'_>| None)
    };
    let mut registrar = PartRegistrar::new("alpha");
    assert!(registrar.register_menu_screen(CUSTOM, factory()).is_ok());
    assert!(
        registrar
            .register_menu_screen("pack", factory())
            .unwrap_err()
            .contains("namespaced")
    );
    assert!(
        registrar
            .register_menu_screen(CUSTOM, factory())
            .unwrap_err()
            .contains("duplicate")
    );
    for builtin in BUILTIN_MENU_SCREENS {
        let error = registrar
            .register_menu_screen(builtin, factory())
            .unwrap_err();
        assert!(
            error.contains("duplicate") && error.contains(builtin),
            "{error}"
        );
    }

    // Another mod may register a screen for another mod's custom menu or a
    // non-built-in `minecraft:*` menu type, while duplicates still fail.
    registrar.mod_id = "beta";
    registrar.mod_index = 1;
    let error = registrar
        .register_menu_screen(CUSTOM, factory())
        .unwrap_err();
    assert!(error.contains("duplicate") && error.contains("beta") && error.contains(CUSTOM));
    assert!(registrar.register_menu_screen(CROSS_MOD, factory()).is_ok());
    assert!(
        registrar
            .register_menu_screen(MINECRAFT_CUSTOM, factory())
            .is_ok()
    );
    assert!(
        registrar
            .register_menu_screen(MINECRAFT_CUSTOM, factory())
            .unwrap_err()
            .contains("duplicate")
    );
    assert_eq!(registrar.menu_screen_factories[CUSTOM].owner, "alpha");
    assert_eq!(registrar.menu_screen_factories[CROSS_MOD].owner, "beta");
    assert_eq!(
        registrar.menu_screen_factories[MINECRAFT_CUSTOM].owner,
        "beta"
    );
    assert_eq!(registrar.refused.len(), 4 + BUILTIN_MENU_SCREENS.len());

    // Checkpoint rollback restores `menu_screen_factories` on refusal.
    let mut clean = PartRegistrar::new("alpha");
    clean.register_menu_screen(CUSTOM, factory()).unwrap();
    let rollback = clean.registration_checkpoint();
    clean.mod_id = "beta";
    clean.mod_index = 1;
    clean.register_menu_screen(CROSS_MOD, factory()).unwrap();
    rollback(&mut clean);
    crate::testing::with_mods(Vec::new().into(), || {
        let schema = super::super::tests::rule_schema();
        install_registrar(clean, &schema).unwrap();
        assert!(has_menu_screen_factory(CUSTOM));
        assert!(!has_menu_screen_factory(CROSS_MOD));
    });
}

#[test]
fn constructing_a_menu_screen_requires_the_client() {
    use std::sync::Mutex;

    enum ScreenPolicy {
        Factory(Arc<Mutex<Vec<Option<String>>>>),
        NoScreen,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ScreenPolicy {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            if let Self::Factory(openers) = self {
                let openers = openers.clone();
                r.register_menu_screen(
                    "runes:altar",
                    Arc::new(move |_: &dyn ClientHost, open: crate::MenuScreenOpen<'_>| {
                        openers.lock().unwrap().push(open.block.map(str::to_owned));
                        None
                    }),
                )
                .unwrap();
            }
        }
    }
    let openers = Arc::new(Mutex::new(Vec::new()));
    let mods: crate::Mods = vec![
        ModEntry::new("factory", Box::new(ScreenPolicy::Factory(openers.clone()))),
        ModEntry::new("unrelated", Box::new(ScreenPolicy::NoScreen)),
    ]
    .into();
    let served = crate::PackageInfo::new(
        "runes".into(),
        "1".into(),
        crate::PackageKind::Content,
        crate::Side::Both,
        vec!["runes".into()],
        vec![],
    )
    .with_served_by(Some("unrelated".into()));
    crate::load_state::with_test_package_inventory(vec![served], || {
        crate::testing::with_mods(mods.clone(), || {
            crate::__private::part_tests::install_parts_fixture(&mods).unwrap();
            assert_eq!(
                crate::__private::client_requirement("factory").as_deref(),
                Some("registers a client menu screen factory")
            );
            assert_eq!(crate::__private::client_requirement("unrelated"), None);

            let host = crate::testing::FakeHost::new();
            let open = |menu, block| {
                crate::__private::create_menu_screen(
                    &host,
                    crate::MenuScreenOpen {
                        window: 1,
                        menu,
                        block,
                        pos: None,
                        title: None,
                        backing_slots: 1,
                        player_slots: 36,
                        data_slots: 0,
                        extra: &[],
                    },
                )
            };
            // The factory owner differs from the serving package. The opener is context,
            // not a dispatch key: both openings select the same registered factory.
            for block in [Some("runes:altar"), Some("other:table"), None] {
                assert!(matches!(
                    open("runes:altar", block),
                    Some(("factory", Ok(None)))
                ));
            }
            assert!(open("minecraft:hopper", Some("runes:altar")).is_none());
            assert!(open("runes:unregistered", Some("runes:altar")).is_none());
            assert_eq!(
                *openers.lock().unwrap(),
                [Some("runes:altar".into()), Some("other:table".into()), None]
            );
        });
    });
}

#[test]
fn served_content_menu_buttons_dispatch_to_serving_host_only() {
    use std::sync::{Arc, Mutex};

    struct HostPart(Arc<Mutex<Option<&'static str>>>);
    #[mod_api::sunrise_part]
    impl crate::MenuPart for HostPart {
        fn menu_button_valid(
            &self,
            _: &dyn crate::Host,
            _: &mut crate::MenuButtonValidEvent<'_>,
        ) -> crate::Flow {
            *self.0.lock().unwrap() = Some("horizon_java");
            crate::Flow::Continue
        }
    }
    struct ThirdPart(Arc<Mutex<Option<&'static str>>>);
    #[mod_api::sunrise_part]
    impl crate::MenuPart for ThirdPart {
        fn menu_button_valid(
            &self,
            _: &dyn crate::Host,
            _: &mut crate::MenuButtonValidEvent<'_>,
        ) -> crate::Flow {
            *self.0.lock().unwrap() = Some("third");
            crate::Flow::Continue
        }
    }
    struct HostMod {
        seen: Arc<Mutex<Option<&'static str>>>,
    }
    #[mod_api::__subscriptions]
    impl crate::Hooks for HostMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            r.menu_part("runes:altar", HostPart(self.seen.clone()));
            r.menu_part("runes_extra:forge", HostPart(self.seen.clone()));
            r.menu_part("minecraft:hopper", HostPart(self.seen.clone()));
        }
    }
    struct ThirdMod {
        seen: Arc<Mutex<Option<&'static str>>>,
    }
    #[mod_api::__subscriptions]
    impl crate::Hooks for ThirdMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            r.menu_part("runes:altar", ThirdPart(self.seen.clone()));
            r.menu_part("plain:table", ThirdPart(self.seen.clone()));
            r.menu_part("minecraft:hopper", ThirdPart(self.seen.clone()));
        }
    }

    let host = crate::testing::FakeHost::new();
    let seen: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));

    let packages = || {
        vec![
            crate::PackageInfo::new(
                "horizon_java".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["horizon_java".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "third".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Both,
                vec!["third".into()],
                vec![],
            ),
            crate::PackageInfo::new(
                "runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Both,
                vec!["runes".into(), "runes_extra".into()],
                vec![],
            )
            .with_served_by(Some("horizon_java".into())),
            crate::PackageInfo::new(
                "plain".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Both,
                vec!["plain".into()],
                vec![],
            ),
        ]
    };

    let third = || crate::ModEntry::new("third", Box::new(ThirdMod { seen: seen.clone() }));
    // Registration order cannot let the unrelated Part claim the hosted menu's buttons.
    let mods: crate::Mods = vec![
        third(),
        crate::ModEntry::new("horizon_java", Box::new(HostMod { seen: seen.clone() })),
    ]
    .into();
    crate::load_state::with_test_package_inventory(packages(), || {
        crate::testing::with_mods(mods.clone(), || {
            let menu_opener_owner = |menu: &str, block: Option<&str>| -> Option<&'static str> {
                *seen.lock().unwrap() = None;
                let mut accepted = false;
                crate::dispatch_menu_button_valid(&host, menu, block, 1, &mut accepted);
                *seen.lock().unwrap()
            };
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            assert_eq!(menu_opener_owner("runes:altar", None), Some("horizon_java"));
            assert_eq!(
                menu_opener_owner("runes_extra:forge", None),
                Some("horizon_java")
            );
            assert_eq!(
                menu_opener_owner("minecraft:hopper", Some("runes:altar")),
                Some("horizon_java")
            );
            assert_eq!(
                menu_opener_owner("minecraft:hopper", Some("minecraft:hopper")),
                None
            );
            assert_eq!(menu_opener_owner("plain:table", None), None);
            assert_eq!(
                menu_opener_owner("minecraft:hopper", Some("third:block")),
                Some("third")
            );
            // Opener dispatch reads the pre-indexed table without querying the package
            // inventory again: clearing the test package inventory leaves the resolved owners
            // intact.
            crate::load_state::with_test_package_inventory(Vec::new(), || {
                assert_eq!(menu_opener_owner("runes:altar", None), Some("horizon_java"));
                assert_eq!(
                    menu_opener_owner("runes_extra:forge", None),
                    Some("horizon_java")
                );
                assert_eq!(
                    menu_opener_owner("minecraft:hopper", Some("runes:altar")),
                    Some("horizon_java")
                );
                assert_eq!(
                    menu_opener_owner("minecraft:hopper", Some("runes_extra:forge")),
                    Some("horizon_java")
                );
                assert_eq!(
                    menu_opener_owner("minecraft:hopper", Some("minecraft:hopper")),
                    None
                );
                assert_eq!(menu_opener_owner("plain:table", None), None);
            });
        });
    });

    let hijack_mods: crate::Mods = vec![third()].into();
    crate::load_state::with_test_package_inventory(packages(), || {
        crate::testing::with_mods(hijack_mods.clone(), || {
            crate::parts::tests::install_parts_fixture(&hijack_mods).unwrap();
            // A Part attached by a non-owner cannot become the button handler even
            // when the serving mod itself did not register a Part.
            *seen.lock().unwrap() = None;
            let mut accepted = false;
            crate::dispatch_menu_button_valid(&host, "runes:altar", None, 1, &mut accepted);
            assert_eq!(*seen.lock().unwrap(), None);
            assert!(!accepted);
        });
    });
}
