use super::*;
use mod_api::ConfigKind::{Client, Common, Server};
use mod_api::{
    ConfigListener, ConfigSpec, Hooks, HostSide, ModEntry, Mods, Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};

impl NativeConfigs {
    /// Opens each registration of `kinds` in order and answers the snapshots.
    pub(super) fn load(
        &mut self,
        configs: &'static [RegisteredConfig],
        kinds: &[ConfigKind],
        config_dir: &Path,
        world: Option<&Path>,
    ) -> Vec<(&'static RegisteredConfig, ConfigSnapshot)> {
        self.started = true;
        configs
            .iter()
            .filter(|c| kinds.contains(&c.kind))
            .filter_map(|c| Some((c, self.open(c, config_dir, world)?)))
            .collect()
    }
}

pub(super) type ConfigDecl = (ConfigKind, Option<&'static str>, &'static str);

pub(super) fn with_registered_configs(
    entries: &[(&'static str, &[ConfigDecl])],
    f: impl FnOnce(&'static [RegisteredConfig]),
) {
    struct Decl(Vec<ConfigDecl>);
    #[mod_api::__subscriptions]
    impl Hooks for Decl {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for &(kind, file, default) in &self.0 {
                let mut spec = ConfigSpec::new(kind, default);
                if let Some(file) = file {
                    spec = spec.file(file);
                }
                r.register_config(spec, None).unwrap();
            }
        }
    }
    let mods: Mods = entries
        .iter()
        .map(|&(id, specs)| ModEntry::new(id, Box::new(Decl(specs.to_vec()))))
        .collect::<Vec<_>>()
        .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        f(registered_configs());
    });
}

#[test]
#[ignore = "daily"]
fn panicking_config_hooks_keep_live_writes_and_do_not_stop_later_listeners() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use mod_api::{__private::take_hook_panics, SettingSpec, SettingValue};

    if run_fresh_cases(
        "game::native_configs::tests::panicking_config_hooks_keep_live_writes_and_do_not_stop_later_listeners",
        "SUNRISE_CONFIG_PANIC_CHILD",
        &["server"],
    ) {
        return;
    }

    struct Broken(Arc<Mutex<Vec<(String, SettingValue)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for key in ["reload", "unload"] {
                let setting = r
                    .register_setting(SettingSpec::toggle(key, format!("fault.{key}"), false))
                    .unwrap();
                r.register_config(
                    ConfigSpec::new(Server, "enabled = true").file(format!("fault-{key}.toml")),
                    Some(Arc::new(
                        move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                            let host = match event {
                                ConfigEvent::Reloading(HostSide::Server(host))
                                    if key == "reload" =>
                                {
                                    host
                                }
                                ConfigEvent::Unloading(host) if key == "unload" => host,
                                _ => return Ok(()),
                            };
                            assert_eq!(
                                host.set_setting(setting, SettingValue::Toggle(true)),
                                Ok(true)
                            );
                            panic!("broken {key} listener");
                        },
                    )),
                )
                .unwrap();
            }
        }

        fn setting_changed(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::SettingChangedEvent<'_>,
        ) {
            let _ = host;
            self.0
                .lock()
                .unwrap()
                .push((event.key.to_owned(), event.value));
        }
    }

    struct Later(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Later {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let calls = self.0.clone();
            r.register_config(
                ConfigSpec::new(Server, "enabled = true"),
                Some(Arc::new(
                    move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                        let name = match event {
                            ConfigEvent::Reloading(_) => "reload",
                            ConfigEvent::Unloading(_) => "unload",
                            _ => return Ok(()),
                        };
                        calls.lock().unwrap().push(name);
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let changed = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("fault", Box::new(Broken(changed.clone()))),
        ModEntry::new("later", Box::new(Later(observed.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let _ = take_hook_panics();
            let config_dir = game.config_root.clone();
            let reload = mod_api::__private::setting_key_by_name("fault", "reload").unwrap();
            let unload = mod_api::__private::setting_key_by_name("fault", "unload").unwrap();
            for (text, expected) in [("first", true), ("second", false)] {
                assert_eq!(
                    game.set_setting(reload, SettingValue::Toggle(false)),
                    Ok(!expected)
                );
                assert_eq!(
                    game.set_setting(unload, SettingValue::Toggle(false)),
                    Ok(!expected)
                );
                game.native_configs_server_start(Path::new("test-world"), false);
                fs::write(config_dir.join("fault-reload.toml"), text).unwrap();
                fs::write(config_dir.join("later-server.toml"), text).unwrap();
                game.native_configs.next_check = None;
                game.native_configs_poll();
                game.native_configs_server_stop();
                for (name, key) in [("reload", reload), ("unload", unload)] {
                    assert_eq!(game.setting(key), Ok(SettingValue::Toggle(expected)));
                    assert_eq!(
                        fs::read_to_string(game.native_settings_root().join("fault").join(name))
                            .unwrap(),
                        expected.to_string()
                    );
                }
            }
            assert_eq!(
                *observed.lock().unwrap(),
                ["reload", "unload", "reload", "unload"]
            );
            assert_eq!(
                changed.lock().unwrap().as_slice(),
                [
                    ("reload".to_owned(), SettingValue::Toggle(true)),
                    ("unload".to_owned(), SettingValue::Toggle(true)),
                    ("reload".to_owned(), SettingValue::Toggle(false)),
                    ("unload".to_owned(), SettingValue::Toggle(false)),
                ]
            );
            let notices = take_hook_panics();
            assert_eq!(notices.len(), 2);
            assert!(notices.iter().all(|notice| notice.mod_id == "fault"));
            assert_eq!(
                notices.iter().map(|notice| notice.hook).collect::<Vec<_>>(),
                ["config_reloading", "config_unloading"]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn config_permissions_and_once_per_side_survive_two_worlds() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game, with_game};
    use mod_api::{SettingSpec, SettingValue};

    if run_fresh_cases(
        "game::native_configs::tests::config_permissions_and_once_per_side_survive_two_worlds",
        "SUNRISE_CONFIG_LIFECYCLE_CHILD",
        &["lifecycle"],
    ) {
        return;
    }

    struct Probe(Arc<Mutex<Vec<(bool, ConfigKind)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r
                .register_setting(SettingSpec::toggle(
                    "unloaded",
                    "phase_probe.unloaded",
                    false,
                ))
                .unwrap();
            for kind in [Client, Common, Server] {
                let calls = self.0.clone();
                let listener: Arc<ConfigListener> =
                    Arc::new(move |event: ConfigEvent<'_>, config: &ConfigSnapshot| {
                        match event {
                            ConfigEvent::Loading(context) => {
                                assert_eq!(
                                    mod_api::content::crash::current_native_mod(),
                                    Some("phase_probe")
                                );
                                assert_eq!(context.config_dir(), Game::game_dir().join("config"));
                                assert!(matches!(
                                    context.setting(key),
                                    Ok(SettingValue::Toggle(_))
                                ));
                                calls.lock().unwrap().push((true, config.kind));
                            }
                            ConfigEvent::Unloading(host) => {
                                let SettingValue::Toggle(current) = host.setting(key).unwrap()
                                else {
                                    panic!("expected a toggle")
                                };
                                assert_eq!(
                                    host.set_setting(key, SettingValue::Toggle(!current)),
                                    Ok(true)
                                );
                                calls.lock().unwrap().push((false, config.kind));
                            }
                            _ => {}
                        }
                        Ok(())
                    });
                r.register_config(ConfigSpec::new(kind, "enabled=true"), Some(listener))
                    .unwrap();
            }
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new("phase_probe", Box::new(Probe(calls.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            game.native_configs_client_start();
            game.native_configs_client_start();
        });
        with_game(|game| {
            let key = mod_api::__private::setting_key_by_name("phase_probe", "unloaded").unwrap();
            assert_eq!(game.setting(key), Ok(SettingValue::Toggle(false)));
            for (world, expected) in [("phase-world-one", true), ("phase-world-two", false)] {
                game.native_configs_server_start(Path::new(world), false);
                game.native_configs_server_stop();
                assert_eq!(game.setting(key), Ok(SettingValue::Toggle(expected)));
            }
        });
    });
    assert_eq!(
        *calls.lock().unwrap(),
        [
            (true, Client),
            (true, Common),
            (true, Server),
            (false, Server),
            (true, Server),
            (false, Server)
        ]
    );
}

#[test]
#[ignore = "daily"]
fn owner_bound_config_lifecycle_isolates_owners_refusals_and_panics() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Moment {
        Loading,
        Reloading,
        Unloading,
    }
    fn moment(event: ConfigEvent<'_>) -> Moment {
        match event {
            ConfigEvent::Loading(_) => Moment::Loading,
            ConfigEvent::Reloading(HostSide::Server(_)) => Moment::Reloading,
            ConfigEvent::Unloading(_) => Moment::Unloading,
            _ => panic!("expected server config callback"),
        }
    }
    type TraceRow = (&'static str, Moment, PathBuf, String);

    let tag = format!("p3-25-lifecycle-{}", std::process::id());
    let alpha_first_rel = PathBuf::from(&tag).join("alpha-first.toml");
    let alpha_second_rel = PathBuf::from(&tag).join("alpha-second.toml");
    let beta_rel = PathBuf::from(&tag).join("beta.toml");

    let world_one = Folder::new("daily-w1");
    let world_two = Folder::new("daily-w2");
    let root = Folder::new("owner-config-root");
    let config_dir = root.0.join("config");

    let override_path = world_one.0.join("serverconfig").join(&alpha_second_rel);
    fs::create_dir_all(override_path.parent().unwrap()).unwrap();
    fs::write(&override_path, "second = override\n").unwrap();

    struct Alpha {
        first: PathBuf,
        second: PathBuf,
        trace: Arc<Mutex<Vec<TraceRow>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Alpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let trace = self.trace.clone();
            let first_listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("alpha"));
                    assert_eq!(snapshot.mod_id, "alpha");
                    trace.lock().unwrap().push((
                        "alpha-first",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    if event == Moment::Reloading {
                        return Err("refused reload".into());
                    }
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "alpha = 1\n").file(self.first.clone()),
                Some(first_listener),
            )
            .unwrap();

            let trace = self.trace.clone();
            let second_listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("alpha"));
                    assert_eq!(snapshot.mod_id, "alpha");
                    trace.lock().unwrap().push((
                        "alpha-second",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "second = 1\n").file(self.second.clone()),
                Some(second_listener),
            )
            .unwrap();
        }
    }

    struct Beta {
        file: PathBuf,
        trace: Arc<Mutex<Vec<TraceRow>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Beta {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let trace = self.trace.clone();
            let listener: Arc<ConfigListener> =
                Arc::new(move |event: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                    let event = moment(event);
                    assert_eq!(mod_api::content::crash::current_native_mod(), Some("beta"));
                    assert_eq!(snapshot.mod_id, "beta");
                    trace.lock().unwrap().push((
                        "beta",
                        event,
                        snapshot.path.clone(),
                        snapshot.text.clone(),
                    ));
                    if event == Moment::Reloading {
                        panic!("beta reload boom");
                    }
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(Server, "beta = 1\n").file(self.file.clone()),
                Some(listener),
            )
            .unwrap();
        }
    }

    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        ModEntry::new(
            "alpha",
            Box::new(Alpha {
                first: alpha_first_rel.clone(),
                second: alpha_second_rel.clone(),
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "beta",
            Box::new(Beta {
                file: beta_rel.clone(),
                trace: trace.clone(),
            }),
        ),
    ]
    .into();

    let alpha_first_path = config_dir.join(&alpha_first_rel);
    let alpha_second_base = config_dir.join(&alpha_second_rel);
    let beta_path = config_dir.join(&beta_rel);

    mod_platform::testing::with_mods(mods, || {
        let _ = mod_api::__private::take_hook_panics();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.config_root = config_dir.clone().into();

            // World 1: loads in declaration order; world override wins without writing a base file.
            game.native_configs_server_start(&world_one.0, false);
            assert!(!alpha_second_base.exists());
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Loading,
                        alpha_first_path.clone(),
                        "alpha = 1\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Loading,
                        override_path.clone(),
                        "second = override\n".into(),
                    ),
                    (
                        "beta",
                        Moment::Loading,
                        beta_path.clone(),
                        "beta = 1\n".into(),
                    ),
                ]
            );

            // Consecutive edits whose listener returns Err must keep delivering only to that owner.
            for text in ["alpha = 2\n", "alpha = 3\n"] {
                fs::write(&alpha_first_path, text).unwrap();
                game.native_configs.next_check = None;
                game.native_configs_poll();
                assert_eq!(
                    std::mem::take(&mut *trace.lock().unwrap()),
                    [(
                        "alpha-first",
                        Moment::Reloading,
                        alpha_first_path.clone(),
                        text.into(),
                    )]
                );
            }
            assert!(mod_api::__private::take_hook_panics().is_empty());

            // Editing the world override notifies only its owning registration with the override path.
            fs::write(&override_path, "second = override2\n").unwrap();
            game.native_configs.next_check = None;
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "alpha-second",
                    Moment::Reloading,
                    override_path.clone(),
                    "second = override2\n".into(),
                )]
            );

            // A real panic records a notice naming that mod and disables only that registration.
            fs::write(&beta_path, "beta = 2\n").unwrap();
            game.native_configs.next_check = None;
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "beta",
                    Moment::Reloading,
                    beta_path.clone(),
                    "beta = 2\n".into(),
                )]
            );
            let notices = mod_api::__private::take_hook_panics();
            assert!(
                notices
                    .iter()
                    .any(|notice| { notice.mod_id == "beta" && notice.hook == "config_reloading" }),
                "missing panic notice for beta: {notices:?}"
            );

            // Further edits still reach alpha-first while poisoned beta stays silent.
            fs::write(&beta_path, "beta = 3\n").unwrap();
            fs::write(&alpha_first_path, "alpha = 4\n").unwrap();
            game.native_configs.next_check = None;
            game.native_configs_poll();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [(
                    "alpha-first",
                    Moment::Reloading,
                    alpha_first_path.clone(),
                    "alpha = 4\n".into(),
                )]
            );
            assert!(mod_api::__private::take_hook_panics().is_empty());

            // Unload reaches only live registrations in order.
            game.native_configs_server_stop();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Unloading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Unloading,
                        override_path.clone(),
                        "second = override2\n".into(),
                    ),
                ]
            );

            // World 2 has no override: alpha-second creates its base default, while poisoned beta
            // remains disabled across worlds.
            game.native_configs_server_start(&world_two.0, false);
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Loading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Loading,
                        alpha_second_base.clone(),
                        "second = 1\n".into(),
                    ),
                ]
            );
            game.native_configs_server_stop();
            assert_eq!(
                std::mem::take(&mut *trace.lock().unwrap()),
                [
                    (
                        "alpha-first",
                        Moment::Unloading,
                        alpha_first_path.clone(),
                        "alpha = 4\n".into(),
                    ),
                    (
                        "alpha-second",
                        Moment::Unloading,
                        alpha_second_base.clone(),
                        "second = 1\n".into(),
                    ),
                ]
            );
        });
    });
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-native-configs-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_missing_file_gets_its_default_and_an_edited_one_is_read() {
    let root = Folder::new("default");
    let config = root.0.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("gt-common.toml"), "voltage = 512\n").unwrap();
    with_registered_configs(
        &[(
            "gt",
            &[
                (Client, None, "hud = true\n"),
                (Common, None, "voltage = 32\n"),
                (Server, None, "ores = 1\n"),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Client, Common], &config, None);
            let got: Vec<_> = loaded
                .iter()
                .map(|(_, snapshot)| {
                    (
                        snapshot.file.as_os_str(),
                        snapshot.path.as_path(),
                        snapshot.text.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                got,
                [
                    (
                        Path::new("gt-client.toml").as_os_str(),
                        config.join("gt-client.toml").as_path(),
                        "hud = true\n",
                    ),
                    (
                        Path::new("gt-common.toml").as_os_str(),
                        config.join("gt-common.toml").as_path(),
                        "voltage = 512\n",
                    ),
                ]
            );
            assert_eq!(
                fs::read_to_string(config.join("gt-client.toml")).unwrap(),
                "hud = true\n"
            );
            // Another side's kind is neither read nor written.
            assert!(!config.join("gt-server.toml").exists());
        },
    );
}

#[test]
fn a_world_overrides_a_server_config_and_the_folder_serves_the_rest() {
    let root = Folder::new("world");
    let (config, world) = (root.0.join("config"), root.0.join("saves/W"));
    fs::create_dir_all(world.join("serverconfig/gt")).unwrap();
    fs::write(world.join("serverconfig/gt/ores.toml"), "veins = 9\n").unwrap();
    with_registered_configs(
        &[(
            "gt",
            &[
                (Server, Some("gt/ores.toml"), "veins = 3\n"),
                (Server, None, "tiers = 4\n"),
                // A Common config of the world's folder is not looked for.
                (Common, None, "a = 1\n"),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Server], &config, Some(&world));
            let got: Vec<_> = configs
                .files
                .iter()
                .map(|t| (t.snapshot.path.clone(), t.snapshot.text.as_str()))
                .collect();
            assert_eq!(loaded.len(), 2);
            assert_eq!(
                got,
                [
                    (world.join("serverconfig/gt/ores.toml"), "veins = 9\n"),
                    (config.join("gt-server.toml"), "tiers = 4\n"),
                ]
            );
            // The world's copy stands for the default: nothing is written beside it.
            assert!(!config.join("gt/ores.toml").exists());
            assert_eq!(configs.drop_server().len(), 2);
            assert!(configs.files.is_empty());
        },
    );
}

#[test]
fn registered_configs_load_in_declaration_order_and_keep_io_failures_isolated() {
    let root = Folder::new("order");
    let config = root.0.join("config");
    let broken_world = root.0.join("saves/broken");
    let clean_world = root.0.join("saves/clean");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("z-server.toml"), "fallback = true\n").unwrap();
    // An unreadable world override path (here a directory) must fail on that path,
    // never silently substitute config/z-server.toml or block other configs.
    fs::create_dir_all(broken_world.join("serverconfig/z-server.toml")).unwrap();

    // Resolved dependency order ["lib", "z", "a"] with multiple files under one owner
    // and a native path not prefixed with the mod's id.
    with_registered_configs(
        &[
            (
                "lib",
                &[
                    (Server, Some("shared/first.toml"), "first = 1\n"),
                    (Server, None, "lib = 2\n"),
                ],
            ),
            ("z", &[(Server, None, "z = 3\n")]),
            ("a", &[(Server, None, "a = 4\n")]),
        ],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[Server], &config, Some(&broken_world));
            assert_eq!(
                loaded
                    .iter()
                    .map(|(_, s)| (s.mod_id, s.file.as_os_str(), s.text.as_str()))
                    .collect::<Vec<_>>(),
                [
                    (
                        "lib",
                        Path::new("shared/first.toml").as_os_str(),
                        "first = 1\n",
                    ),
                    ("lib", Path::new("lib-server.toml").as_os_str(), "lib = 2\n"),
                    ("a", Path::new("a-server.toml").as_os_str(), "a = 4\n"),
                ]
            );
            assert_eq!(configs.drop_server().len(), 3);

            // A failed load is not latched: a later world without the broken override loads all.
            let loaded = configs.load(registered, &[Server], &config, Some(&clean_world));
            assert_eq!(
                loaded
                    .iter()
                    .map(|(_, s)| (s.mod_id, s.text.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("lib", "first = 1\n"),
                    ("lib", "lib = 2\n"),
                    ("z", "fallback = true\n"),
                    ("a", "a = 4\n"),
                ]
            );
        },
    );
}

#[test]
fn a_change_reloads_once_and_a_touch_without_one_does_not() {
    let root = Folder::new("reload");
    let config = root.0.join("config");
    with_registered_configs(&[("gt", &[(Common, None, "a = 1\n")])], |registered| {
        let mut configs = NativeConfigs::default();
        configs.load(registered, &[Common], &config, None);
        let path = config.join("gt-common.toml");
        assert!(configs.changed().is_empty());
        fs::write(&path, "a = 2\n").unwrap();
        let changed = configs.changed();
        assert_eq!(
            changed
                .iter()
                .map(|(_, s)| s.text.as_str())
                .collect::<Vec<_>>(),
            ["a = 2\n"]
        );
        assert!(configs.changed().is_empty());
        // Saved again with the same text: nothing to reload.
        fs::write(&path, "a = 2\n").unwrap();
        assert!(configs.changed().is_empty());
        // Same length, written at once after the load, as a mod's own save
        // from `ConfigEvent::Loading` is: still a change.
        fs::write(&path, "a = 3\n").unwrap();
        assert_eq!(configs.changed().len(), 1);
        fs::write(&path, "a = 2\n").unwrap();
        configs.changed();
        // Gone: the last snapshot stays, nothing fires, and recovery is not latched.
        fs::remove_file(&path).unwrap();
        assert!(configs.changed().is_empty());
        assert_eq!(configs.files[0].snapshot.text, "a = 2\n");
        fs::write(&path, "a = 4\n").unwrap();
        let recovered = configs.changed();
        assert_eq!(
            recovered
                .iter()
                .map(|(_, s)| s.text.as_str())
                .collect::<Vec<_>>(),
            ["a = 4\n"]
        );
    });
}

#[test]
fn files_are_looked_at_once_a_second_and_never_without_configs() {
    with_registered_configs(&[("gt", &[(Common, None, "")])], |registered| {
        let mut configs = NativeConfigs::default();
        let start = Instant::now();
        assert!(!configs.due(start));
        let config = &registered[0];
        configs.files.push(Tracked {
            config,
            snapshot: config.snapshot(PathBuf::new(), String::new()),
        });
        assert!(configs.due(start));
        assert!(!configs.due(start + Duration::from_millis(999)));
        assert!(configs.due(start + CHECK_EVERY));
    });
}

/// A registration's initial callback that edits the next registration's file: the next one is
/// read after that edit, as FML's `ConfigTracker.loadConfigs` opens each registration only
/// when its turn comes.
#[test]
#[ignore = "daily"]
fn an_initial_callback_that_edits_the_next_file_is_seen_by_it() {
    use crate::game::native_entity_refs::tests::with_client_game;
    let root = Folder::new("sequential");
    let config = root.0.join("config");
    fs::create_dir_all(&config).unwrap();
    let target = config.join("b-common.toml");
    let seen = Arc::new(Mutex::new(Vec::new()));
    struct First(PathBuf);
    #[mod_api::__subscriptions]
    impl Hooks for First {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let target = self.0.clone();
            r.register_config(
                ConfigSpec::new(Common, "a = 1\n"),
                Some(Arc::new(move |_: ConfigEvent<'_>, _: &ConfigSnapshot| {
                    fs::write(&target, "b = 2\n").unwrap();
                    Ok(())
                })),
            )
            .unwrap();
        }
    }
    struct Second(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Second {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let seen = self.0.clone();
            r.register_config(
                ConfigSpec::new(Common, "b = 1\n"),
                Some(Arc::new(
                    move |_: ConfigEvent<'_>, snapshot: &ConfigSnapshot| {
                        seen.lock().unwrap().push(snapshot.text.clone());
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
    }
    let mods: Mods = vec![
        ModEntry::new("a", Box::new(First(target.clone()))),
        ModEntry::new("b", Box::new(Second(seen.clone()))),
    ]
    .into();
    fs::write(&target, "b = 1\n").unwrap();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
        with_client_game(|game| {
            game.config_root = std::sync::Arc::from(config.as_path());
            game.native_configs = NativeConfigs::default();
            game.native_configs_load(&[Common], None);
        });
    });
    assert_eq!(*seen.lock().unwrap(), ["b = 2\n"]);
}
