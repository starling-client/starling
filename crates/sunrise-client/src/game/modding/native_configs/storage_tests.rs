use super::*;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sunrise-config-storage-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn chunk_disk_uses_the_captured_native_config_root() {
    use mod_api::{
        __private::{RegistrationData, WorldgenReader, game::RegistrationContextNew},
        ChunkDiskContext, ChunkDiskKind, ChunkDiskPhase, Dist, Identifier, InstanceDirectory,
        OwningChunkDisk, OwningWorldgen, RegistrationContext, RuntimeEnvironment, WorldEpoch,
        prelude::ConfigHost,
    };
    use std::collections::BTreeMap;

    struct NoWorldgen;
    impl WorldgenReader for NoWorldgen {
        fn registry(&self, _: &str) -> Option<&[String]> {
            None
        }
        fn setting(&self, _: &str, _: &str) -> Option<&str> {
            None
        }
    }

    Game::initialize_mod_runtime_for_tests();
    let root = ScratchDir::new("startup-host-config-dir");
    let data = RegistrationData::new(
        BTreeMap::new(),
        BTreeMap::new(),
        InstanceDirectory::from_path(&root.0).unwrap(),
        RuntimeEnvironment::new(Dist::Client, "26.3".into(), "test".into(), false),
    );

    mod_platform::__private::with_test_registration_data(data, || {
        assert_eq!(
            RegistrationContext::new()
                .instance_dir()
                .unwrap()
                .to_path_buf(),
            Some(root.0.clone())
        );
        let captured = ChunkDiskContext::new(
            WorldEpoch::new(1),
            Identifier::parse("minecraft:overworld").unwrap(),
            [0, 0],
            ChunkDiskKind::Full,
            ChunkDiskPhase::Save,
        );
        let resources = OwningWorldgen::new(&NoWorldgen, &|_, _, _| None, &|_, _| None);
        let chunk_disk = OwningChunkDisk::new(&captured, &resources, |_, _, nbt| Ok(nbt));
        assert_eq!(chunk_disk.config_dir(), root.0.join("config"));
    });
}

#[cfg(unix)]
#[test]
fn dangling_serverconfig_override_symlink_falls_through_to_base_config() {
    let root = ScratchDir::new("dangling-override");
    let config = root.0.join("config");
    let world = root.0.join("saves/W");
    let base_dangling = config.join("alpha/dangling.toml");
    let base_working = config.join("alpha/working.toml");
    let override_dangling = world.join("serverconfig/alpha/dangling.toml");
    let override_working = world.join("serverconfig/alpha/working.toml");
    let working_target = root.0.join("working-override.toml");

    fs::create_dir_all(base_dangling.parent().unwrap()).unwrap();
    fs::create_dir_all(override_dangling.parent().unwrap()).unwrap();
    fs::write(&base_dangling, "base = 1\n").unwrap();
    fs::write(&base_working, "base_working = 0\n").unwrap();
    fs::write(&working_target, "override = 1\n").unwrap();
    std::os::unix::fs::symlink(root.0.join("missing-target.toml"), &override_dangling).unwrap();
    std::os::unix::fs::symlink(&working_target, &override_working).unwrap();

    super::tests::with_registered_configs(
        &[(
            "alpha",
            &[
                (
                    ConfigKind::Server,
                    Some("alpha/dangling.toml"),
                    "default = 0\n",
                ),
                (
                    ConfigKind::Server,
                    Some("alpha/working.toml"),
                    "default = 0\n",
                ),
            ],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[ConfigKind::Server], &config, Some(&world));
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded[0].1.path, base_dangling);
            assert_eq!(loaded[0].1.text, "base = 1\n");
            assert_eq!(loaded[1].1.path, override_working);
            assert_eq!(loaded[1].1.text, "override = 1\n");

            fs::write(&base_dangling, "base = 2\n").unwrap();
            fs::write(&working_target, "override = 2\n").unwrap();
            let reloaded = configs.changed();
            assert_eq!(reloaded.len(), 2);
            assert_eq!(reloaded[0].1.path, base_dangling);
            assert_eq!(reloaded[0].1.text, "base = 2\n");
            assert_eq!(reloaded[1].1.path, override_working);
            assert_eq!(reloaded[1].1.text, "override = 2\n");
        },
    );
}

#[test]
fn native_configs_load_and_reload_follow_native_paths_and_symlinks() {
    let root = ScratchDir::new("native-paths");
    let config = root.0.join("config");
    let link_path = config.join("other/linked.toml");
    fs::create_dir_all(link_path.parent().unwrap()).unwrap();

    #[cfg(unix)]
    let target = {
        let target = root.0.join("external.toml");
        fs::write(&target, "linked = 1\n").unwrap();
        std::os::unix::fs::symlink(&target, &link_path).unwrap();
        target
    };
    #[cfg(not(unix))]
    let target = {
        fs::write(&link_path, "linked = 1\n").unwrap();
        link_path.clone()
    };

    super::tests::with_registered_configs(
        &[(
            "alpha",
            &[(
                ConfigKind::Common,
                Some("other/linked.toml"),
                "default = 0\n",
            )],
        )],
        |registered| {
            let mut configs = NativeConfigs::default();
            let loaded = configs.load(registered, &[ConfigKind::Common], &config, None);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].1.path, link_path);
            assert_eq!(loaded[0].1.text, "linked = 1\n");

            fs::write(&target, "linked = 2\n").unwrap();
            let reloaded = configs.changed();
            assert_eq!(reloaded.len(), 1);
            assert_eq!(reloaded[0].1.path, link_path);
            assert_eq!(reloaded[0].1.text, "linked = 2\n");
        },
    );
}

/// A mod saves its registered config and a sibling file inside `ConfigContext::config_dir()`
/// from `ConfigEvent::Loading`, and the reload poll hands the saved text back.
/// Runs in a fresh process in a scratch folder since `config/` is beside the
/// game's working folder.
#[test]
#[ignore = "daily"]
fn a_mod_saves_its_config_through_the_game_and_the_reload_hears_it() {
    use mod_api::{
        ConfigEvent, ConfigKind, ConfigListener, ConfigSnapshot, ConfigSpec, Hooks, HostSide,
        ModEntry, Mods, Registrar, RegistrationContext,
    };
    use std::sync::{Arc, Mutex};
    static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn heard() -> std::sync::MutexGuard<'static, Vec<String>> {
        HEARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_configs::storage_tests::a_mod_saves_its_config_through_the_game_and_the_reload_hears_it",
        "SUNRISE_CONFIG_FILES_CHILD",
        &["game"],
    ) {
        return;
    }
    struct Proof;
    #[mod_api::__subscriptions]
    impl Hooks for Proof {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let listener: Arc<ConfigListener> =
                Arc::new(|event: ConfigEvent<'_>, config: &ConfigSnapshot| {
                    match event {
                        ConfigEvent::Loading(context) => {
                            let config_dir = context.config_dir();
                            assert_eq!(config.path, config_dir.join("proof-common.toml"));
                            heard().push(format!("loading {}", config.text.trim()));
                            fs::write(&config.path, "count = 1\n").unwrap();
                            let state = config_dir.join("proof/state.bin");
                            fs::create_dir_all(state.parent().unwrap()).unwrap();
                            fs::write(&state, [1, 2, 3]).unwrap();
                            assert_eq!(fs::read(&state).unwrap(), [1, 2, 3]);
                        }
                        ConfigEvent::Reloading(HostSide::Client(host)) => {
                            assert_eq!(
                                fs::read(host.config_dir().join("proof/state.bin")).unwrap(),
                                [1, 2, 3]
                            );
                            heard().push(format!("reloading {}", config.text.trim()));
                        }
                        _ => {}
                    }
                    Ok(())
                });
            r.register_config(
                ConfigSpec::new(ConfigKind::Common, "count = 0\n"),
                Some(listener),
            )
            .unwrap();
        }
    }
    let mods: Mods = vec![ModEntry::new("proof", Box::new(Proof))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.native_configs_client_start();
            game.native_configs_tick_client();
        });
    });
    assert_eq!(*heard(), ["loading count = 0", "reloading count = 1"]);
    let config = std::env::current_dir().unwrap().join("config");
    assert_eq!(fs::read(config.join("proof/state.bin")).unwrap(), [1, 2, 3]);
}

#[test]
#[ignore = "daily"]
fn world_bulk_files_survive_save_reload_backup_and_world_deletion() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use crate::game::world_loading::WorldRequest;
    if run_fresh_cases(
        "game::native_configs::storage_tests::world_bulk_files_survive_save_reload_backup_and_world_deletion",
        "SUNRISE_BULK_FILES_CHILD",
        &["game"],
    ) {
        return;
    }
    static WARNINGS: AtomicUsize = AtomicUsize::new(0);
    struct SettingsLog;
    impl log::Log for SettingsLog {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.level() == log::Level::Warn
                && record.target() == "sunrise_world::native_load_context"
            {
                WARNINGS.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&SettingsLog).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let scratch = ScratchDir::new("world-lifecycle");
    let saves = scratch.0.join("saves");
    let world = saves.join("Bulk World");
    let request = |create: bool| {
        let rules = std::sync::Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla());
        let options = sunrise_client::title::WorldOptions {
            world_type: sunrise_client::save::WorldPreset::Flat,
            rules: sunrise_client::gamerules::GameRules::with_registry(rules.clone(), &[]),
            ..Default::default()
        };
        let mut request = WorldRequest::new(
            "Bulk World",
            Some("Bulk World"),
            Some(1),
            true,
            create.then_some(&options),
            rules,
        )
        .unwrap();
        request.save_root = world.clone();
        request
    };
    let mut instance_file = PathBuf::new();
    with_game(|game| {
        game.world_root.clear();
        let host: &dyn mod_api::ServerHost = game;
        assert_eq!(host.world_dir(), None);

        game.load_world(request(true)).unwrap();
        let host: &dyn mod_api::ServerHost = game;
        instance_file = host.instance_dir().join("sunrise-mod-data/fd/server-cache");
        fs::create_dir_all(instance_file.parent().unwrap()).unwrap();
        fs::write(&instance_file, b"game-cache").unwrap();
        let world_file = host
            .world_dir()
            .unwrap()
            .join("sunrise-mod-data/fd/tiles/nether/0.bin");
        fs::create_dir_all(world_file.parent().unwrap()).unwrap();
        fs::write(&world_file, b"world:42").unwrap();
        assert_eq!(fs::read(&world_file).unwrap(), b"world:42");
        let settings = Game::mod_data_root();
        fs::create_dir_all(settings.join("fd")).unwrap();
        fs::write(settings.join("fd/enabled"), b"true").unwrap();
        WARNINGS.store(0, Ordering::Relaxed);
        let captured = sunrise_world::native_load_context::capture_settings(&settings, ["fd"]);
        assert_eq!(captured["fd"].len(), 1);
        assert_eq!(captured["fd"]["enabled"], "true");
        assert_eq!(WARNINGS.load(Ordering::Relaxed), 0);
        game.save_game().unwrap();
    });
    with_game(|game| {
        game.load_world(request(false)).unwrap();
        let host: &dyn mod_api::ServerHost = game;
        let tiles = host
            .world_dir()
            .unwrap()
            .join("sunrise-mod-data/fd/tiles/nether");
        assert_eq!(fs::read(tiles.join("0.bin")).unwrap(), b"world:42");
        let names: Vec<_> = fs::read_dir(&tiles)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsStr::new("0.bin")]);
    });
    let (backup, _) =
        sunrise_client::title::world_management::backup(&saves, "Bulk World").unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(backup).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name("Bulk World/sunrise-mod-data/fd/tiles/nether/0.bin")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"world:42");
    sunrise_client::title::world_management::delete(&saves, "Bulk World").unwrap();
    assert!(!world.exists());
    assert_eq!(fs::read(&instance_file).unwrap(), b"game-cache");
}
