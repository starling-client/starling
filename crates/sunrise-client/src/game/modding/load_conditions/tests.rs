use crate::game::native_entity_refs::tests::run_fresh_cases;
use mod_api::__private::{game::RegistrationContextNew, reload::ReloadStageError};
use mod_api::{
    ConditionError, Hooks, Identifier, LoadCondition, ModEntry, RegisterLoadConditionsEvent,
    RegistrationContext,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Flag(Arc<AtomicUsize>, Arc<AtomicUsize>);
impl LoadCondition for Flag {
    fn test(&self, args: &Value) -> std::result::Result<bool, ConditionError> {
        assert!(args.get("type").is_none());
        self.0.fetch_add(1, Ordering::SeqCst);
        if args["value"] == "content" {
            return Ok(self.1.load(Ordering::SeqCst) == 0);
        }
        if args["value"] == "panic" {
            panic!("fixture panic");
        }
        args["value"].as_bool().ok_or_else(|| ConditionError {
            message: "value must be a boolean".into(),
        })
    }
}
struct Registrant {
    count: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl Hooks for Registrant {
    fn register_load_conditions(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterLoadConditionsEvent<'_>,
    ) {
        event
            .conditions
            .register(
                Identifier::parse("testmod:flag").unwrap(),
                Flag(self.count.clone(), self.mode.clone()),
            )
            .unwrap();
    }
    fn data_pack_files(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataPackFilesEvent<'_>,
    ) {
        let yes = match self.mode.load(Ordering::SeqCst) {
            1 => json!({"type":"testmod:nope"}),
            2 => json!({"type":"testmod:flag","value":"bad"}),
            3 => json!({"type":"testmod:flag","value":"panic"}),
            _ => json!({"type":"testmod:flag","value":true}),
        };
        if self.mode.load(Ordering::SeqCst) == 5 {
            event.files.push((
                "advancement/testmod:bad".into(),
                br#"{"neoforge:conditions":[{"type":"testmod:nope"}]}"#.to_vec(),
            ));
        }
        for (name, condition) in [
            ("enabled", yes),
            ("disabled", json!({"type":"testmod:flag","value":false})),
            (
                "not_false",
                json!({"type":"neoforge:not","value":{"type":"testmod:flag","value":false}}),
            ),
            (
                "not_true",
                json!({"type":"neoforge:not","value":{"type":"testmod:flag","value":true}}),
            ),
            (
                "nested",
                json!({"type":"neoforge:and","values":[{"type":"neoforge:always"},{"type":"neoforge:or","values":[{"type":"testmod:flag","value":true}]}]}),
            ),
        ] {
            let mut recipe = json!({
                "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:flint"],
                "result":{"id":"minecraft:stick"}, "neoforge:conditions":[condition]
            });
            if name == "enabled" {
                match self.mode.load(Ordering::SeqCst) {
                    6 => recipe["result"] = Value::Null,
                    7 => recipe["ingredients"] = json!([{"type":"testmod:unregistered"}]),
                    8 => {
                        recipe["type"] = json!("minecraft:stonecutting");
                        recipe["ingredient"] = json!(42);
                    }
                    _ => {}
                }
            }
            event.files.push((
                format!("recipe/testmod:{name}"),
                serde_json::to_vec(&recipe).unwrap(),
            ));
        }
    }
}
#[test]
#[ignore = "daily"]
fn malformed_known_recipe_stops_dedicated_world_startup() {
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: Arc::new(AtomicUsize::new(0)),
                mode: Arc::new(AtomicUsize::new(6)),
            }),
        )]
        .into(),
        || {
            // The world-open consumer used by dedicated startup must stop
            // before publishing any candidate recipes.
            crate::game::native_entity_refs::tests::with_game_role(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap()),
                crate::game::execution_role::ExecutionRole::DedicatedServer,
                |game| {
                    let mut request = crate::game::world_loading::WorldRequest::new(
                        "bad-recipes",
                        Some("test"),
                        Some(1),
                        true,
                        None,
                        Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
                    )
                    .unwrap();
                    request.save_root = scratch.path().join("refused-startup");
                    let error = game.load_world(request).err().unwrap().to_string();
                    assert!(
                        error.contains("data/testmod/recipe/enabled.json"),
                        "{error}"
                    );
                    assert!(error.contains("invalid result"), "{error}");
                    assert!(game.recipes().recipe("testmod:enabled").is_none());
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn load_conditions_recipe_reload_counts_and_failed_candidates_keep_old_recipes() {
    if run_fresh_cases(
        "game::load_conditions::tests::load_conditions_recipe_reload_counts_and_failed_candidates_keep_old_recipes",
        "SUNRISE_LOAD_CONDITIONS_RELOAD_CHILD",
        &["reload"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: count.clone(),
                mode: mode.clone(),
            }),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let mut request = crate::game::world_loading::WorldRequest::new(
                    "conditions",
                    Some("test"),
                    Some(1),
                    true,
                    None,
                    Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
                )
                .unwrap();
                request.save_root = scratch.path().join("save");
                let module = scratch.path().join("content");
                std::fs::create_dir_all(module.join("data/testmod/tags/block")).unwrap();
                std::fs::write(module.join("sunrise-mod.toml"), "id='testmod'\nname='Conditions fixture'\nversion='1'\napi='0.2'\nside='both'\ncapabilities=['registries']\n").unwrap();
                std::fs::write(module.join("data/testmod/tags/block/flag.json"), r#"{"neoforge:conditions":[{"type":"testmod:flag","value":"content"}],"values":["minecraft:stone"]}"#).unwrap();
                // Tags deliberately ignore load conditions. A recipe exercises
                // the supported content-file gate alongside the five native files.
                std::fs::create_dir_all(module.join("data/testmod/recipe")).unwrap();
                std::fs::write(module.join("data/testmod/recipe/content.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"testmod:flag","value":"content"}]}"#).unwrap();
                game.mods.selected = vec![sunrise_mod_package::selection::SelectedPackage {
                    package: sunrise_mod_package::Package::read(&module).unwrap(),
                    path: module.clone(),
                }]
                .into();
                game.mods.dev.push(module);
                game.load_world(request).unwrap();
                assert!(game.items().block_tags["testmod:flag"].contains("minecraft:stone"));
                assert!(game.recipes().recipe("testmod:content").is_some());
                assert!(game.recipes().recipe("testmod:enabled").is_some());
                assert!(game.recipes().recipe("testmod:disabled").is_none());
                assert!(game.recipes().recipe("testmod:nested").is_some());
                for _ in 0..2 {
                    count.store(0, Ordering::SeqCst);
                    let stack = game
                        .reload_datapacks(true, &game.level.enabled_features.clone())
                        .unwrap();
                    assert_eq!(count.load(Ordering::SeqCst), 6);
                    assert!(game.recipes().recipe("testmod:content").is_some());
                    assert!(game.recipes().recipe("testmod:not_false").is_some());
                    assert!(game.recipes().recipe("testmod:not_true").is_none());
                    assert!(game.recipes().recipe("testmod:nested").is_some());
                    let conditional_recipe =
                        json!({"neoforge:conditions":[{"type":"testmod:flag","value":"content"}]});
                    mode.store(4, Ordering::SeqCst);
                    game.prepare_pack_condition_reload(&stack).unwrap();
                    let attempt = game.mods.load_condition_candidate.as_ref().unwrap();
                    assert!(
                        !attempt
                            .allows_file(&conditional_recipe, "data/testmod/recipe/content.json")
                    );
                    assert!(!attempt.allows_file(
                        &json!({"neoforge:conditions":[{"type":"testmod:nope"}]}),
                        "failed-pack.json"
                    ));
                    assert!(attempt.check_errors().is_err());
                    mode.store(0, Ordering::SeqCst);
                    game.prepare_pack_condition_reload(&stack).unwrap();
                    let recovered = game.mods.load_condition_candidate.as_ref().unwrap();
                    assert_eq!(recovered.check_errors(), Ok(()));
                    assert!(
                        recovered
                            .allows_file(&conditional_recipe, "data/testmod/recipe/content.json")
                    );
                }
                mode.store(4, Ordering::SeqCst);
                count.store(0, Ordering::SeqCst);
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                assert_eq!(count.load(Ordering::SeqCst), 6);
                assert!(
                    game.items()
                        .block_tags
                        .get("testmod:flag")
                        .is_some_and(|tag| tag.contains("minecraft:stone"))
                );

                assert!(game.recipes().recipe("testmod:content").is_none());
                let old: Vec<_> = game
                    .recipes()
                    .crafting
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect();
                let old_generation = game.recipes().by_type["minecraft:crafting"].clone();
                let grid = [Some(sunrise_client::items::ItemStack::new(
                    "minecraft:flint",
                    1,
                ))];
                for (bad, expected) in [
                    (1, "testmod:nope"),
                    (2, "value must be a boolean"),
                    (3, "handler panicked"),
                    (6, "invalid result"),
                    (7, "ingredient kind testmod:unregistered"),
                    (8, "invalid ingredient: 42"),
                ] {
                    mode.store(bad, Ordering::SeqCst);
                    let error = game
                        .reload_datapacks(true, &game.level.enabled_features.clone())
                        .err()
                        .unwrap();
                    let error = match (bad, error) {
                        (8, ReloadStageError::DataSource(message)) => message,
                        (8, other) => panic!("expected a recipe data-source error, got {other:?}"),
                        (_, other) => format!("{other:?}"),
                    };
                    assert!(
                        error.contains("data/testmod/recipe/enabled.json"),
                        "{error}"
                    );
                    assert!(error.contains(expected), "{error}");
                    if bad == 2 || bad == 3 {
                        assert!(error.contains("testmod:flag"), "{error}");
                    }
                    if bad == 3 {
                        assert!(error.contains("mod testmod"), "{error}");
                    }
                    assert_eq!(
                        game.recipes()
                            .crafting
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>(),
                        old
                    );
                    assert!(game.recipes().recipe("testmod:enabled").is_some());
                    assert!(Arc::ptr_eq(
                        &old_generation,
                        &game.recipes().by_type["minecraft:crafting"],
                    ));
                    assert!(game.recipes().recipe_matches(
                        &game.loop_context,
                        "testmod:enabled",
                        &grid,
                        1,
                    ));
                    assert_eq!(
                        game.recipes()
                            .craft(&game.loop_context, &grid, 1)
                            .unwrap()
                            .item
                            .as_ref(),
                        "minecraft:stick",
                    );
                }
                let host = game.lan.take();
                crate::game::lan::sync::forget_sync_data();
                assert!(crate::game::lan::sync::cached_reload_sync().is_none());
                mode.store(5, Ordering::SeqCst);
                let error = format!(
                    "{:?}",
                    game.reload_datapacks(true, &game.level.enabled_features.clone())
                        .err()
                        .unwrap()
                );
                assert!(
                    error.contains("data/testmod/advancement/bad.json"),
                    "{error}"
                );
                assert_eq!(
                    game.recipes()
                        .crafting
                        .iter()
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>(),
                    old
                );
                game.lan = host;
                mode.store(0, Ordering::SeqCst);
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                assert!(game.recipes().recipe("testmod:enabled").is_some());
                assert!(game.recipes().recipe("testmod:content").is_some());
                assert!(!Arc::ptr_eq(
                    &old_generation,
                    &game.recipes().by_type["minecraft:crafting"],
                ));
            });
        },
    );
}
struct NamedCondition(&'static str);
#[mod_api::__subscriptions]
impl Hooks for NamedCondition {
    fn register_load_conditions(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterLoadConditionsEvent<'_>,
    ) {
        let _ = event.conditions.register(
            Identifier::parse(self.0).unwrap(),
            Flag(Arc::default(), Arc::default()),
        );
    }
}
#[test]
fn load_conditions_registration_allows_shared_namespaces() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:y"))),
            ModEntry::new("b", Box::new(NamedCondition("a:x"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            mod_api::dispatch_register_load_conditions(&RegistrationContext::new(), |listen| {
                registrar.register_listener(listen)
            });
            let entries = registrar.finish().unwrap();
            assert_eq!(entries.len(), 2);
            for (name, owner, value) in [("a:y", "a", true), ("a:x", "b", false)] {
                let (registered_owner, handler) =
                    entries.get(&Identifier::parse(name).unwrap()).unwrap();
                assert_eq!(registered_owner, owner);
                assert_eq!(handler.test(&json!({"value": value})), Ok(value));
            }
        },
    );
}

#[test]
fn load_conditions_registration_refuses_duplicate_ids_even_if_ignored() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:x"))),
            ModEntry::new("b", Box::new(NamedCondition("a:x"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            mod_api::dispatch_register_load_conditions(&RegistrationContext::new(), |listen| {
                registrar.register_listener(listen)
            });
            let error = registrar.finish().err().unwrap().message;
            assert!(error.contains("condition a:x"), "{error}");
            assert!(error.contains("both a and b"), "{error}");
        },
    );
}

#[test]
fn a_panicking_load_condition_registrar_leaves_neither_handlers_nor_errors() {
    use std::sync::atomic::AtomicBool;

    struct Failing(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Failing {
        fn register_load_conditions(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterLoadConditionsEvent<'_>,
        ) {
            let handler = || Flag(Arc::default(), Arc::default());
            let shared = Identifier::parse("a:shared").unwrap();
            event
                .conditions
                .register(shared.clone(), handler())
                .unwrap();
            event
                .conditions
                .register(Identifier::parse("a:leaked").unwrap(), handler())
                .unwrap();
            assert_eq!(
                event
                    .conditions
                    .register(shared, handler())
                    .unwrap_err()
                    .message,
                "condition a:shared registered by both a and a",
            );
            self.0.store(true, Ordering::Relaxed);
            panic!("load condition registrar fixture failed");
        }
    }

    let reached = Arc::new(AtomicBool::new(false));
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(Failing(reached.clone()))),
            ModEntry::new("b", Box::new(NamedCondition("a:shared"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            super::Game::native_register_load_conditions(
                &RegistrationContext::new(),
                &mut registrar,
            );
            assert!(reached.load(Ordering::Relaxed));
            let entries = registrar.finish().unwrap();
            assert_eq!(entries.len(), 1);
            let (owner, handler) = &entries[&Identifier::parse("a:shared").unwrap()];
            assert_eq!(owner, "b");
            assert_eq!(handler.test(&json!({"value": false})), Ok(false));
        },
    );
}

#[test]
fn load_conditions_refuse_a_builtin_id_at_startup_even_if_ignored() {
    for name in ["neoforge:always", "neoforge:never", "neoforge:mod_loaded"] {
        mod_platform::testing::with_mods(
            vec![ModEntry::new("compat", Box::new(NamedCondition(name)))].into(),
            || {
                let sources = crate::game::mods::ModsState::default();
                let inventory = sunrise_mod_package::enabled::EnabledMods::default();
                let error = super::conditions_for_mods(&sources, &inventory).unwrap_err();
                assert_eq!(
                    error,
                    format!("mod compat cannot register condition {name}: that ID is built in")
                );
            },
        );
    }
}

#[test]
fn load_conditions_admit_registered_foreign_ids_but_refuse_every_unknown_id() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:x"))),
            ModEntry::new("b", Box::new(NamedCondition("a:z"))),
            ModEntry::new("compat", Box::new(NamedCondition("c:config"))),
            ModEntry::new("other", Box::new(NamedCondition("neoforge:custom"))),
        ]
        .into(),
        || {
            let sources = crate::game::mods::ModsState::default();
            let inventory = sunrise_mod_package::enabled::EnabledMods::default();
            let conditions = super::conditions_for_mods(&sources, &inventory).unwrap();
            let file = |id: &str| json!({"neoforge:conditions":[{"type":id, "value":true}]});
            assert!(conditions.allows_file(&file("c:config"), "data/demo/advancement/known.json"));
            assert!(conditions.allows_file(
                &file("neoforge:custom"),
                "data/demo/advancement/custom.json"
            ));
            for namespace in ["a", "c", "neoforge"] {
                let id = format!("{namespace}:missing");
                let path = format!("data/demo/advancement/{namespace}.json");
                assert!(!conditions.allows_file(&file(&id), &path));
                let error = conditions.check_errors().unwrap_err();
                assert!(
                    error.contains(&format!("{path}: unknown load conditions [{id}]")),
                    "{error}"
                );
            }
        },
    );
}

#[test]
fn load_conditions_disabled_owner_does_not_execute_registered_handler() {
    let count = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: count.clone(),
                mode: Arc::default(),
            }),
        )]
        .into(),
        || {
            let sources = crate::game::mods::ModsState::default();
            let inventory = sunrise_mod_package::enabled::EnabledMods::default();
            let file = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
            assert!(
                super::conditions_for_mods(&sources, &inventory)
                    .unwrap()
                    .allows_file(&file, "flag.json")
            );
            assert!(mod_api::__private::set_mod_enabled("testmod", false));
            let disabled = super::conditions_for_mods(&sources, &inventory).unwrap();
            assert!(!disabled.allows_file(&file, "flag.json"));
            assert_eq!(disabled.check_errors(), Ok(()));
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert!(mod_api::__private::set_mod_enabled("testmod", true));
            assert!(
                super::conditions_for_mods(&sources, &inventory)
                    .unwrap()
                    .allows_file(&file, "flag.json")
            );
            assert_eq!(count.load(Ordering::SeqCst), 2);
        },
    );
}

/// `neoforge:registered` reads the candidate's registries: the built-in entries the prepared root
/// exports and what this load's packs add, so a new pack entry counts on its first load and one
/// a pack dropped stops counting, whatever an earlier load put in the game's registries.
#[test]
fn registered_reads_the_candidate_packs_not_the_last_load() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("command-registries.json"),
        json!({"by_registry": {
            "minecraft:item": {"ids": ["minecraft:stone"]},
            "minecraft:worldgen/biome": {"ids": ["minecraft:plains"]}
        }})
        .to_string(),
    )
    .unwrap();
    let pack = |keys: &[&str]| -> Vec<(String, Vec<Vec<u8>>)> {
        keys.iter()
            .map(|key| ((*key).to_owned(), vec![b"{}".to_vec()]))
            .collect()
    };
    let registered = |registry: &str, value: &str| {
        json!({"neoforge:conditions": [
            {"type": "neoforge:registered", "registry": registry, "value": value}
        ]})
    };
    let dunes = registered("minecraft:worldgen/biome", "test:dunes");
    let admission = sunrise_world::conditions::LoadConditions::new([]);
    let mut files = pack(&["worldgen/biome/test:dunes", "recipe/test:a/b"]);
    // An entry its own conditions turn off is not in the registry.
    files.push((
        "worldgen/biome/test:off".into(),
        vec![br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#.to_vec()],
    ));
    let first = super::candidate_registries(root.path(), &files, &admission);
    let first = sunrise_world::conditions::LoadConditions::new([]).with_registries(
        first
            .iter()
            .map(|(registry, ids)| (registry.as_str(), ids.iter().map(String::as_str))),
    );
    assert!(
        first.allows(&dunes),
        "a pack's new biome counts on its first load"
    );
    assert!(first.allows(&registered("minecraft:worldgen/biome", "minecraft:plains")));
    assert!(!first.allows(&registered("minecraft:worldgen/biome", "test:off")));
    assert!(first.allows(&registered("minecraft:item", "minecraft:stone")));
    let next = super::candidate_registries(root.path(), &pack(&[]), &admission);
    let next = sunrise_world::conditions::LoadConditions::new([]).with_registries(
        next.iter()
            .map(|(registry, ids)| (registry.as_str(), ids.iter().map(String::as_str))),
    );
    assert!(
        !next.allows(&dunes),
        "the pack no longer has it: not registered"
    );
    assert!(next.allows(&registered("minecraft:worldgen/biome", "minecraft:plains")));
}
