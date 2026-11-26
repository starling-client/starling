//! Two separately built native mods prove `neoforge:conditions` across every data loader wired
//! in #5139: `chemlib` (optional library mod) registers a custom `chemlib:active` condition and
//! ships a recipe guarded by it; `boiler` (optional dependency on `chemlib`) ships conditional
//! data files across recipes, recipe filters, content-mod worldgen bundles, pack worldgen
//! features (including `neoforge:tag_empty` on static block tags), villager trades, trade sets,
//! predicates, item modifiers, and slot sources.
//!
//! Run 1 (both `chemlib` and `boiler` installed): all guarded entries load and the conditional
//! `recipe_filter` removes `boiler:filtered_out`, while `neoforge:never` siblings stay omitted.
//! Run 2 (`boiler` installed alone): `boiler` starts cleanly with zero errors, `boiler:filtered_out`
//! survives, every `chemlib`-guarded entry is absent, and the static-tag `tag_empty` pack feature
//! still loads.
//! Run 3: an unknown condition type on a worldgen file and on a dimension file fails the load,
//! naming each file and the type.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const CHEMLIB_SOURCE: &str = r#"
#[derive(Default)]
struct ChemLib;

struct Active;
impl mod_api::LoadCondition for Active {
    fn test(&self, _: &mod_api::serde_json::Value) -> Result<bool, mod_api::ConditionError> {
        Ok(true)
    }
}

#[mod_api::sunrise_mod]
impl mod_api::Hooks for ChemLib {
    fn register_load_conditions(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterLoadConditionsEvent<'_>,
    ) {
        event
            .conditions
            .register(
                mod_api::Identifier::parse("chemlib:active").unwrap(),
                Active,
            )
            .expect("register chemlib:active");
    }
}
"#;

const BOILER_SOURCE: &str = r#"
#[derive(Default)]
struct Boiler;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Boiler {
    fn server_started(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        let crafting = host.recipes_of_type("minecraft:crafting");
        let has_recipe = |id: &str| crafting.iter().any(|r| r.id == id);
        let has_entry = |registry: &str, id: &str| host.registry_entry(registry, id).is_some();

        let chemlib_custom = has_recipe("chemlib:active_brew");
        let boiler_recipe = has_recipe("boiler:chem_recipe");
        let filtered_out = has_recipe("boiler:filtered_out");
        let content_feature = has_entry("minecraft:worldgen/feature", "boiler:chem_feature");
        let pack_feature = has_entry("minecraft:worldgen/feature", "boiler:pack_chem_feature");
        let pack_tag_feature = has_entry("minecraft:worldgen/feature", "boiler:pack_tag_feature");
        let false_feature = has_entry("minecraft:worldgen/feature", "boiler:never_feature");
        let false_dimension = has_entry("minecraft:dimension", "boiler:never_dimension");
        let trade = has_entry("minecraft:villager_trade", "boiler:chem_trade");
        let trade_set = has_entry("minecraft:trade_set", "boiler:chem_set");
        let predicate = has_entry("minecraft:predicate", "boiler:chem_pred");
        let item_mod = has_entry("minecraft:item_modifier", "boiler:chem_mod");
        let slot_source = has_entry("minecraft:slot_source", "boiler:chem_slots");

        println!(
            "m2m_load_conditions chemlib_custom={chemlib_custom} boiler_recipe={boiler_recipe} filtered_out={filtered_out} content_feature={content_feature} pack_feature={pack_feature} pack_tag_feature={pack_tag_feature} false_feature={false_feature} false_dimension={false_dimension} trade={trade} trade_set={trade_set} predicate={predicate} item_mod={item_mod} slot_source={slot_source}"
        );
        std::process::exit(0);
    }
}
"#;

fn passed(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

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

fn write_world_datapack(game_dir: &Path) {
    let pack = game_dir.join("world/datapacks/m2m_cond_pack");
    write_file(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"m2m load conditions test pack","min_format":[121,0],"max_format":[121,0]}}"#,
    );
    // Pack worldgen feature guarded by neoforge:mod_loaded("chemlib").
    write_file(
        &pack.join("data/boiler/worldgen/feature/pack_chem_feature.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:no_op","config":{}}"#,
    );
    // Pack worldgen feature guarded by neoforge:tag_empty on static block tag minecraft:logs.
    write_file(
        &pack.join("data/boiler/worldgen/feature/pack_tag_feature.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:tag_empty","registry":"minecraft:block","tag":"minecraft:logs"}}],"type":"minecraft:no_op","config":{}}"#,
    );
    // Sibling guarded by neoforge:never must stay omitted in both runs.
    write_file(
        &pack.join("data/boiler/worldgen/feature/never_feature.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"type":"minecraft:no_op","config":{}}"#,
    );
    // A dimension file whose condition fails is left out of the level stem registry; its stem
    // names a generator that does not exist, so it would fail the load if it were read.
    write_file(
        &pack.join("data/boiler/dimension/never_dimension.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"type":"boiler:no_such_type","generator":{"type":"boiler:no_such_generator"}}"#,
    );
}

#[test]
#[ignore = "daily"]
fn two_separately_built_mods_gate_all_data_loaders_with_load_conditions() {
    let resources = prepared_resources();
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = scratch.path().canonicalize().unwrap();

    let stub_resources = root.join("stub-resources");
    fs::create_dir(&stub_resources).unwrap();
    for (name, json) in [
        (
            "sunrise-resources.json",
            r#"{"schema":7,"minecraft":"26.3"}"#,
        ),
        ("sunrise-prepare.json", r#"{"output_version":8}"#),
        ("registries.json", "{}"),
    ] {
        fs::write(stub_resources.join(name), json).unwrap();
    }

    let tool = |dir: &Path, args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
        cmd.current_dir(dir)
            .env("SUNRISE_MOD_CACHE", root.join("cache"))
            .env("SUNRISE_HOME", root.join("home"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .args(args);
        passed(cmd.output().unwrap())
    };

    // 1. chemlib: built alone; registers custom condition `chemlib:active` and a recipe using it.
    tool(&root, &["new", "chemlib", "--template", "native"]);
    let chemlib = root.join("chemlib");
    fs::write(chemlib.join("src/lib.rs"), CHEMLIB_SOURCE).unwrap();
    let chemlib_manifest = fs::read_to_string(chemlib.join("sunrise-mod.toml"))
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(chemlib.join("sunrise-mod.toml"), chemlib_manifest).unwrap();
    write_file(
        &chemlib.join("data/chemlib/recipe/active_brew.json"),
        r#"{"neoforge:conditions":[{"type":"chemlib:active"}],"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stick"],"result":{"id":"minecraft:torch","count":1}}"#,
    );

    let stage = |name: &str, project: &Path| -> PathBuf {
        let run = project.join("target/sunrise-mod/run");
        fs::create_dir_all(run.join("game")).unwrap();
        fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
        let stdout = tool(
            project,
            &[
                "run",
                "--resources",
                stub_resources.to_str().unwrap(),
                "--",
                "--dedicated",
                "--initSettings",
            ],
        );
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Starting "))
            .and_then(|line| line.split(" with ").next())
            .map(PathBuf::from)
            .expect("run names the program it starts");
        let kept = root.join("builds").join(name);
        copy_dir(&run.join("app"), &kept);
        fs::write(
            kept.join(".program"),
            executable.file_name().unwrap().as_encoded_bytes(),
        )
        .unwrap();
        kept
    };

    let chemlib_app = stage("chemlib", &chemlib);

    // 2. boiler: built alone with optional dependency on chemlib; ships conditional files across
    // all newly wired loaders.
    tool(&root, &["new", "boiler", "--template", "native"]);
    let boiler = root.join("boiler");
    fs::write(boiler.join("src/lib.rs"), BOILER_SOURCE).unwrap();
    let boiler_manifest = fs::read_to_string(boiler.join("sunrise-mod.toml")).unwrap();
    fs::write(
        boiler.join("sunrise-mod.toml"),
        format!(
            "{boiler_manifest}\n[dependencies]\nchemlib = {{ version = '^1', optional = true }}\n"
        ),
    )
    .unwrap();
    // Conditional recipe + unconditional recipe removed by a conditional recipe_filter.
    write_file(
        &boiler.join("data/boiler/recipe/chem_recipe.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:crafting_shapeless","ingredients":["minecraft:cobblestone"],"result":{"id":"minecraft:stone","count":1}}"#,
    );
    write_file(
        &boiler.join("data/boiler/recipe/filtered_out.json"),
        r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:dirt"],"result":{"id":"minecraft:coarse_dirt","count":1}}"#,
    );
    write_file(
        &boiler.join("data/boiler/recipe_filter/remove_when_chem.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"filters":[{"id":"boiler:filtered_out","remove":true}]}"#,
    );
    // Content-mod worldgen feature guarded by mod_loaded("chemlib").
    write_file(
        &boiler.join("data/boiler/worldgen/feature/chem_feature.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:no_op","config":{}}"#,
    );
    // Villager trade + trade set guarded by mod_loaded("chemlib").
    write_file(
        &boiler.join("data/boiler/villager_trade/chem_trade.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"wants":{"id":"minecraft:emerald","count":1},"gives":{"id":"minecraft:iron_ingot","count":1}}"#,
    );
    write_file(
        &boiler.join("data/boiler/trade_set/chem_set.json"),
        r##"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"trades":"#boiler:chem_trades"}"##,
    );
    // Predicate + item modifier + slot source guarded by mod_loaded("chemlib").
    write_file(
        &boiler.join("data/boiler/predicate/chem_pred.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:random_chance","chance":0.5}"#,
    );
    write_file(
        &boiler.join("data/boiler/item_modifier/chem_mod.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:set_count","count":2}"#,
    );
    write_file(
        &boiler.join("data/boiler/slot_source/chem_slots.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:slot_range","slots":"container.*"}"#,
    );
    let boiler_app = stage("boiler", &boiler);

    let read_record = |app: &Path| -> Value {
        serde_json::from_str(&fs::read_to_string(app.join("native-mods.json")).unwrap()).unwrap()
    };
    assert_eq!(
        read_record(&chemlib_app)["libraries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read_record(&boiler_app)["libraries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Combine both separately built mods into one installation for Run 1.
    let combined = root.join("combined/both");
    copy_dir(&boiler_app, &combined);
    let mut merged = read_record(&combined);
    let combined_rows = merged["libraries"].as_array_mut().unwrap();
    let other_record = read_record(&chemlib_app);
    for row in other_record["libraries"].as_array().unwrap() {
        let file = row["file"].as_str().unwrap();
        fs::copy(chemlib_app.join(file), combined.join(file)).unwrap();
        combined_rows.push(row.clone());
    }
    let extra_native = chemlib_app.join(sunrise_mod_package::mod_list::NATIVE_DIR);
    if extra_native.is_dir() {
        let target_native = combined.join(sunrise_mod_package::mod_list::NATIVE_DIR);
        fs::create_dir_all(&target_native).unwrap();
        for entry in fs::read_dir(&extra_native).unwrap() {
            let entry = entry.unwrap();
            let dest = target_native.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    fs::write(combined.join("native-mods.json"), merged.to_string()).unwrap();

    let run_dedicated = |app: &Path, name: &str| -> Output {
        let game_dir = root.join(name);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("eula.txt"), "eula=true\n").unwrap();
        write_world_datapack(&game_dir);
        let program = fs::read_to_string(app.join(".program")).unwrap();
        let home = tempfile::tempdir_in(&root).unwrap();
        let mut cmd = Command::new(app.join(program.trim()));
        cmd.arg("--game-dir")
            .arg(&game_dir)
            .arg("--resources")
            .arg(&resources)
            .arg("--dedicated")
            .args(["--nogui", "--port", "0"])
            .env("SUNRISE_HOME", home.path())
            .env("SUNRISE_NO_AUDIO", "1")
            .env_remove("SUNRISE_SESSION_FILE");
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", app);
        cmd.output().unwrap()
    };
    let start_dedicated = |app: &Path, name: &str| passed(run_dedicated(app, name));

    // Run 1: chemlib + boiler together.
    let stdout_both = start_dedicated(&combined, "game-both");
    let expected_both = "m2m_load_conditions chemlib_custom=true boiler_recipe=true filtered_out=false content_feature=true pack_feature=true pack_tag_feature=true false_feature=false false_dimension=false trade=true trade_set=true predicate=true item_mod=true slot_source=true";
    assert!(
        stdout_both.lines().any(|line| line == expected_both),
        "expected `{expected_both}` in Run 1 output:\n{stdout_both}"
    );

    // Run 2: boiler alone (optional chemlib absent).
    let stdout_alone = start_dedicated(&boiler_app, "game-alone");
    let expected_alone = "m2m_load_conditions chemlib_custom=false boiler_recipe=false filtered_out=true content_feature=false pack_feature=false pack_tag_feature=true false_feature=false false_dimension=false trade=false trade_set=false predicate=false item_mod=false slot_source=false";
    assert!(
        stdout_alone.lines().any(|line| line == expected_alone),
        "expected `{expected_alone}` in Run 2 output:\n{stdout_alone}"
    );

    // Run 3: an unknown condition type on a worldgen file and on a dimension file fails the
    // whole registry load (`ConditionalOps` decode error), naming each file and the type.
    let bad_game = root.join("game-unknown-condition");
    for file in [
        "data/boiler/worldgen/feature/unknown_condition.json",
        "data/boiler/dimension/unknown_condition.json",
    ] {
        write_file(
            &bad_game.join("world/datapacks/m2m_cond_pack").join(file),
            r#"{"neoforge:conditions":[{"type":"unknown:bogus"}],"type":"minecraft:no_op","config":{}}"#,
        );
    }
    let refused = run_dedicated(&boiler_app, "game-unknown-condition");
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(!refused.status.success(), "the load must fail:\n{log}");
    for file in [
        "data/boiler/worldgen/feature/unknown_condition.json",
        "data/boiler/dimension/unknown_condition.json",
    ] {
        assert!(
            log.contains(file) && log.contains("unknown:bogus"),
            "expected `{file}` and `unknown:bogus` in the refusal:\n{log}"
        );
    }
}
