use super::*;
use crate::game::native_entity_refs::tests::run_fresh_cases;
use mod_api::{
    DatapackEntry, DatapackField, DatapackFieldKind, DatapackReadError as E, DatapackRegistrySync,
    LevelRef, Registrar, RegistrationContext, WorldEpoch,
};
struct SyncedRegistryMod;
#[mod_api::__subscriptions]
impl Hooks for SyncedRegistryMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::Required,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
#[test]
#[ignore = "daily"]
fn native_datapack_host_reads_enforce_level_session_and_generation() {
    // The mock server and client each use a process-wide registry publication.
    if run_fresh_cases(
        "game::server_reload::tests::datapack_publication_tests::datapack_host_read_tests::native_datapack_host_reads_enforce_level_session_and_generation",
        "SUNRISE_DATAPACK_HOST_READ_CHILD",
        &["host-reads"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(SyncedRegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                let candidate = |power: i64| {
                    mod_api::DatapackCandidate::build(
                        mod_api::__private::datapack_registry_declarations(),
                        &[
                            (
                                "data/registry_test/registry_test/spell/fire.json".into(),
                                vec![(
                                    "pack".to_owned(),
                                    format!(r#"{{"power":{power}}}"#).into_bytes(),
                                )],
                            ),
                            (
                                "data/addon/registry_test/spell/ice.json".into(),
                                vec![("pack".to_owned(), br#"{"power":3}"#.to_vec())],
                            ),
                        ],
                    )
                    .unwrap()
                };
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(candidate(7)),
                ));
                let level = game.level(game.dim.id()).unwrap();
                assert_eq!(
                    level.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["addon:ice".into(), "registry_test:fire".into()])
                );
                let published = game
                    .mods
                    .datapack_registries
                    .as_ref()
                    .unwrap()
                    .get("registry_test:spell", "registry_test:fire")
                    .unwrap();
                let mut visits = 0;
                level
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert!(std::ptr::eq(entry, published));
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                            visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(visits, 1);
                let mut visited_refusal = false;
                let mut unexpected_visit = |_: &DatapackEntry| visited_refusal = true;
                assert_eq!(level.datapack_entries(game, "missing:registry"), None);
                assert_eq!(
                    level.datapack_read(
                        game,
                        "missing:registry",
                        "addon:ice",
                        &mut unexpected_visit,
                    ),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_read(
                        game,
                        "registry_test:spell",
                        "addon:missing",
                        &mut unexpected_visit,
                    ),
                    Err(E::MissingEntry)
                );
                for refused in [
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                    LevelRef::with_generation(
                        level.epoch(),
                        level.dimension(),
                        level.generation() + 1,
                    ),
                    LevelRef::new(
                        level.epoch(),
                        mod_api::__private::dimension_key("registry_test:unopened"),
                    ),
                ] {
                    assert_eq!(refused.datapack_entries(game, "registry_test:spell"), None);
                    assert_eq!(
                        refused.datapack_read(
                            game,
                            "registry_test:spell",
                            "registry_test:fire",
                            &mut unexpected_visit,
                        ),
                        Err(E::UnknownRegistry)
                    );
                }
                // World open already parked the Nether. Its handle shares the server's registries.
                let parked = game.level("minecraft:the_nether").unwrap();
                assert_ne!(parked.dimension(), level.dimension());
                assert_eq!(
                    parked.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["addon:ice".into(), "registry_test:fire".into()])
                );
                let mut parked_visits = 0;
                parked
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                            parked_visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(parked_visits, 1);
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(candidate(9)),
                ));
                let mut reloaded_visits = 0;
                level
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(9))]);
                            reloaded_visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(reloaded_visits, 1);
                // The client must use the received snapshot, not this Game's server set.
                let declarations = mod_api::__private::datapack_registry_declarations();
                let remote = mod_api::DatapackCandidate::build(
                    declarations,
                    &[(
                        "data/registry_test/registry_test/spell/fire.json".into(),
                        vec![("pack".to_owned(), br#"{"power":19}"#.to_vec())],
                    )],
                )
                .unwrap();
                sunrise_client::datapack_net::serve(Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(remote),
                )));
                let offer = sunrise_client::datapack_net::offer().unwrap();
                let sunrise_client::datapack_net::Step::Send(bytes) =
                    sunrise_client::datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("snapshot required");
                };
                sunrise_client::datapack_net::clear_received();
                sunrise_client::datapack_net::accept(&bytes).unwrap();
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                assert_eq!(parked.datapack_entries(game, "registry_test:spell"), None);
                assert_eq!(
                    parked.datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut unexpected_visit,
                    ),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["registry_test:fire".into()])
                );
                let mut visits = 0;
                level
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            // Clearing takes the publication lock and drops its reference to the set.
                            sunrise_client::datapack_net::clear_received();
                            assert_eq!(entry.get("power"), Some(&DatapackValue::Int(19)));
                            visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(visits, 1);
                assert_eq!(
                    level.datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut unexpected_visit,
                    ),
                    Err(E::UnknownRegistry)
                );
                assert!(!visited_refusal);
            });
        },
    );
}
