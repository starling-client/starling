use std::sync::{Arc, atomic::AtomicUsize};

struct Registrant(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Registrant {
    fn register_load_conditions(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterLoadConditionsEvent<'_>,
    ) {
        struct Flag(Arc<AtomicUsize>);
        impl mod_api::LoadCondition for Flag {
            fn test(
                &self,
                args: &mod_api::serde_json::Value,
            ) -> Result<bool, mod_api::ConditionError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(args["value"] == true)
            }
        }
        event
            .conditions
            .register(
                mod_api::Identifier::parse("testmod:flag").unwrap(),
                Flag(self.0.clone()),
            )
            .unwrap();
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
#[ignore = "daily"]
fn first_open_conditions_see_static_tags_and_admitted_biomes() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use serde_json::{Value, json};
    use std::fs;
    use sunrise_client::lan::data::BIOME_REGISTRY;

    // World opening installs part/entity/particle registries and the LAN sync cache.
    if run_fresh_cases(
        "game::load_conditions::startup_tests::first_open_conditions_see_static_tags_and_admitted_biomes",
        "SUNRISE_BIOME_ADMISSION_CHILD",
        &["world"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/conditions");
    fs::create_dir_all(&pack).unwrap();
    fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"conditions","min_format":[121,0],"max_format":[121,0]}}"#,
    )
    .unwrap();
    fs::write(
        scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/conditions"],"disabled":[]}"#,
    )
    .unwrap();
    let write = |file: &str, value: Value| {
        let path = pack.join("data/demo").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value.to_string()).unwrap();
    };
    let registered =
        |id| json!({"type":"neoforge:registered","registry":"minecraft:worldgen/biome","value":id});
    let registered_in =
        |registry, id| json!({"type":"neoforge:registered","registry":registry,"value":id});
    let tag_empty =
        |registry, tag| json!({"type":"neoforge:tag_empty","registry":registry,"tag":tag});
    let not = |condition| json!({"type":"neoforge:not","value":condition});
    with_game(|game| {
        write(
            "worldgen/biome/disabled.json",
            json!({
                "neoforge:conditions":[{"type":"neoforge:never"}],
                "neoforge:value":{}
            }),
        );
        let mut enabled = game
            .res
            .biomes()
            .builtin_file("minecraft:plains")
            .unwrap()
            .clone();
        enabled["effects"]["water_color"] = json!([1.0, 0.5, 0.0]);
        enabled["attributes"]["minecraft:visual/cloud_height"] = json!(123.25);
        enabled["attributes"]["minecraft:gameplay/increased_fire_burnout"] = json!(true);
        write(
            "worldgen/biome/enabled.json",
            json!({
                "neoforge:conditions":[not(registered("minecraft:plains"))],
                "neoforge:value":enabled
            }),
        );
        write(
            "tags/worldgen/biome/disabled.json",
            json!({"values":["demo:disabled"]}),
        );
        // A pack-added placed feature and its tag, read by downstream conditions.
        write(
            "worldgen/placed_feature/added.json",
            json!({"feature":{"type":"minecraft:no_op","config":{}},"placement":[]}),
        );
        write(
            "tags/worldgen/placed_feature/added_tag.json",
            json!({"values":["demo:added"]}),
        );
        for (id, condition) in [
            (
                "placed_feature",
                registered_in("minecraft:worldgen/placed_feature", "demo:added"),
            ),
            (
                "placed_feature_tag",
                not(tag_empty(
                    "minecraft:worldgen/placed_feature",
                    "demo:added_tag",
                )),
            ),
            (
                "static_tag",
                not(tag_empty(
                    "minecraft:point_of_interest_type",
                    "minecraft:acquirable_job_site",
                )),
            ),
            ("known_biome", registered("demo:enabled")),
            ("unknown_biome", registered("demo:disabled")),
            (
                "empty_biome_tag",
                tag_empty("minecraft:worldgen/biome", "demo:disabled"),
            ),
        ] {
            write(
                &format!("recipe/{id}.json"),
                json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:flint"],
                    "result":{"id":"minecraft:stick"}, "neoforge:conditions":[condition]
                }),
            );
        }
        let mut request = crate::game::world_loading::WorldRequest::new(
            "condition-winners",
            Some("test"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().to_owned();
        game.load_world(request).unwrap();
        let biomes = game.world_biomes_base.as_ref().unwrap();
        assert!(biomes.contains("demo:enabled"));
        assert_eq!(biomes.get(biomes.id("demo:enabled")).water, 0xff7f00);
        let local = biomes.pack_file("demo:enabled").unwrap();
        assert!(local["features"].is_array());
        assert_eq!(
            local["attributes"]["minecraft:gameplay/increased_fire_burnout"],
            true
        );
        assert!(!biomes.contains("demo:disabled"));
        for id in [
            "demo:static_tag",
            "demo:known_biome",
            "demo:empty_biome_tag",
            "demo:placed_feature",
            "demo:placed_feature_tag",
        ] {
            assert!(
                game.recipes().recipe(id).is_some(),
                "missing admitted recipe {id}"
            );
        }
        assert!(game.recipes().recipe("demo:unknown_biome").is_none());
        let sync = crate::game::lan::sync::sync_data(
            &game.options.resources,
            &game.res.core.item_data,
            game.items(),
            &game.pack_files(),
        )
        .unwrap();
        let packet = sync
            .registry_packets(true, &game.level_shapes)
            .into_iter()
            .find(|packet| packet.registry == BIOME_REGISTRY)
            .unwrap();
        assert!(
            packet
                .entries
                .iter()
                .all(|entry| entry.id != "demo:disabled")
        );
        let wire = packet
            .entries
            .into_iter()
            .find(|entry| entry.id == "demo:enabled")
            .unwrap()
            .data
            .unwrap();
        let wire = sunrise_client::scoreboard::tag_to_json(&wire);
        assert_eq!(wire["effects"]["water_color"], "#ff7f00");
        assert!(wire.get("neoforge:value").is_none());
        assert!(wire.get("carvers").is_none());
        assert!(wire.get("features").is_none());
        assert_eq!(wire["attributes"]["minecraft:visual/cloud_height"], 123.25);
        assert!(
            wire["attributes"]
                .get("minecraft:gameplay/increased_fire_burnout")
                .is_none()
        );
    });
}

/// A private copy of the prepared resources, which the test may write into.
fn scratch_resources() -> (tempfile::TempDir, std::path::PathBuf) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("resources");
    copy_tree(
        &std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap()),
        &root,
    );
    (scratch, root)
}

fn load_startup(root: &std::path::Path) -> sunrise_client::Result<crate::game::Loaded> {
    crate::game::resource_load::load_resources_for_mods(
        sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()).into(),
        Vec::new(),
        None,
        root,
        &crate::game::Settings::default(),
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &crate::game::Progress::new(),
        &mut |_, _| unreachable!(),
        &mut || unreachable!("the server has no client pack selection"),
        &crate::game::mods::ModsState::default(),
    )
}

#[test]
#[ignore = "daily"]
fn startup_recipe_uses_registered_condition() {
    let (_scratch, root) = scratch_resources();
    // Both bootstrap readers see a provisional registry. A world's packs can
    // supply these entries later, so neither may refuse the shared bootstrap.
    for (kind, definition) in [
        (
            "damage_type",
            r#"{"message_id":"condition_startup","scaling":"never","exhaustion":0}"#,
        ),
        (
            "instrument",
            r#"{"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":7,"range":256}"#,
        ),
    ] {
        let folder = root.join("data/testmod").join(kind);
        std::fs::create_dir_all(&folder).unwrap();
        let mut file: mod_api::serde_json::Value =
            mod_api::serde_json::from_str(definition).unwrap();
        file["neoforge:conditions"] =
            mod_api::serde_json::json!([{"type":"absent:may_be_overridden"}]);
        std::fs::write(folder.join("condition_startup.json"), file.to_string()).unwrap();
    }
    let recipe = root.join("data/minecraft/recipe/condition_startup.json");
    std::fs::write(&recipe, r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"testmod:flag","value":true}]}"#).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "testmod",
            Box::new(Registrant(count.clone())),
        )]
        .into(),
        || {
            let loaded = load_startup(&root)
                .expect("startup must accept a mod-registered condition in a recipe");
            assert!(
                loaded
                    .resources
                    .recipes()
                    .recipe("minecraft:condition_startup")
                    .is_some()
            );
            assert!(count.load(std::sync::atomic::Ordering::SeqCst) > 0);
        },
    );
}

/// `neoforge:tag_empty` in a startup recipe reads the tags the startup load bound, as
/// `ICondition.IContext.getTag` does: a recipe guarded by an empty tag loads, one guarded by
/// a tag with members is left out, and neither refuses the load.
#[test]
#[ignore = "daily"]
fn startup_recipe_reads_bound_tags_for_tag_empty() {
    let (_scratch, root) = scratch_resources();
    let folder = root.join("data/minecraft/recipe");
    for (name, tag) in [
        ("tag_empty_absent", "c:ingots/never"),
        ("tag_empty_logs", "minecraft:logs"),
    ] {
        std::fs::write(
            folder.join(format!("{name}.json")),
            format!(r#"{{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{{"id":"minecraft:stick"}},"neoforge:conditions":[{{"type":"neoforge:tag_empty","tag":"{tag}"}}]}}"#),
        )
        .unwrap();
    }
    let loaded = load_startup(&root).expect("startup must answer neoforge:tag_empty");
    let recipes = loaded.resources.recipes();
    assert!(recipes.recipe("minecraft:tag_empty_absent").is_some());
    assert!(recipes.recipe("minecraft:tag_empty_logs").is_none());
}
