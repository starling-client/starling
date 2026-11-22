use serde_json::{Map, Value, json};
use sunrise_world::{
    item_defaults, loot,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    native_load_hooks::{self, NativeLoadHooks},
};

#[test]
fn load_hooks_are_noops_before_install_and_use_first_callbacks_after() {
    let root = std::env::temp_dir().join(format!("sunrise-load-hooks-{}", std::process::id()));
    let context = LoadContext::new(
        DataLoadOrigin::ClientPreload,
        std::sync::Arc::new(LoadInputs::new(Default::default(), root.join("settings"))),
        [],
    );
    let server_context = context.for_load(DataLoadOrigin::ServerDataLoad, []);
    let mut table = json!({"markers":[]});
    let mut components = Map::new();
    loot::native_loot_table_load(&server_context, "table", &mut table);
    item_defaults::native_modify_default_components(&context, "item", &mut components);
    assert_eq!(table, json!({"markers":[]}));
    assert!(components.is_empty());

    let hooks = || NativeLoadHooks {
        item_ability_default: |_, _, _| false,
        loot_ability: |_, _, _| {},
        loot_table_load: |context, id, table| {
            assert_eq!(context.origin(), DataLoadOrigin::ServerDataLoad);
            if id == "minecraft:malformed" {
                *table = json!({"pools": [{"entries": [{
                    "type": "minecraft:tag",
                    "items": {"type": "neoforge:or", "values": 42}
                }]}]});
            } else if id == "minecraft:bad_codec" {
                table["modifier"] = json!({"type":"missing:codec"});
            } else {
                table["markers"].as_array_mut().unwrap().push(id.into());
            }
        },
        modify_default_components: |context, item, components| {
            assert_eq!(context.origin(), DataLoadOrigin::ClientPreload);
            components.insert("item".into(), Value::String(item.into()));
        },
    };
    assert!(native_load_hooks::install(hooks()).is_ok());
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            item_ability_default: |_, _, _| false,
            loot_ability: |_, _, _| {},
            loot_table_load: |_, _, table| *table = json!("second"),
            modify_default_components: |_, _, components| components.clear(),
        })
        .is_err()
    );
    loot::native_loot_table_load(&server_context, "table", &mut table);
    item_defaults::native_modify_default_components(&context, "item", &mut components);
    assert_eq!(table, json!({"markers":["table"]}));
    assert_eq!(components["item"], json!("item"));

    // Client/shared loads keep raw tables; each server load gets exactly one
    // callback per table, even when the raw map is already cached.
    let dir = root.join("data/minecraft/loot_table");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("items.json"), r#"{"probe:token":{}}"#).unwrap();
    std::fs::write(dir.join("test.json"), r#"{"markers":[]}"#).unwrap();
    let item_data = item_defaults::ItemData::load(&root, context).unwrap();
    // The server loot load owns its context even when the item data came from a client preload.
    assert_eq!(
        item_data.defaults().entries()["probe:token"]["item"],
        "probe:token"
    );
    let raw = loot::shared_tables(&root, &loot::PackOverlay::NONE).unwrap();
    assert_eq!(raw["minecraft:test"], json!({"markers":[]}));
    for _ in 0..2 {
        let mut server =
            loot::LootData::load(&root, &loot::PackOverlay::NONE, item_data.defaults()).unwrap();
        server.apply_native_table_loads(&server_context).unwrap();
        assert_eq!(
            server.tables()["minecraft:test"],
            json!({"markers":["minecraft:test"]})
        );
        assert_eq!(raw["minecraft:test"], json!({"markers":[]}));
    }
    // A malformed native replacement refuses the whole candidate, not just
    // the bad field, and leaves the previously usable table snapshot intact.
    std::fs::write(dir.join("malformed.json"), r#"{"markers":["original"]}"#).unwrap();
    let mut server =
        loot::LootData::load(&root, &loot::PackOverlay::NONE, item_data.defaults()).unwrap();
    let previous = server.tables().clone();
    let error = server
        .apply_native_table_loads(&server_context)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("native loot_table minecraft:malformed")
            && error.contains("pools[0].entries[0].items")
            && error.contains("no values"),
        "{error}"
    );
    assert!(std::sync::Arc::ptr_eq(server.tables(), &previous));
    assert_eq!(
        server.tables()["minecraft:malformed"],
        json!({"markers":["original"]})
    );
    assert_eq!(server.tables()["minecraft:test"], json!({"markers":[]}));
    assert_eq!(raw["minecraft:test"], json!({"markers":[]}));
    std::fs::remove_file(dir.join("malformed.json")).unwrap();
    std::fs::write(dir.join("bad_codec.json"), r#"{"markers":[]}"#).unwrap();
    let mut server =
        loot::LootData::load(&root, &loot::PackOverlay::NONE, item_data.defaults()).unwrap();
    let previous = server.tables().clone();
    let error = server
        .apply_native_table_loads(&server_context)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("loot_table minecraft:bad_codec")
            && error.contains("[data/minecraft/loot_table/bad_codec.json]")
            && error.contains("$.modifier")
            && error.contains("missing:codec"),
        "{error}"
    );
    assert!(std::sync::Arc::ptr_eq(server.tables(), &previous));
    assert_eq!(
        server.tables()["minecraft:bad_codec"],
        json!({"markers":[]})
    );
    std::fs::remove_dir_all(&root).unwrap();
}
