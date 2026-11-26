//! Registration uses the actual identity of the mod whose startup hook is running.
use super::PartRegistrar;
use crate::{ConfigListener, ConfigRegistrationError, ConfigSpec, ConfigSpecsRegistrar};
use mod_vocab::__private::RegisteredConfig;
use std::sync::Arc;

impl ConfigSpecsRegistrar for PartRegistrar {
    fn register_config(
        &mut self,
        spec: ConfigSpec,
        listener: Option<Arc<ConfigListener>>,
    ) -> Result<(), ConfigRegistrationError> {
        RegisteredConfig::register(
            &mut self.configs,
            self.mod_id,
            self.mod_index,
            spec,
            listener,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConfigEvent, ConfigKind, ConfigSnapshot, Hooks, HostSide, ModEntry, Registrar,
        RegistrationContext,
        testing::{FakeHost, with_mods},
    };
    use mod_vocab::__private::parts_installation::registered_configs;
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    #[test]
    fn config_registrations_freeze_in_order_with_the_registering_owner() {
        const FILE: &str = "shared folder/é config.toml";
        const OTHER_SPELLING: &str = "shared folder/./é config.toml";

        enum TestMod {
            First,
            Second,
        }
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                match self {
                    Self::First => {
                        r.register_config(ConfigSpec::new(ConfigKind::Common, "common = 1"), None)
                            .unwrap();
                        r.register_config(
                            ConfigSpec::new(ConfigKind::Client, "client = 2").file(FILE),
                            None,
                        )
                        .unwrap();
                    }
                    Self::Second => {
                        assert_eq!(
                            r.register_config(
                                ConfigSpec::new(ConfigKind::Server, "refused = 3").file(FILE),
                                None,
                            ),
                            Err(ConfigRegistrationError::Duplicate {
                                file: PathBuf::from(FILE),
                                owner: "first",
                            })
                        );
                        r.register_config(
                            ConfigSpec::new(ConfigKind::Server, "server = 4").file(OTHER_SPELLING),
                            None,
                        )
                        .unwrap();
                    }
                }
            }
        }

        let mods: crate::Mods = vec![
            ModEntry::new("first", Box::new(TestMod::First)),
            ModEntry::new("second", Box::new(TestMod::Second)),
        ]
        .into();
        with_mods(mods.clone(), || {
            crate::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let configs = registered_configs();
            assert_eq!(
                configs
                    .iter()
                    .map(|config| (config.mod_id(), config.kind, config.default.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("first", ConfigKind::Common, "common = 1"),
                    ("first", ConfigKind::Client, "client = 2"),
                    ("second", ConfigKind::Server, "server = 4"),
                ]
            );
            assert_eq!(configs[0].file.as_os_str(), "first-common.toml");
            assert_eq!(configs[1].file.as_os_str(), FILE);
            assert_eq!(configs[2].file.as_os_str(), OTHER_SPELLING);
        });
    }

    #[test]
    fn listener_errors_leave_the_owner_bound_listener_active() {
        struct Initial;
        impl crate::ConfigContext for Initial {
            fn config_dir(&self) -> &Path {
                Path::new("config")
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&calls);
        let listener: Arc<ConfigListener> =
            Arc::new(move |event: ConfigEvent<'_>, file: &ConfigSnapshot| {
                let event = match event {
                    ConfigEvent::Loading(cx) => {
                        assert_eq!(cx.config_dir(), Path::new("config"));
                        "loading"
                    }
                    ConfigEvent::Reloading(HostSide::Server(_)) => "reloading",
                    ConfigEvent::Unloading(_) => "unloading",
                    _ => panic!("expected server config callback"),
                };
                record.lock().unwrap().push((
                    crate::content::crash::current_native_mod(),
                    event,
                    file.clone(),
                ));
                Err("test listener error".into())
            });
        let mut registrar = PartRegistrar::new("test-config-owner");
        registrar
            .register_config(
                ConfigSpec::new(ConfigKind::Server, "default text"),
                Some(listener),
            )
            .unwrap();
        let config = &registrar.configs[0];
        let mut host = FakeHost::new();
        let path = Path::new("world/serverconfig/test-config-owner-server.toml");
        let initial = config.snapshot(path.to_owned(), "loaded".into());
        let changed = config.snapshot(path.to_owned(), "changed".into());
        config.notify_snapshot(ConfigEvent::Loading(&Initial), &initial);
        config.notify_snapshot(
            ConfigEvent::Reloading(HostSide::Server(&mut host)),
            &changed,
        );
        config.notify_snapshot(ConfigEvent::Unloading(&mut host), &changed);

        let expected = [
            ("loading", "loaded"),
            ("reloading", "changed"),
            ("unloading", "changed"),
        ];
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), expected.len());
        for ((owner, event, file), (expected_event, expected_text)) in calls.iter().zip(expected) {
            assert_eq!(*owner, Some("test-config-owner"));
            assert_eq!(*event, expected_event);
            assert_eq!(file.mod_id, "test-config-owner");
            assert_eq!(file.kind, ConfigKind::Server);
            assert_eq!(file.file, Path::new("test-config-owner-server.toml"));
            assert_eq!(file.path, path);
            assert_eq!(file.text, expected_text);
        }
    }
}
