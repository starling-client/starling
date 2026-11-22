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

#[test]
#[ignore = "daily"]
fn startup_recipe_uses_registered_condition() {
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
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("resources");
    copy_tree(
        &std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap()),
        &root,
    );
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
            let loaded = crate::game::load_resources_for_mods(
                sunrise_assets::resource_source::ResourceRoot::directory(root.clone()).into(),
                Vec::new(),
                None,
                &root,
                &crate::game::Settings::default(),
                std::time::Instant::now(),
                false,
                crate::game::execution_role::ExecutionRole::IntegratedServer,
                &crate::game::Progress::new(),
                &mut |_, _| unreachable!(),
                &mut || unreachable!("the server has no client pack selection"),
                &crate::game::mods::ModsState::default(),
            )
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
