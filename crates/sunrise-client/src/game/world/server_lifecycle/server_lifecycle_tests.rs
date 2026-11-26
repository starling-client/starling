use crate::game::{
    Dim,
    execution_role::ExecutionRole,
    native_entity_refs::tests::{run_fresh_cases, with_game},
    native_guest_turn_tests::with_guests,
};
use mod_api::{
    __private::{HookPanicReport, take_hook_panics},
    ConfigEvent, ConfigKind, ConfigSnapshot, ConfigSpec, Hooks, ModEntry, Registrar,
    RegistrationContext, ServerHost,
};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const CHILD: &str = "SUNRISE_SERVER_LIFECYCLE_CHILD";

/// Runs `test`, this module's test of that name, in a child process of its own, and false there.
/// A final save is admitted only while no entity pass is lifted anywhere in the process
/// (`Game::admit_entity_save`), so in a shared runner another test's lifted pass drops the
/// `saved` these tests expect.
fn in_child(test: &str) -> bool {
    let name = format!("game::server_lifecycle::tests::{test}");
    run_fresh_cases(&name, CHILD, &["stop"])
}

struct PanicAt {
    stopping: bool,
    config_unloading: bool,
    logout: bool,
}

struct StoppedProbe {
    serving_stopped: Box<dyn Fn() -> bool + Send + Sync>,
    lan: std::sync::Weak<sunrise_client::lan::Shared>,
    root: std::path::PathBuf,
    save: Arc<sunrise_client::save::SaveDir>,
}

#[test]
#[ignore = "daily"]
fn integrated_stop_finishes_after_an_unencodable_record() {
    if in_child("integrated_stop_finishes_after_an_unencodable_record") {
        return;
    }
    use sunrise_client::save::{LevelShape, SaveDir, SessionLock};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    struct SaveEvents(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SaveEvents {
        fn world_save_start(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::WorldSaveStartEvent,
        ) {
            self.0.lock().unwrap().push(if event.wait {
                "save start, wait"
            } else {
                "save start"
            });
        }
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.0.lock().unwrap().push("player saved");
        }
        fn level_save(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelSaveEvent<'_>) {
            self.0.lock().unwrap().push("level saved");
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("world saved");
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push("stopped");
        }
    }

    let root = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "save_events",
        Box::new(SaveEvents(events.clone())),
    )]
    .into();
    with_mods(mods, || {
        with_guests(|game| {
            game.join_world_rest().unwrap();
            game.world_root = root.path().to_path_buf();
            let open = || {
                SaveDir::new(
                    root.path().to_path_buf(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                )
            };
            game.save = Arc::new(open());
            game.session_lock = Some(SessionLock::acquire(root.path()).unwrap());
            game.world_adopted = true;
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            game.native_lifecycle.world_open_ready().unwrap();
            game.save_game().unwrap();
            assert_eq!(
                *events.lock().unwrap(),
                [
                    "save start, wait",
                    "player saved",
                    "player saved",
                    "level saved",
                    "world saved",
                ]
            );
            events.lock().unwrap().clear();
            game.level.name = "saved on exit after record failure".into();

            let slot = (2, -3);
            let neighbour = (3, -3);
            let deleted = (4, -3);
            let entity = |marker| Tag::compound([("marker", Tag::Int(marker))]);
            game.save
                .save_entities(vec![(slot, vec![entity(1)]), (deleted, vec![entity(3)])])
                .unwrap();
            let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
            game.save
                .save_entities_later(vec![
                    (slot, vec![invalid]),
                    (neighbour, vec![entity(2)]),
                    (deleted, vec![]),
                ])
                .unwrap();

            // An Err here reaches the client through Integrated::stop and makes quitting fatal.
            game.run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                .unwrap();
            assert!(game.session_lock.is_none());
            let disk = open();
            assert_eq!(disk.load_entities(slot.0, slot.1).unwrap(), [entity(1)]);
            assert_eq!(
                disk.load_entities(neighbour.0, neighbour.1).unwrap(),
                [entity(2)]
            );
            assert!(disk.load_entities(deleted.0, deleted.1).unwrap().is_empty());
            assert_eq!(
                sunrise_client::save::Level::read(root.path()).unwrap().name,
                "saved on exit after record failure"
            );
            game.save.flush().unwrap();
            assert_eq!(game.save.record_save_failures(), 1);
            assert_eq!(
                *events.lock().unwrap(),
                [
                    "save start, wait",
                    "player saved",
                    "player saved",
                    "level saved",
                    "world saved",
                    "stopped",
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn integrated_stop_propagates_metadata_errors_even_after_a_record_failure() {
    if in_child("integrated_stop_propagates_metadata_errors_even_after_a_record_failure") {
        return;
    }
    use sunrise_client::save::{LevelShape, SaveDir, SessionLock};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    with_mods(Vec::new().into(), || {
        with_game(|game| {
            for mixed in [false, true] {
                let root = tempfile::tempdir().unwrap();
                game.world_root = root.path().to_path_buf();
                game.save = Arc::new(SaveDir::new(
                    game.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.session_lock = Some(SessionLock::acquire(root.path()).unwrap());
                game.world_adopted = true;
                if mixed {
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.save
                        .save_entities(vec![((2, 0), vec![invalid])])
                        .unwrap();
                    assert_eq!(game.save.record_save_failures(), 1);
                }
                let blocked = root.path().join("level.dat_new");
                std::fs::create_dir(&blocked).unwrap();
                let expected = std::fs::write(&blocked, b"test").unwrap_err();
                let error = game
                    .run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                    .unwrap_err();
                assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    expected.kind()
                );
                assert!(game.session_lock.is_none());
            }
        });
    });
}

/// Exercise the real stop/save paths, not just direct calls to hook dispatch.
#[test]
#[ignore = "daily"]
fn every_started_server_stops_and_saves_even_when_startup_or_ticks_fail() {
    if in_child("every_started_server_stops_and_saves_even_when_startup_or_ticks_fail") {
        return;
    }
    struct Log(
        Arc<Mutex<Vec<String>>>,
        PanicAt,
        Arc<Mutex<Option<StoppedProbe>>>,
    );
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let log = self.0.clone();
            let panic_on_unload = self.1.config_unloading;
            r.register_config(
                ConfigSpec::new(ConfigKind::Server, ""),
                Some(Arc::new(
                    move |event: ConfigEvent<'_>, _: &ConfigSnapshot| {
                        if matches!(event, ConfigEvent::Unloading(_)) {
                            log.lock().unwrap().push("config-unloading".into());
                            if panic_on_unload {
                                panic!("config unloading listener");
                            }
                        }
                        Ok(())
                    },
                )),
            )
            .unwrap();
        }
        fn server_about_to_start(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0.lock().unwrap().push("about-to-start".into());
        }
        fn level_load(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelLoadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-load:{}", e.dimension));
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting".into());
        }
        fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
            self.0.lock().unwrap().push("started".into());
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping".into());
            if self.1.stopping {
                panic!("stopping listener");
            }
        }
        fn level_save(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelSaveEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-save:{}", e.dimension));
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("saved".into());
        }
        fn player_logged_out(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerLoggedOutEvent<'_>,
        ) {
            if self.1.logout {
                self.0.lock().unwrap().push("logout".into());
                panic!("logout listener");
            }
        }
        fn level_unload(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("level-unload:{}", e.dimension));
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            if let Some(probe) = self.2.lock().unwrap().as_ref() {
                assert!((probe.serving_stopped)(), "serving retired before Stopped");
                assert!(probe.lan.upgrade().is_none(), "LAN dropped before Stopped");
                let saved = sunrise_client::save::Level::read(&probe.root).unwrap();
                assert_eq!(saved.name, "lifecycle final save");
                assert!(
                    probe.save.raw_chunk(0, 0).is_some(),
                    "dirty chunk saved before Stopped"
                );
                assert_eq!(
                    self.0.lock().unwrap().last().map(String::as_str),
                    Some("level-unload:minecraft:overworld")
                );
                // Storage is released before the notification.
                let _lock = sunrise_client::save::SessionLock::acquire(&probe.root).unwrap();
            }
            self.0.lock().unwrap().push("stopped".into());
        }
    }
    for dedicated in [false, true] {
        for case in [
            "normal",
            "startup-error",
            "opening-failed-init",
            "bind-failed-init",
            "runtime-extra-level",
            "tick-error",
            "panic",
            "save-error",
            "unopened",
            "locked-unadopted",
            "stopping-panic",
            "config-unloading-panic",
            "config-unloading-logout-panic",
        ] {
            let prepare_error = case == "dedicated-prepare-error";
            if prepare_error && !dedicated {
                continue;
            }
            let log = Arc::new(Mutex::new(Vec::new()));
            let stopped_probe = Arc::new(Mutex::new(None));
            let config_panic = case.starts_with("config-unloading");
            let panic_at = PanicAt {
                stopping: case == "stopping-panic",
                config_unloading: config_panic,
                logout: case == "config-unloading-logout-panic",
            };
            // Panic latches are per installed mod and hook, not per world. Each case needs
            // a fresh installation to test its own failures, including the dedicated cases.
            with_mods(
                vec![ModEntry::new(
                    "log",
                    Box::new(Log(log.clone(), panic_at, stopped_probe.clone())),
                )]
                .into(),
                || {
                    let run_case = |game: &mut crate::game::Game<'_>| {
                        game.options.execution_role = if dedicated {
                            ExecutionRole::DedicatedServer
                        } else {
                            ExecutionRole::IntegratedServer
                        };
                        let root = std::env::temp_dir().join(format!(
                            "server-lifecycle-{}-{dedicated}-{case}",
                            std::process::id()
                        ));
                        std::fs::create_dir_all(&root).unwrap();
                        let serverconfig = root.join("serverconfig");
                        std::fs::create_dir_all(&serverconfig).unwrap();
                        std::fs::write(serverconfig.join("log"), "").unwrap();
                        game.world_root = root.clone();
                        game.save = Arc::new(sunrise_client::save::SaveDir::new(
                            root.clone(),
                            sunrise_client::save::LevelShape::OVERWORLD,
                            Some(Default::default()),
                        ));
                        game.level.name = "lifecycle final save".into();
                        let adopted =
                            !matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted");
                        if adopted || case == "locked-unadopted" {
                            game.session_lock =
                                Some(sunrise_client::save::SessionLock::acquire(&root).unwrap());
                        }
                        // The lock is held while tags are readable, before the prepared
                        // level replaces the placeholder. Only adoption may save.
                        game.world_adopted = adopted;
                        if matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted") {
                            // A failed open must not overwrite an existing, unreadable save.
                            std::fs::write(root.join("level.dat"), b"unadopted save").unwrap();
                        }
                        if config_panic || prepare_error {
                            let config = root.join("serverconfig");
                            std::fs::create_dir_all(&config).unwrap();
                            std::fs::write(config.join("log-server.toml"), "fixture = true\n")
                                .unwrap();
                        }
                        if config_panic {
                            game.level.name = "previous save".into();
                            game.level.write(&root, &Default::default()).unwrap();
                            // Nothing queues these metadata/block changes. A flush
                            // without the final save must leave the old name and no chunk.
                            game.level.name = "lifecycle final save".into();
                            game.world.chunks.insert(
                                (0, 0),
                                Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
                            );
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.world.set(1, 64, 1, stone).unwrap();
                            assert!(game.save.raw_chunk(0, 0).is_none());
                            let serving_stopped = if dedicated {
                                Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>
                            } else {
                                // Cover both a published and an unpublished integrated
                                // server. Neither may merely unpublish on shutdown.
                                Box::new(super::super::integrated::tests::install_serving(
                                    game,
                                    case == "config-unloading-logout-panic",
                                ))
                            };
                            let lan = Arc::downgrade(&game.lan.as_ref().unwrap().hub.lan.shared);
                            *stopped_probe.lock().unwrap() = Some(StoppedProbe {
                                serving_stopped,
                                lan,
                                root: root.clone(),
                                save: game.save.clone(),
                            });
                        }
                        let result = game.run_server_lifecycle(dedicated, |game| {
                            if case == "bind-failed-init" {
                                return Ok(super::ServerRunOutcome::FailedInit(
                                    std::io::Error::other("fixture initialization error"),
                                ));
                            }
                            game.native_configs_server_start(&root, dedicated);
                            game.native_server_about_to_start(dedicated);
                            game.native_lifecycle_open_started();
                            if case == "unopened" {
                                return Err("failed init".into());
                            }
                            game.native_level_opened(game.dim);
                            if matches!(case, "startup-error" | "locked-unadopted") {
                                return Err("failed init".into());
                            }
                            if case == "opening-failed-init" {
                                return Ok(super::ServerRunOutcome::FailedInit(
                                    std::io::Error::other("fixture initialization error"),
                                ));
                            }
                            game.native_server_starting(dedicated);
                            game.native_lifecycle_world_ready();
                            if case == "runtime-extra-level" {
                                game.join_world_rest().unwrap();
                                // The world's data pack defines the pocket's stem, as a
                                // data pack's `dimension/` entry does in vanilla.
                                let pack = root.join("datapacks/pocket");
                                let stems = pack.join("data/test/dimension");
                                std::fs::create_dir_all(&stems).unwrap();
                                std::fs::write(
                                    pack.join("pack.mcmeta"),
                                    r#"{"pack":{"description":"Pocket","min_format":[121,0],"max_format":[121,0]}}"#,
                                )
                                .unwrap();
                                std::fs::write(
                                    stems.join("pocket.json"),
                                    r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{
                                        "biome":"minecraft:plains","features":false,"lakes":false,"structure_overrides":[],
                                        "layers":[{"height":1,"block":"minecraft:bedrock"}]}}}"#,
                                )
                                .unwrap();
                                std::fs::write(
                                    root.join("datapacks.json"),
                                    r#"{"enabled":["vanilla","file/pocket"],"disabled":[]}"#,
                                )
                                .unwrap();
                                Dim::add_extra("test:pocket");
                                let pocket = Dim::extra("test:pocket").unwrap();
                                let parked = game.open_level(pocket).unwrap();
                                game.levels.parked.insert(pocket, parked);
                                game.native_level_opened(pocket);
                            }
                            match case {
                                "tick-error" => Err("failed tick".into()),
                                "panic" => panic!("failed tick"),
                                "save-error" => {
                                    let file = root.join("not-a-directory");
                                    std::fs::write(&file, b"occupied").unwrap();
                                    game.world_root = file;
                                    Ok(super::ServerRunOutcome::Orderly)
                                }
                                _ => Ok(super::ServerRunOutcome::Orderly),
                            }
                        });
                        assert_eq!(
                            result.is_ok(),
                            matches!(case, "normal" | "runtime-extra-level" | "stopping-panic")
                                || config_panic,
                            "{dedicated} {case}: {result:?}"
                        );
                        let expected_panics: &[&str] = match case {
                            "stopping-panic" => &["server_stopping"],
                            "config-unloading-panic" => &["config_unloading"],
                            "config-unloading-logout-panic" => {
                                &["player_logged_out", "config_unloading"]
                            }
                            _ => &[],
                        };
                        assert_eq!(
                            take_hook_panics(),
                            expected_panics
                                .iter()
                                .map(|&hook| HookPanicReport {
                                    mod_id: "log",
                                    hook
                                })
                                .collect::<Vec<_>>(),
                            "{dedicated} {case}"
                        );
                        if prepare_error {
                            let error = result.as_ref().unwrap_err().to_string();
                            assert!(error.contains("its level data does not read"), "{error}");
                        }
                        let expected: &[&str] = match case {
                            "bind-failed-init" => &["stopped"],
                            "unopened" => &["about-to-start", "stopped", "config-unloading"],
                            "locked-unadopted" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "startup-error" | "opening-failed-init" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "tick-error" | "panic" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "save-error" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            "runtime-extra-level" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "level-load:test:pocket",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "level-save:test:pocket",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "level-unload:test:pocket",
                                "stopped",
                                "config-unloading",
                            ],
                            "config-unloading-logout-panic" => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "logout",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                            _ => &[
                                "about-to-start",
                                "level-load:minecraft:overworld",
                                "starting",
                                "started",
                                "stopping",
                                "level-save:minecraft:overworld",
                                "saved",
                                "level-unload:minecraft:overworld",
                                "stopped",
                                "config-unloading",
                            ],
                        };
                        assert_eq!(*log.lock().unwrap(), expected, "{dedicated} {case}");
                        game.native_lifecycle_world_closed();
                        assert_eq!(
                            *log.lock().unwrap(),
                            expected,
                            "second close is a no-op after {case}"
                        );
                        assert!(game.session_lock.is_none(), "storage released after {case}");
                        assert!(!game.world_adopted, "adoption cleared after {case}");
                        assert!(game.serving.is_none(), "serving retired after {case}");
                        assert!(game.lan.is_none(), "LAN dropped after {case}");
                        if config_panic {
                            let registry = &game.pack.assets.registry;
                            let chunk =
                                game.save.load(0, 0, registry, world_biomes!(game)).unwrap();
                            assert_eq!(
                                chunk.block(1, 64, 1),
                                registry.default_state("minecraft:stone").unwrap()
                            );
                        }
                        if matches!(case, "unopened" | "bind-failed-init" | "locked-unadopted") {
                            assert_eq!(
                                std::fs::read(root.join("level.dat")).unwrap(),
                                b"unadopted save",
                                "{dedicated} {case}"
                            );
                        } else if case != "save-error" {
                            let saved = sunrise_client::save::Level::read(&root).unwrap();
                            assert_eq!(saved.name, "lifecycle final save");
                        }
                        if case == "runtime-extra-level" {
                            Dim::set_extra(std::iter::empty());
                        }
                        std::fs::remove_dir_all(root).unwrap();
                    };
                    if config_panic {
                        with_guests(run_case);
                    } else {
                        with_game(run_case);
                    }
                },
            );
        }
    }
}

/// A failed notice cannot turn a successful `saveAllChunks` into a save error or skip
/// the `stopServer` level-close loop.
#[test]
#[ignore = "daily"]
fn a_world_saved_panic_discards_only_its_listeners_journal() {
    // The save gate observes process-wide visibility leases from other tests.
    crate::game::in_own_process(
        "game::server_lifecycle::tests::a_world_saved_panic_discards_only_its_listeners_journal",
        world_saved_panic_during_shutdown,
    );
}

fn world_saved_panic_during_shutdown() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{BlockState, BlockUpdateFlags, HostTransaction, WorldSavedEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunrise_client::{chunk::Chunk, save};

    const POS: [i32; 3] = [1, 64, 1];
    const LEVEL: &str = "minecraft:overworld";

    struct Panics {
        calls: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = 100)]
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let level = host.level(LEVEL).unwrap();
            let gold = host.block_state_id("minecraft:gold_block").unwrap();
            host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(host, POS, gold, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap();
            assert_eq!(level.block_state(host, POS), Some(gold));
            self.writes.fetch_add(1, Ordering::Relaxed);
            panic!("after committing a nested write");
        }
    }

    #[derive(Debug, PartialEq)]
    enum Notice {
        Saved(Option<BlockState>),
        Unloaded(String),
        Stopped,
    }
    struct Observer(Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            self.0.lock().unwrap().push(Notice::Saved(
                host.level(LEVEL)
                    .and_then(|level| level.block_state(host, POS)),
            ));
        }
        fn level_unload(&self, _: &mut dyn ServerHost, event: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(Notice::Unloaded(event.dimension.into()));
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push(Notice::Stopped);
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let notices = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![
            ModEntry::new(
                "broken-save",
                Box::new(Panics {
                    calls: calls.clone(),
                    writes: writes.clone(),
                }),
            ),
            ModEntry::new("observer", Box::new(Observer(notices.clone()))),
        ]
        .into(),
        || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.join_world_rest().unwrap();
                game.world_root = root.path().to_path_buf();
                game.save = Arc::new(save::SaveDir::new(
                    game.world_root.clone(),
                    save::LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.session_lock = Some(save::SessionLock::acquire(root.path()).unwrap());
                game.world_adopted = true;
                game.level.name = "saved despite listener panic".into();
                game.native_level_opened(Dim::Overworld);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                game.world.set(POS[0], POS[1], POS[2], stone).unwrap();
                let stone_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:stone")
                    .unwrap();

                game.run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                    .unwrap();

                assert_eq!(
                    writes.load(Ordering::Relaxed),
                    1,
                    "the callback reached its deliberate panic"
                );
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "broken-save",
                        hook: "world_saved"
                    }]
                );
                assert_eq!(
                    notices.lock().unwrap().as_slice(),
                    [
                        Notice::Saved(Some(stone_api)),
                        Notice::Unloaded(LEVEL.into()),
                        Notice::Stopped
                    ]
                );
                assert_eq!(
                    save::Level::read(root.path()).unwrap().name,
                    "saved despite listener panic"
                );
                let saved = game.save.load(0, 0, registry, world_biomes!(game)).unwrap();
                assert_eq!(saved.block(1, 64, 1), stone);
                assert!(game.session_lock.is_none());

                game.native_world_saved(true);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "the panicking callback stays disabled"
                );
                assert_eq!(notices.lock().unwrap().last(), Some(&Notice::Saved(None)));
                assert!(take_hook_panics().is_empty());
            })
        },
    );
}

/// A panicking `world_save_start` listener rolls back its committed nested writes before
/// the snapshot is taken, while healthy listeners' writes persist and shutdown finishes.
#[test]
#[ignore = "daily"]
fn a_world_save_start_panic_discards_only_its_listeners_journal() {
    crate::game::in_own_process(
        "game::server_lifecycle::tests::a_world_save_start_panic_discards_only_its_listeners_journal",
        world_save_start_panic_during_shutdown,
    );
}

fn world_save_start_panic_during_shutdown() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{
        BlockState, BlockUpdateFlags, HostTransaction, WorldSaveStartEvent, WorldSavedEvent,
    };
    use std::sync::atomic::AtomicUsize;
    use sunrise_client::{chunk::Chunk, save};

    const POS_PANIC: [i32; 3] = [1, 64, 1];
    const POS_HEALTHY: [i32; 3] = [2, 64, 1];
    const LEVEL: &str = "minecraft:overworld";

    struct Panics {
        calls: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Panics {
        #[listener(priority = 100)]
        fn world_save_start(&self, host: &mut dyn ServerHost, _: &mut WorldSaveStartEvent) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let level = host.level(LEVEL).unwrap();
            let gold = host.block_state_id("minecraft:gold_block").unwrap();
            host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(host, POS_PANIC, gold, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                Ok::<_, std::convert::Infallible>(())
            })
            .unwrap();
            assert_eq!(level.block_state(host, POS_PANIC), Some(gold));
            self.writes.fetch_add(1, Ordering::Relaxed);
            panic!("after committing a nested write");
        }
    }

    #[derive(Debug, PartialEq)]
    enum Notice {
        Start(Option<BlockState>),
        Saved(Option<BlockState>, Option<BlockState>),
        Unloaded(String),
        Stopped,
    }
    struct Observer(Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn world_save_start(&self, host: &mut dyn ServerHost, _: &mut WorldSaveStartEvent) {
            let at_panic = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_PANIC));
            if let Some(level) = host.level(LEVEL) {
                let iron = host.block_state_id("minecraft:iron_block").unwrap();
                assert_eq!(
                    level.set_block(host, POS_HEALTHY, iron, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            self.0.lock().unwrap().push(Notice::Start(at_panic));
        }
        fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
            let at_panic = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_PANIC));
            let at_healthy = host
                .level(LEVEL)
                .and_then(|level| level.block_state(host, POS_HEALTHY));
            self.0
                .lock()
                .unwrap()
                .push(Notice::Saved(at_panic, at_healthy));
        }
        fn level_unload(&self, _: &mut dyn ServerHost, event: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(Notice::Unloaded(event.dimension.into()));
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push(Notice::Stopped);
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let notices = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![
            ModEntry::new(
                "broken-save-start",
                Box::new(Panics {
                    calls: calls.clone(),
                    writes: writes.clone(),
                }),
            ),
            ModEntry::new("observer", Box::new(Observer(notices.clone()))),
        ]
        .into(),
        || {
            with_game(|game| {
                let root = tempfile::tempdir().unwrap();
                game.join_world_rest().unwrap();
                game.world_root = root.path().to_path_buf();
                let open = || {
                    save::SaveDir::new(
                        root.path().to_path_buf(),
                        save::LevelShape::OVERWORLD,
                        Some(Default::default()),
                    )
                };
                game.save = Arc::new(open());
                game.session_lock = Some(save::SessionLock::acquire(root.path()).unwrap());
                game.world_adopted = true;
                game.level.name = "saved despite start listener panic".into();
                game.native_level_opened(Dim::Overworld);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let iron = registry.default_state("minecraft:iron_block").unwrap();
                game.world
                    .set(POS_PANIC[0], POS_PANIC[1], POS_PANIC[2], stone)
                    .unwrap();
                game.world
                    .set(POS_HEALTHY[0], POS_HEALTHY[1], POS_HEALTHY[2], stone)
                    .unwrap();
                let stone_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:stone")
                    .unwrap();
                let iron_api = (&*game as &dyn ServerHost)
                    .block_state_id("minecraft:iron_block")
                    .unwrap();

                game.run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                    .unwrap();

                assert_eq!(
                    writes.load(Ordering::Relaxed),
                    1,
                    "the callback reached its deliberate panic"
                );
                assert_eq!(
                    take_hook_panics(),
                    [HookPanicReport {
                        mod_id: "broken-save-start",
                        hook: "world_save_start"
                    }]
                );
                assert_eq!(
                    notices.lock().unwrap().as_slice(),
                    [
                        Notice::Start(Some(stone_api)),
                        Notice::Saved(Some(stone_api), Some(iron_api)),
                        Notice::Unloaded(LEVEL.into()),
                        Notice::Stopped
                    ]
                );
                assert_eq!(
                    save::Level::read(root.path()).unwrap().name,
                    "saved despite start listener panic"
                );
                let reloaded = open().load(0, 0, registry, world_biomes!(game)).unwrap();
                assert_eq!(reloaded.block(1, 64, 1), stone);
                assert_eq!(reloaded.block(2, 64, 1), iron);
                assert!(game.session_lock.is_none());

                game.native_world_save_start(true);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "the panicking callback stays disabled"
                );
                assert_eq!(notices.lock().unwrap().last(), Some(&Notice::Start(None)));
                assert!(take_hook_panics().is_empty());
            })
        },
    );
}

/// `DedicatedServer.initServer` binds the TCP port before `ServerAboutToStartEvent`
/// and world preparation; a bind failure emits only `server_stopped` and leaves
/// the world directory untouched.
#[test]
#[ignore = "daily"]
fn dedicated_port_bind_failure_emits_only_stopped_without_touching_world() {
    use sunrise_client::server::properties::{ServerProperties, SystemEntropy};
    use sunrise_java_properties::Properties;

    struct Log(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn server_about_to_start(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0.lock().unwrap().push("about-to-start");
        }
        fn level_load(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelLoadEvent<'_>) {
            self.0.lock().unwrap().push("level-load");
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting");
        }
        fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
            self.0.lock().unwrap().push("started");
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping");
        }
        fn level_unload(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0.lock().unwrap().push("level-unload");
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            self.0.lock().unwrap().push("stopped");
        }
    }

    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let universe = tempfile::tempdir().unwrap();
    let world_dir = universe.path().join("untouched_world");
    let mut properties = Properties::default();
    properties.put("online-mode", "false".into());
    properties.put("server-ip", "127.0.0.1".into());
    properties.put("server-port", port.to_string());
    let props = ServerProperties::new(properties, &mut SystemEntropy::default()).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new("log", Box::new(Log(log.clone())))].into(),
        || {
            with_game(|game| {
                game.options.execution_role = ExecutionRole::DedicatedServer;
                let setup = crate::game::dedicated::Setup {
                    props,
                    universe: universe.path().to_path_buf(),
                    level_name: "untouched_world".into(),
                    port: Some(port),
                    allow_missing_mods: false,
                };
                let error = game.run_dedicated_with_setup(setup).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .starts_with("Failed to initialize server:")
                );
            });
        },
    );
    assert_eq!(*log.lock().unwrap(), ["stopped"]);
    assert!(
        !world_dir.exists(),
        "failed port bind must not create or write world files"
    );
    drop(occupied);
}

/// The real world-open path promotes the Overworld, initializes its spawn, then
/// walks the resolved stems. Tick, save, close and Host enumeration reuse that order.
#[test]
#[ignore = "daily"]
fn datapack_dimensions_open_prepare_save_and_close_in_creation_order() {
    crate::game::in_own_process(
        "game::server_lifecycle::tests::datapack_dimensions_open_prepare_save_and_close_in_creation_order",
        || {
            use crate::game::world_loading::WorldRequest;
            use std::fs;
            use sunrise_client::save::Level;

            struct Log {
                events: Arc<Mutex<Vec<String>>>,
                listed_at_load: Arc<Mutex<Vec<Vec<String>>>>,
            }
            #[mod_api::__subscriptions]
            impl Hooks for Log {
                fn level_load(
                    &self,
                    host: &mut dyn ServerHost,
                    e: &mut mod_api::LevelLoadEvent<'_>,
                ) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("load:{}", e.dimension));
                    let mut levels = Vec::new();
                    let _ = host.levels().visit_levels(&mut |level| {
                        levels.push(level.dimension_id().to_owned());
                        std::ops::ControlFlow::Continue(())
                    });
                    self.listed_at_load.lock().unwrap().push(levels);
                }
                fn create_spawn_position(
                    &self,
                    _: &mut dyn ServerHost,
                    e: &mut mod_api::CreateSpawnPositionEvent<'_>,
                ) -> mod_api::Flow {
                    self.events.lock().unwrap().push("initialize-spawn".into());
                    *e.position = [32, 73, 16];
                    mod_api::Flow::Cancel
                }
                fn level_save(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelSaveEvent<'_>) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("save:{}", e.dimension));
                }
                fn level_unload(
                    &self,
                    _: &mut dyn ServerHost,
                    e: &mut mod_api::LevelUnloadEvent<'_>,
                ) {
                    self.events
                        .lock()
                        .unwrap()
                        .push(format!("unload:{}", e.dimension));
                }
            }

            let root = tempfile::tempdir().unwrap();
            let pack = root.path().join("datapacks/stems");
            let dimensions = pack.join("data/test/dimension");
            fs::create_dir_all(&dimensions).unwrap();
            fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Lifecycle fixture","min_format":[121,0],"max_format":[121,0]}}"#,
    )
    .unwrap();
            fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["vanilla","file/stems"],"disabled":[]}"#,
            )
            .unwrap();
            for name in ["last", "first"] {
                fs::write(
            dimensions.join(format!("{name}.json")),
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{
                "biome":"minecraft:plains","features":false,"lakes":false,"structure_overrides":[],
                "layers":[{"height":1,"block":"minecraft:bedrock"}]}}}"#,
        )
        .unwrap();
            }
            let log = Arc::new(Mutex::new(Vec::new()));
            let listed_at_load = Arc::new(Mutex::new(Vec::new()));
            with_mods(
                vec![ModEntry::new(
                    "log",
                    Box::new(Log {
                        events: log.clone(),
                        listed_at_load: listed_at_load.clone(),
                    }),
                )]
                .into(),
                || {
                    with_game(|game| {
                        game.options.execution_role = ExecutionRole::DedicatedServer;
                        game.join_world_rest().unwrap();
                        let mut request = WorldRequest::new(
                            "stems",
                            Some("stems"),
                            Some(1),
                            false,
                            None,
                            Level::default().rule_registry,
                        )
                        .unwrap();
                        request.save_root = root.path().to_path_buf();
                        // Supply the effective registry's traversal before the normal prepare path
                        // projects its shapes. This is not a claim about Java's hash-bucket order.
                        request.stem_order = Some(&[
                            "test:last",
                            "minecraft:the_nether",
                            "minecraft:overworld",
                            "test:first",
                            "minecraft:the_end",
                        ]);
                        let prepared = game.prepare_world_and_rules(request).unwrap();
                        assert!(prepared.level.world_spawn.is_none());
                        let order = [
                            "minecraft:overworld",
                            "test:last",
                            "minecraft:the_nether",
                            "test:first",
                            "minecraft:the_end",
                        ];
                        game.native_lifecycle_open_started();
                        game.finish_open_world(prepared).unwrap();
                        game.prepare_levels().unwrap();
                        assert_eq!(game.level.world_spawn, Some([32, 73, 16]));
                        // Switching the active level must not put it first in saves or close.
                        game.enter_level(Dim::Nether).unwrap();
                        let mut visited = Vec::new();
                        let _ = game.native_visit_levels(&mut |level| {
                            visited.push(level.dimension_id());
                            std::ops::ControlFlow::Continue(())
                        });
                        assert_eq!(visited, order);
                        let listed = listed_at_load.lock().unwrap();
                        assert_eq!(listed.len(), order.len());
                        for (i, levels) in listed.iter().enumerate() {
                            assert_eq!(
                                levels.iter().map(String::as_str).collect::<Vec<_>>(),
                                &order[..=i]
                            );
                        }
                        // Set work on a mix of built-in and custom levels, including ones
                        // whose registry order crosses the built-in and filename order.
                        for id in order {
                            game.enter_level(Dim::from_id(id)).unwrap();
                            game.streaming.forced.insert((0, 0));
                        }
                        assert_eq!(
                            game.levels_to_tick()
                                .into_iter()
                                .map(Dim::id)
                                .collect::<Vec<_>>(),
                            order
                        );
                        game.enter_level(Dim::Nether).unwrap();
                        game.save_game().unwrap();
                        assert_eq!(game.dim, Dim::Nether);
                        game.close_loaded_levels_for_stop();
                        game.native_lifecycle_world_closed();
                        Dim::set_extra(std::iter::empty());

                        let mut expected =
                            vec!["load:minecraft:overworld".into(), "initialize-spawn".into()];
                        expected.extend(order[1..].iter().map(|id| format!("load:{id}")));
                        expected.extend(order.iter().map(|id| format!("save:{id}")));
                        expected.extend(order.iter().map(|id| format!("unload:{id}")));
                        assert_eq!(*log.lock().unwrap(), expected);
                    })
                },
            );
        },
    );
}

/// In `MinecraftServer.stopServer`, a per-level close (`save.flush()`) error
/// logs and skips `level_unload` for that level while still closing and firing
/// `level_unload` for the other loaded levels.
#[test]
#[ignore = "daily"]
fn per_level_close_failure_skips_only_that_levels_unload() {
    struct Log(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn level_unload(&self, _: &mut dyn ServerHost, e: &mut mod_api::LevelUnloadEvent<'_>) {
            self.0.lock().unwrap().push(e.dimension.to_owned());
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new("log", Box::new(Log(log.clone())))].into(),
        || {
            with_game(|game| {
                game.join_world_rest().unwrap();
                let root = tempfile::tempdir().unwrap();
                game.world_root = root.path().to_path_buf();
                game.save = Arc::new(sunrise_client::save::SaveDir::new(
                    game.world_root.clone(),
                    sunrise_client::save::LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.level.world_spawn = Some([0, 64, 0]);
                game.native_lifecycle_open_started();
                game.open_configured_levels(&[Dim::Overworld, Dim::Nether, Dim::End])
                    .unwrap();
                // Inject a storage sync error on the parked Nether's SaveDir so its
                // close (`save.flush()`) fails with an IO error while individual record
                // failures continue to be reported non-fatally.
                let bad_save = Arc::new(sunrise_client::save::SaveDir::new(
                    root.path().join("broken_nether"),
                    sunrise_client::save::LevelShape::NETHER,
                    Some(Default::default()),
                ));
                bad_save.inject_sync_error_for_test(std::io::Error::other(
                    "simulated storage sync failure",
                ));
                game.levels.parked.get_mut(&Dim::Nether).unwrap().save = bad_save;

                game.close_loaded_levels_for_stop();
                game.native_lifecycle_world_closed();
            });
        },
    );
    assert_eq!(
        *log.lock().unwrap(),
        ["minecraft:overworld", "minecraft:the_end"]
    );
}

/// Final level.dat writes must not serialize the integrated server's shell as its owner.
#[test]
#[ignore = "daily"]
fn integrated_shutdown_reopens_the_authoritative_owner_or_keeps_an_interrupted_save() {
    use crate::game::lan::Owner;
    use sunrise_client::{
        items::ItemStack,
        lan::playerdata,
        save::{self, Level, LevelShape, SaveDir},
    };
    use sunrise_core::nbt::Tag;

    for (interrupted, legacy) in [(false, false), (true, false), (true, true)] {
        with_guests(|game| {
            game.join_world_rest().unwrap();
            let root = tempfile::tempdir().unwrap();
            let dir = root.path();
            let items = game.res.items();
            let mut old = Level {
                player_uuid: Some(playerdata::uuid_ints(12)),
                name: "previous".into(),
                position: Some([1.0, 72.0, 3.0]),
                dimension: "minecraft:overworld".into(),
                xp_level: 4,
                xp_total: 14,
                inventory: vec![(0, "minecraft:stone".into(), 2, 0)],
                ..Level::default()
            };
            let old_player = old.player_tag(items);
            let level_dat = dir.join("level.dat");
            if legacy {
                old.write_without_player(dir).unwrap();
                let mut root = save::read_dat(&level_dat).unwrap();
                let Some(Tag::Compound(data)) = root.get_mut("Data") else {
                    panic!("level.dat has a Data compound");
                };
                data.insert("Player".into(), old_player.clone());
                data.remove(&"singleplayer_uuid".into());
                save::write_dat(&level_dat, root).unwrap();
            } else {
                old.write(dir, items).unwrap();
            }
            let player_file = playerdata::path(dir, 12);
            let prior_bytes = std::fs::read(&player_file).ok();

            let host = game.lan.as_mut().unwrap();
            host.owner = Some(Owner {
                uuid: 12,
                saved: None,
            });
            host.hub.singleplayer_owner = Some(12);
            host.hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:the_nether".into();
                    guest.position = [8.0, 90.0, -4.0];
                    guest.xp_level = 11;
                    guest.xp_total = 90;
                    guest.inventory[0] = Some(ItemStack {
                        item: "minecraft:diamond".into(),
                        count: 9,
                        damage: 0,
                        extra: None,
                    });
                })
                .unwrap();
            game.world_root = dir.to_path_buf();
            game.save = Arc::new(SaveDir::new(
                dir.to_path_buf(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            ));
            game.session_lock = Some(save::SessionLock::acquire(dir).unwrap());
            game.world_adopted = true;
            game.level = old;
            game.level.name = "stopped".into();
            if interrupted {
                game.interrupted_turns.push(12);
            }
            game.run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                .unwrap();

            let saved = Level::read(dir).unwrap();
            assert_eq!(saved.name, "stopped");
            if interrupted {
                assert_eq!(std::fs::read(&player_file).ok(), prior_bytes);
                assert_eq!(saved.dimension, "minecraft:overworld");
                assert_eq!((saved.xp_level, saved.xp_total), (4, 14));
                assert_eq!(saved.inventory, [(0, "minecraft:stone".to_owned(), 2, 0)]);
                if legacy {
                    let root = save::read_dat(&level_dat).unwrap();
                    assert_eq!(
                        root.get("Data").and_then(|data| data.get("Player")),
                        Some(&old_player)
                    );
                }
            } else {
                assert!(saved.singleplayer_file);
                assert_eq!(saved.dimension, "minecraft:the_nether");
                assert_eq!(saved.position, Some([8.0, 90.0, -4.0]));
                assert_eq!((saved.xp_level, saved.xp_total), (11, 90));
                assert_eq!(saved.inventory, [(0, "minecraft:diamond".to_owned(), 9, 0)]);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn shutdown_notifies_only_for_replaced_player_files() {
    use crate::game::{lan::Owner, native_entity_refs::tests::run_fresh_cases};
    use sunrise_client::{
        lan::playerdata,
        save::{self, Level, LevelShape, SaveDir, SessionLock},
    };
    use sunrise_core::nbt::Tag;

    const CASE: &str = "SUNRISE_SHUTDOWN_PLAYER_FILE_CASE";
    if run_fresh_cases(
        "game::server_lifecycle::tests::shutdown_notifies_only_for_replaced_player_files",
        CASE,
        &["owner", "guest"],
    ) {
        return;
    }
    let (failed, written) = if std::env::var(CASE).unwrap() == "owner" {
        (12, 13)
    } else {
        (13, 12)
    };
    struct Log(Arc<Mutex<Vec<u128>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.0.lock().unwrap().push(event.player);
        }
    }
    let notices = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "shutdown_save",
            Box::new(Log(notices.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.join_world_rest().unwrap();
                game.options.execution_role = ExecutionRole::IntegratedServer;
                let root = tempfile::tempdir().unwrap();
                let dir = root.path();
                let mut level = Level {
                    player_uuid: Some(playerdata::uuid_ints(12)),
                    name: "before shutdown".into(),
                    xp_level: 3,
                    ..Level::default()
                };
                level.write(dir, game.res.items()).unwrap();
                let failed_path = playerdata::path(dir, failed);
                if failed != 12 {
                    save::write_dat(&failed_path, Tag::compound([("XpLevel", Tag::Int(3))]))
                        .unwrap();
                }
                let prior = std::fs::read(&failed_path).unwrap();
                // Block the backup rename before the existing player file can move.
                let backup = failed_path.with_extension("dat_old");
                std::fs::create_dir(&backup).unwrap();
                std::fs::write(backup.join("sentinel"), b"keep").unwrap();
                let host = game.lan.as_mut().unwrap();
                host.owner = Some(Owner {
                    uuid: 12,
                    saved: None,
                });
                host.hub.singleplayer_owner = Some(12);
                for uuid in [12, 13] {
                    host.hub.update_guest(uuid, |g| g.xp_level = 9).unwrap();
                }
                game.world_root = dir.to_owned();
                game.save = Arc::new(SaveDir::new(
                    dir.to_owned(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                game.session_lock = Some(SessionLock::acquire(dir).unwrap());
                game.world_adopted = true;
                game.level = level;
                game.level.name = "after shutdown".into();
                game.run_server_lifecycle(false, |_| Ok(super::ServerRunOutcome::Orderly))
                    .unwrap();

                assert_eq!(std::fs::read(&failed_path).unwrap(), prior);
                let player = save::read_dat(&playerdata::path(dir, written)).unwrap();
                assert_eq!(player.get("XpLevel"), Some(&Tag::Int(9)));
                assert_eq!(Level::read(dir).unwrap().name, "after shutdown");
            });
        },
    );
    assert_eq!(*notices.lock().unwrap(), [written]);
}

/// A Debug world's fixed spawn bypasses the NeoForge event in setInitialSpawn.
#[test]
#[ignore = "daily"]
fn debug_world_uses_fixed_spawn_without_create_spawn_position() {
    struct ReplaceSpawn(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for ReplaceSpawn {
        fn create_spawn_position(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::CreateSpawnPositionEvent<'_>,
        ) -> mod_api::Flow {
            self.0.store(true, Ordering::Relaxed);
            *event.position = [32, 73, 16];
            mod_api::Flow::Cancel
        }
    }

    let fired = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![ModEntry::new(
            "spawn",
            Box::new(ReplaceSpawn(fired.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                game.world_debug = true;
                assert!(game.level.world_spawn.is_none());
                game.open_configured_levels(&[Dim::Overworld]).unwrap();
                assert_eq!(game.level.world_spawn, Some([0, 80, 0]));
                assert_eq!(game.level.world_spawn_angle, [0.0, 0.0]);
            })
        },
    );
    assert!(!fired.load(Ordering::Relaxed));
}

/// saveEverything saves the placed players, without removing them, before LevelEvent.Save.
#[test]
#[ignore = "daily"]
fn full_saves_write_live_players_before_saving_levels() {
    use sunrise_client::{lan::playerdata, save};

    #[derive(Debug, PartialEq)]
    enum Notice {
        Start(bool),
        Player(u128),
        Level(Option<i32>, Option<i32>),
        End(bool),
    }

    struct Log {
        directory: std::path::PathBuf,
        notices: Arc<Mutex<Vec<Notice>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn world_save_start(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::WorldSaveStartEvent,
        ) {
            self.notices.lock().unwrap().push(Notice::Start(event.wait));
        }
        fn player_save_to_file(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::PlayerSaveToFileEvent<'_>,
        ) {
            self.notices
                .lock()
                .unwrap()
                .push(Notice::Player(event.player));
        }
        fn level_save(&self, _: &mut dyn ServerHost, _: &mut mod_api::LevelSaveEvent<'_>) {
            let xp = |uuid| {
                let tag = save::read_dat(&playerdata::path(&self.directory, uuid))?;
                match tag.get("XpLevel") {
                    Some(sunrise_core::nbt::Tag::Int(xp)) => Some(*xp),
                    _ => None,
                }
            };
            self.notices
                .lock()
                .unwrap()
                .push(Notice::Level(xp(12), xp(13)));
        }
        fn world_saved(&self, _: &mut dyn ServerHost, event: &mut mod_api::WorldSavedEvent) {
            self.notices.lock().unwrap().push(Notice::End(event.wait));
        }
    }

    for role in [
        ExecutionRole::DedicatedServer,
        ExecutionRole::IntegratedServer,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        with_mods(
            vec![ModEntry::new(
                "save",
                Box::new(Log {
                    directory: dir.path().to_owned(),
                    notices: notices.clone(),
                }),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.join_world_rest().unwrap();
                    game.options.execution_role = role;
                    game.world_root = dir.path().to_owned();
                    let host = game.lan.as_mut().unwrap();
                    if role == ExecutionRole::IntegratedServer {
                        host.owner = Some(crate::game::lan::Owner {
                            uuid: 12,
                            saved: None,
                        });
                        game.level.player_uuid = Some(playerdata::uuid_ints(12));
                    }
                    host.hub
                        .update_guest(12, |guest| guest.xp_level = 7)
                        .unwrap();
                    host.hub
                        .update_guest(13, |guest| guest.xp_level = 9)
                        .unwrap();
                    game.native_level_opened(Dim::Overworld);
                    game.save_game_with(false).unwrap();
                    assert_eq!(game.lan.as_ref().unwrap().hub.online_players().count(), 2);
                })
            },
        );
        assert_eq!(
            *notices.lock().unwrap(),
            [
                Notice::Start(false),
                Notice::Player(12),
                Notice::Player(13),
                Notice::Level(Some(7), Some(9)),
                Notice::End(false),
            ]
        );
    }
}

/// A `world_save_start` listener cannot re-enter the save cycle via `/save-all`; the nested
/// command fails before notifying or writing, and the outer cycle finishes once.
#[test]
#[ignore = "daily"]
fn world_save_start_rejects_nested_save_all_command() {
    crate::game::in_own_process(
        "game::server_lifecycle::tests::world_save_start_rejects_nested_save_all_command",
        world_save_start_nested_save_all,
    );
}

fn world_save_start_nested_save_all() {
    use std::sync::atomic::AtomicUsize;

    struct ReenterSave {
        starts: Arc<AtomicUsize>,
        saved: Arc<AtomicUsize>,
        nested_ok: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ReenterSave {
        fn world_save_start(
            &self,
            host: &mut dyn ServerHost,
            _: &mut mod_api::WorldSaveStartEvent,
        ) {
            if self.starts.fetch_add(1, Ordering::SeqCst) == 0 {
                let ok = host.commands().run_command("save-all");
                self.nested_ok.store(ok, Ordering::SeqCst);
            }
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.saved.fetch_add(1, Ordering::SeqCst);
        }
    }

    let root = tempfile::tempdir().unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let saved = Arc::new(AtomicUsize::new(0));
    let nested_ok = Arc::new(AtomicBool::new(true));
    let mods = vec![ModEntry::new(
        "reenter_save",
        Box::new(ReenterSave {
            starts: starts.clone(),
            saved: saved.clone(),
            nested_ok: nested_ok.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        crate::game::visibility_unwind_tests::with_save_game(root.path(), |game| {
            game.join_world_rest().unwrap();
            game.save_game().unwrap();
            assert!(!nested_ok.load(Ordering::SeqCst));
            assert_eq!(starts.load(Ordering::SeqCst), 1);
            assert_eq!(saved.load(Ordering::SeqCst), 1);
            assert!(!game.world_save_starting);
            assert!(game.native_run_command("save-all"));
            assert_eq!(starts.load(Ordering::SeqCst), 2);
            assert_eq!(saved.load(Ordering::SeqCst), 2);
        });
    });
}
