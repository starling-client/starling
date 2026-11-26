//! Two separately built native mods interoperate through Sunrise's built-in `c:` convention tags
//! without declaring any dependency on each other: `mod_a` registers `mod_a:bronze_ingot` and adds
//! it to `c:ingots/bronze` (plus `#c:ingots/bronze` in `c:ingots`); `mod_b` reads `c::items::INGOTS`
//! and `c:ingots/bronze` in `tags_updated`, ships a recipe consuming `#c:ingots/bronze`, and prints
//! one line naming what it saw when the game loads both mods together.
#[path = "support/own_sdk.rs"]
mod own_sdk;

use own_sdk::{OwnSdkScratch, passed, record};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MOD_A_SOURCE: &str = r#"
#[derive(Default)]
struct ModA;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for ModA {
    fn items(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
        event
            .register(mod_api::Item::new(
                "mod_a:bronze_ingot",
                "minecraft:iron_ingot",
            ))
            .expect("register mod_a:bronze_ingot");
    }
}
"#;

const MOD_B_SOURCE: &str = r#"
#[derive(Default)]
struct ModB;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for ModB {
    fn tags_updated(
        &self,
        context: &dyn mod_api::ReloadContext,
        _: &mut mod_api::TagsUpdatedEvent,
    ) {
        let tags = context.tags();
        let iron = context
            .item_key(&mod_api::Identifier::parse("minecraft:iron_ingot").unwrap())
            .expect("minecraft:iron_ingot");
        let bronze = context
            .item_key(&mod_api::Identifier::parse("mod_a:bronze_ingot").unwrap())
            .expect("mod_a:bronze_ingot");
        let ingots_has_iron = match tags.items(mod_api::tags::c::items::INGOTS) {
            mod_api::TagMembers::Members(members) => {
                members.contains(&iron)
                    && tags.is_in_tag(iron, mod_api::tags::c::items::INGOTS) == Some(true)
            }
            other => panic!("c:ingots did not bind: {other:?}"),
        };
        let ingots_has_bronze = match tags.items(mod_api::tags::c::items::INGOTS) {
            mod_api::TagMembers::Members(members) => {
                members.contains(&bronze)
                    && tags.is_in_tag(bronze, mod_api::tags::c::items::INGOTS) == Some(true)
            }
            other => panic!("c:ingots did not bind: {other:?}"),
        };
        let bronze_tag = tags
            .key(&mod_api::Identifier::parse("c:ingots/bronze").unwrap())
            .expect("c:ingots/bronze key")
            .typed::<mod_api::ItemKey>();
        let mut bronze_members: Vec<String> = match tags.items(bronze_tag) {
            mod_api::TagMembers::Members(members) => members
                .into_iter()
                .map(|key| {
                    context
                        .item_key_id(key)
                        .expect("item key id")
                        .to_owned()
                })
                .collect(),
            other => panic!("c:ingots/bronze did not bind: {other:?}"),
        };
        bronze_members.sort();
        let recipe_matches_bronze = context
            .recipes_of_type("minecraft:crafting")
            .iter()
            .find(|recipe| recipe.id == "mod_b:bronze_nugget")
            .is_some_and(|recipe| {
                recipe
                    .ingredients
                    .first()
                    .is_some_and(|ingredient| ingredient.items.contains("mod_a:bronze_ingot"))
            });
        println!(
            "mod_b_c_tags ingots_iron={ingots_has_iron} ingots_bronze={ingots_has_bronze} bronze={} recipe_bronze={recipe_matches_bronze}",
            bronze_members.join(",")
        );
        std::process::exit(0);
    }
}
"#;

fn prepared_resources() -> PathBuf {
    for var in ["SUNRISE_TEST_SERVER_PROFILE", "SUNRISE_TEST_RESOURCES"] {
        if let Some(path) = std::env::var_os(var) {
            let path = PathBuf::from(path);
            assert!(
                path.join("sunrise-resources.json").is_file(),
                "{var} ({}) is missing sunrise-resources.json",
                path.display()
            );
            return path;
        }
    }
    let fallback = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3");
    assert!(
        fallback.join("sunrise-resources.json").is_file(),
        "set SUNRISE_TEST_SERVER_PROFILE or SUNRISE_TEST_RESOURCES to prepared 26.3 resources"
    );
    fallback.canonicalize().unwrap()
}

#[test]
#[ignore = "daily"]
fn two_separately_built_native_mods_interoperate_through_c_convention_tags() {
    let resources = prepared_resources();
    let scratch = OwnSdkScratch::new();

    // mod_a: built alone; registers mod_a:bronze_ingot and populates c:ingots/bronze + c:ingots.
    let mod_a = scratch.new_native_mod("mod_a", MOD_A_SOURCE);
    let mod_a_tags = mod_a.join("data/c/tags/item");
    fs::create_dir_all(mod_a_tags.join("ingots")).unwrap();
    fs::write(
        mod_a_tags.join("ingots/bronze.json"),
        r#"{"values":["mod_a:bronze_ingot"]}"#,
    )
    .unwrap();
    fs::write(
        mod_a_tags.join("ingots.json"),
        r##"{"values":["#c:ingots/bronze"]}"##,
    )
    .unwrap();
    scratch.tool(&mod_a, &["build"]);
    let (mod_a_app, _) = scratch.stage_dedicated_app("mod_a", &mod_a, &[], &["--initSettings"]);

    // mod_b: built alone with no dependency on mod_a; queries c:ingots + c:ingots/bronze and
    // ships a recipe consuming #c:ingots/bronze.
    let mod_b = scratch.new_native_mod("mod_b", MOD_B_SOURCE);
    let mod_b_manifest = fs::read_to_string(mod_b.join("sunrise-mod.toml")).unwrap();
    assert!(
        !mod_b_manifest.contains("mod_a"),
        "mod_b must not depend on mod_a:\n{mod_b_manifest}"
    );
    let mod_b_recipes = mod_b.join("data/mod_b/recipe");
    fs::create_dir_all(&mod_b_recipes).unwrap();
    fs::write(
        mod_b_recipes.join("bronze_nugget.json"),
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["#c:ingots/bronze"],"result":{"id":"minecraft:gold_nugget","count":1}}"##,
    )
    .unwrap();
    scratch.tool(&mod_b, &["build"]);
    let (mod_b_app, _) = scratch.stage_dedicated_app("mod_b", &mod_b, &[], &["--initSettings"]);

    // Confirm each separate build produced only its own library row.
    assert_eq!(record(&mod_a_app)["libraries"].as_array().unwrap().len(), 1);
    assert_eq!(record(&mod_b_app)["libraries"].as_array().unwrap().len(), 1);

    // Install both separately built mods together and start the dedicated server.
    let combined = scratch.combine_installations("both", &mod_b_app, &[&mod_a_app]);
    assert_eq!(record(&combined)["libraries"].as_array().unwrap().len(), 2);

    let game_dir = scratch.root().join("game");
    fs::create_dir_all(&game_dir).unwrap();
    fs::write(game_dir.join("eula.txt"), "eula=true\n").unwrap();
    let stdout = passed(scratch.start_dedicated(
        &combined,
        &game_dir,
        &resources,
        &["--nogui", "--port", "0"],
    ));
    let expected = "mod_b_c_tags ingots_iron=true ingots_bronze=true bronze=mod_a:bronze_ingot recipe_bronze=true";
    assert!(
        stdout.lines().any(|line| line == expected),
        "expected `{expected}` in game output:\n{stdout}"
    );
}
