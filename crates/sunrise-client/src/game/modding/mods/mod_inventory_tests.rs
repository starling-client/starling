use super::*;
use mod_api::{Hooks, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise_mod_package::{enabled::EnabledMods, selection::SelectedPackage};

struct EmptyCode;
#[mod_api::__subscriptions]
impl Hooks for EmptyCode {}

fn selected(fixture: &Fixture, id: &str, side: &str) -> SelectedPackage {
    let manifest = format!(
        "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\nside='{side}'\nnamespaces=['shared_space']\n"
    );
    let path = fixture.0.join(format!("{id}.smod"));
    std::fs::write(
        &path,
        sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
    )
    .unwrap();
    SelectedPackage {
        package: sunrise_mod_package::Package::read(&path).unwrap(),
        path,
    }
}

#[test]
fn admission_uses_active_native_entries_and_accepted_content_not_startup_metadata() {
    let fixture = Fixture::new("inventory-admission");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        selected: vec![
            selected(&fixture, "plain", "both"),
            selected(&fixture, "refused_content", "both"),
        ]
        .into(),
        ..Default::default()
    };
    with_mods(
        vec![ModEntry::new("machine_sample", Box::new(EmptyCode))].into(),
        || {
            mod_platform::__private::with_test_packages(&["refused_code", "shared_space"], || {
                assert!(
                    mod_api::__private::loaded_mods()
                        .get("refused_code")
                        .is_some()
                );
                let mut startup = sources.clone();
                let disabled = std::collections::BTreeSet::from(["machine_sample".into()]);
                startup.capture_code_switches(&disabled);
                assert!(mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("machine_sample").unwrap()
                ));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["plain"]
                );
                startup.apply_code_switches(&disabled);
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                let initial = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(
                    initial.client_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                let paused = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(paused.server_ids().collect::<Vec<_>>(), ["plain"]);
                assert_eq!(paused.client_ids().collect::<Vec<_>>(), ["plain"]);
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain"]
                );
            });
        },
    );
}

struct Guard(Arc<AtomicBool>);
impl mod_api::ServerReloadListener for Guard {
    fn prepare(
        &self,
        _: &mod_api::ServerReloadContext,
    ) -> std::result::Result<Box<dyn FnOnce() + Send + 'static>, mod_api::ReloadResourceError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(mod_api::ReloadResourceError::new(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                std::io::Error::other("inventory fixture rejected the new generation"),
            ));
        }
        Ok(Box::new(|| {}))
    }
}
#[mod_api::__subscriptions]
impl Hooks for Guard {
    fn add_server_reload_listeners(
        &self,
        _: &dyn mod_api::ReloadContext,
        event: &mut mod_api::AddServerReloadListenersEvent<'_>,
    ) {
        event
            .listeners
            .add_listener(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                mod_api::FileToIdConverter::json("inventory_test").unwrap(),
                Arc::new(Guard(self.0.clone())),
            )
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn code_only_reload_publishes_one_new_inventory_after_prepare_and_preserves_old_readers() {
    let fixture = Fixture::new("inventory-reload");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        ..Default::default()
    };
    let reject = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![
            ModEntry::new("machine_sample", Box::new(EmptyCode)),
            ModEntry::new("reload_guard", Box::new(Guard(reject.clone()))),
        ]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let original = game.options.resources.clone();
                let initial = content_overlay(&original, &sources, &fixture.0.join("jars"))
                    .unwrap()
                    .root;
                game.options.resources = initial.prepared().to_owned();
                game.pack.data_source = initial.data().clone();
                game.pack.data_changes = initial.data_changes().clone();
                game.mods = sources.clone();
                game.world_root = fixture.0.join("world");
                let pack = game.world_root.join("datapacks/inventory");
                let recipes = pack.join("data/inventorytest/recipe");
                std::fs::create_dir_all(&recipes).unwrap();
                std::fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({
                "pack": {"description":"inventory fixture", "min_format":sunrise_core::version::DATA_PACK_FORMAT, "max_format":sunrise_core::version::DATA_PACK_FORMAT}
            })).unwrap()).unwrap();
                std::fs::write(recipes.join("requires_code.json"), br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}]}"#).unwrap();
                let tags = pack.join("data/inventorytest/tags/item");
                std::fs::create_dir_all(&tags).unwrap();
                std::fs::write(tags.join("requires_code.json"), br#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}],"values":["minecraft:diamond"]}"#).unwrap();
                for (kind, mut value) in [
                    ("loot_table", serde_json::json!({"pools":[]})),
                    (
                        "predicate",
                        serde_json::json!({"type":"minecraft:random_chance","chance":1}),
                    ),
                    (
                        "item_modifier",
                        serde_json::json!({"type":"minecraft:set_count","count":1}),
                    ),
                    (
                        "jukebox_song",
                        serde_json::json!({"sound_event":"minecraft:music_disc.cat","length_in_seconds":1,"comparator_output":1,"description":"condition test"}),
                    ),
                    (
                        "instrument",
                        serde_json::json!({"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":7,"range":256,"description":"condition test"}),
                    ),
                    (
                        "enchantment",
                        serde_json::json!({
                            "description":"condition test", "supported_items":"minecraft:stick", "weight":1, "max_level":3,
                            "min_cost":{"base":1,"per_level_above_first":0}, "max_cost":{"base":1,"per_level_above_first":0},
                            "anvil_cost":1, "slots":["mainhand"]
                        }),
                    ),
                ] {
                    value["neoforge:conditions"] = serde_json::json!([{"type":"neoforge:mod_loaded","modid":"machine_sample"}]);
                    let dir = pack.join("data/inventorytest").join(kind);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("requires_code.json"), value.to_string()).unwrap();
                    std::fs::write(
                        dir.join("unsupported.json"),
                        br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#,
                    )
                    .unwrap();
                }
                // This fixture skips world open, which normally loads the built-in sound holders.
                let registries = crate::game::chat::command_export(&original, false)
                    .expect("prepared command registries");
                game.ui.registries.fill_from_exported(&registries);
                game.reload_datapacks(true, &[]).unwrap();
                let loaded = |game: &Game<'_>, id: &str| {
                    let items = game.items();
                    (
                        items.has_loot_table(id),
                        items.songs.contains_key(id),
                        items.instruments.contains_key(id),
                        items.enchantments.list.iter().any(|e| &*e.id == id),
                        game.fns.store.predicates.contains_key(id),
                        game.fns.store.item_modifiers.contains_key(id),
                    )
                };
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    loaded(game, "inventorytest:unsupported"),
                    (false, false, false, false, false, false)
                );
                let old_songs = game.items().songs.clone();
                let read_tags = |game: &Game<'_>| game.items().item_tags.clone();
                let read = |game: &Game<'_>| {
                    crate::game::lan::sync::server_sources(
                        &game.options.resources,
                        &game.res.core.item_data,
                        game.items(),
                        &game.pack_files(),
                    )
                    .unwrap()
                };
                let has_recipe = |sources: &sunrise_client::crafting::ResolvedSources| {
                    sources
                        .recipes
                        .iter()
                        .any(|recipe| recipe.name == "inventorytest:requires_code")
                };
                let old = read(game);
                assert!(has_recipe(&old));
                let old_tags = read_tags(game);
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                let old_root = game.options.resources.clone();
                let old_data = game.pack.data_source.clone();
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                reject.store(true, Ordering::Relaxed);
                assert!(game.reload_datapacks(true, &[]).is_err());
                assert_eq!(game.options.resources, old_root);
                assert!(game.pack.data_source.same_generation(&old_data));
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    EnabledMods::read(&old_root)
                        .unwrap()
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample"]
                );
                reject.store(false, Ordering::Relaxed);
                game.reload_datapacks(true, &[]).unwrap();
                assert_eq!(game.options.resources, original);
                assert!(!has_recipe(&read(game)));
                // Instruments and enchantments are world registries, frozen at world open: a
                // reload re-evaluates the reloadable data (loot, songs, predicates, modifiers).
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (false, false, true, true, false, false)
                );
                assert!(old_songs.contains_key("inventorytest:requires_code"));
                // Tag files ignore load conditions, including mod_loaded.
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                assert!(has_recipe(&old));
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                game.reload_datapacks(true, &[]).unwrap();
                assert!(has_recipe(&read(game)));
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(game.options.resources, old_root);
            });
        },
    );
}

/// Capture metadata uses the same typed runtime provenance as loading.
#[test]
fn capture_metadata_distinguishes_content_native_and_unknown_code() {
    use sunrise_perf_format::v1::ModKind;
    let fixture = Fixture::new("perf-inventory");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        ..Default::default()
    };
    let inventory: Vec<_> = [
        ("plain", mod_api::PackageKind::Content),
        ("machine_sample", mod_api::PackageKind::Code),
        ("builtin_code", mod_api::PackageKind::Code),
    ]
    .into_iter()
    .map(|(id, kind)| {
        mod_api::PackageInfo::new(
            id.to_owned(),
            "2.3".to_owned(),
            kind,
            mod_api::Side::Both,
            vec![id.to_owned()],
            vec![],
        )
    })
    .collect();
    let metadata = sources.perf_metadata(inventory.iter());
    assert_eq!(
        metadata
            .iter()
            .map(|m| (m.id.as_str(), m.version.as_str(), m.kind))
            .collect::<Vec<_>>(),
        vec![
            ("plain", "2.3", ModKind::Content as i32),
            ("machine_sample", "2.3", ModKind::Native as i32),
            ("builtin_code", "2.3", ModKind::Unspecified as i32),
        ]
    );
    assert!(metadata.iter().all(|m| m.crate_name.is_empty()));
}
