//! Isolated real data loaders: a refused loot table, predicate, modifier or advancement fails its load, every one listed; a refused recipe fails its load too.
extern crate sunrise_client_lib as sunrise_client;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    path::Path,
    sync::{Arc, Once},
};
use sunrise_client::{advancements::Tree, crafting::Recipes};
use sunrise_items::{
    item_data, item_nbt,
    items::{ItemStack, Items},
};
use sunrise_world::{
    loot::{LootData, PackOverlay},
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
};

thread_local! { static MESSAGES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
struct Capture;
impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        MESSAGES.with_borrow_mut(|messages| messages.push(record.args().to_string()));
    }
    fn flush(&self) {}
}
fn capture<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
    MESSAGES.with_borrow_mut(Vec::clear);
    let result = run();
    (result, MESSAGES.with_borrow_mut(std::mem::take))
}
fn write(root: &Path, kind: &str, name: &str, value: &Value) {
    let dir = root.join(format!("data/minecraft/{kind}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.json")), value.to_string()).unwrap();
}
fn context() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}
fn rejects_recipe(input: Value, component: &str) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "recipe", "a_bad", &input);
    write(
        root.path(),
        "recipe",
        "z_good",
        &json!({"type":"minecraft:crafting_shapeless",
        "ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"}}),
    );
    let items = Items::empty();
    let (result, _) =
        capture(|| Recipes::load(root.path(), &items, &PackOverlay::NONE, &context()));
    let error = result
        .err()
        .expect("a refused recipe fails the load")
        .to_string();
    assert!(
        error.contains("data/minecraft/recipe/a_bad.json"),
        "{error}"
    );
    assert!(error.contains(component), "{error}");
    assert!(error.contains("not a persistent component"), "{error}");
    assert!(!error.contains("z_good"), "{error}");
}
#[test]
fn creative_slot_lock_result_refuses_the_file() {
    rejects_recipe(
        json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],
        "result":{"id":"minecraft:stick","components":{"minecraft:creative_slot_lock":{}}}}),
        "minecraft:creative_slot_lock",
    );
}
#[test]
fn additional_trade_cost_result_has_no_persistent_payload() {
    rejects_recipe(
        json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],
        "result":{"id":"minecraft:stick","components":{"additional_trade_cost":"not a payload format"}}}),
        "minecraft:additional_trade_cost",
    );
}
#[test]
fn map_post_processing_removal_refuses_the_file() {
    rejects_recipe(
        json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],
        "result":{"id":"minecraft:stick","components":{"!minecraft:map_post_processing":{}}}}),
        "minecraft:map_post_processing",
    );
}
#[test]
fn component_ingredient_removal_refuses_the_load_before_station_publication() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "recipe",
        "a_bad",
        &json!({"type":"minecraft:stonecutting",
        "ingredient":{"type":"neoforge:components","items":"minecraft:stone",
            "components":{"!creative_slot_lock":{}}},"result":{"id":"minecraft:stick"}}),
    );
    write(
        root.path(),
        "recipe",
        "z_good",
        &json!({"type":"minecraft:stonecutting",
        "ingredient":"minecraft:stone","result":{"id":"minecraft:stick"}}),
    );
    let items = Items::empty();
    let error = sunrise_client::crafting::ResolvedSources::load(
        root.path(),
        &items,
        &PackOverlay::NONE,
        &context(),
    )
    .err()
    .expect("a refused recipe fails the load")
    .to_string();
    assert!(
        error.contains("data/minecraft/recipe/a_bad.json"),
        "{error}"
    );
    assert!(error.contains("minecraft:creative_slot_lock"), "{error}");
}
#[test]
fn type_only_displays_and_opaque_data_survive_recipe_loading() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "recipe",
        "display",
        &json!({"type":"minecraft:crafting_shapeless",
        "ingredients":[{"type":"neoforge:custom_display","base":"minecraft:stone",
            "display":{"type":"minecraft:only_with_component","component":"minecraft:creative_slot_lock",
                "contents":{"type":"minecraft:item","item":"minecraft:stone"}}}],
        "result":{"id":"minecraft:stick","components":{
            "minecraft:tooltip_display":{"hidden_components":["minecraft:map_post_processing"]},
            "minecraft:custom_data":{"components":{"minecraft:additional_trade_cost":99}},
            "example:config":{"components":{"minecraft:creative_slot_lock":{}}}}}}),
    );
    let items = Items::empty();
    let (recipes, messages) =
        capture(|| Recipes::load(root.path(), &items, &PackOverlay::NONE, &context()).unwrap());
    assert_eq!(recipes.crafting.len(), 1);
    assert!(
        !messages
            .iter()
            .any(|message| message.contains("not a persistent component")),
        "{messages:?}"
    );
}
#[test]
fn loot_value_removal_and_exact_predicate_refuse_the_load_and_list_each() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "loot_table",
        "a_value",
        &json!({"modifier":{"type":"minecraft:set_components",
        "components":{"minecraft:additional_trade_cost":7}},"pools":[]}),
    );
    write(
        root.path(),
        "loot_table",
        "b_remove",
        &json!({"modifier":{"type":"minecraft:set_components",
        "components":{"!creative_slot_lock":{}}},"pools":[]}),
    );
    write(
        root.path(),
        "loot_table",
        "c_exact",
        &json!({"pools":[{"rolls":1,
        "condition":{"type":"minecraft:match_tool","predicate":{"components":{"map_post_processing":"scale"}}},"entries":[]}]}),
    );
    write(
        root.path(),
        "loot_table",
        "z_good",
        &json!({"modifier":[
        {"type":"minecraft:copy_components","source":"block_entity","include":["minecraft:creative_slot_lock"]},
        {"type":"minecraft:toggle_tooltips","toggles":{"minecraft:additional_trade_cost":false}},
        {"type":"minecraft:set_components","components":{"minecraft:tooltip_display":{
            "hidden_components":["minecraft:map_post_processing"]}}},
        {"type":"minecraft:set_custom_data","tag":{"components":{"minecraft:creative_slot_lock":{}}}}],"pools":[]}),
    );
    std::fs::write(root.path().join("items.json"), "{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    // RegistryDataLoader.logErrors: the load fails and lists every refused table.
    let Err(error) = LootData::load(root.path(), &PackOverlay::NONE, &defaults) else {
        panic!("a refused loot table loaded")
    };
    let error = error.to_string();
    for (table, component) in [
        ("a_value", "minecraft:additional_trade_cost"),
        ("b_remove", "minecraft:creative_slot_lock"),
        ("c_exact", "minecraft:map_post_processing"),
    ] {
        let line = error
            .lines()
            .find(|line| line.contains(&format!("loot table minecraft:{table}:")))
            .unwrap_or_else(|| panic!("{table} not listed: {error}"));
        assert!(line.contains(component), "{line}");
    }
    assert!(!error.contains("z_good"), "{error}");
    for table in ["a_value", "b_remove", "c_exact"] {
        std::fs::remove_file(
            root.path()
                .join(format!("data/minecraft/loot_table/{table}.json")),
        )
        .unwrap();
    }
    let loot = LootData::load(root.path(), &PackOverlay::NONE, &defaults).unwrap();
    assert_eq!(loot.tables().len(), 1);
    assert!(loot.has_table("minecraft:z_good"));
}
#[test]
fn advancement_exact_values_fail_but_any_value_component_types_load() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "advancement",
        "a_bad",
        &json!({"criteria":{"eat":{"trigger":"minecraft:consume_item",
        "conditions":{"item":{"components":{"minecraft:map_post_processing":"scale"}}}}}}),
    );
    write(
        root.path(),
        "advancement",
        "b_icon",
        &json!({"display":{"background":"minecraft:gui/advancements/backgrounds/stone",
        "icon":{"id":"minecraft:stone",
        "components":{"!minecraft:creative_slot_lock":{}}}},
        "criteria":{"never":{"trigger":"minecraft:impossible"}}}),
    );
    write(
        root.path(),
        "advancement",
        "z_good",
        &json!({"criteria":{"eat":{"trigger":"minecraft:consume_item",
        "conditions":{"item":{"predicates":{"minecraft:additional_trade_cost":{}}}}}}}),
    );
    write(
        root.path(),
        "advancement",
        "zz_native",
        &json!({"criteria":{"custom":{"trigger":"example:custom",
        "conditions":{"item":{"components":{"minecraft:creative_slot_lock":{}}}}}}}),
    );
    let tree = Tree::load(&root.path().join("data"));
    assert_eq!(tree.list.len(), 2);
    assert!(tree.by_id.contains_key("minecraft:z_good"));
    assert!(tree.by_id.contains_key("minecraft:zz_native"));
    // The load refuses on any of them (`Advancement.CODEC`), each listed.
    assert_eq!(tree.refused.len(), 2, "{:?}", tree.refused);
    assert!(tree.refused[0].contains("minecraft:a_bad"));
    assert!(tree.refused[0].contains("minecraft:map_post_processing"));
    assert!(tree.refused[1].contains("minecraft:b_icon"));
}
#[test]
fn saves_drop_transient_values_and_removals_without_mutating_live_or_wire_data() {
    use sunrise_core::nbt::Tag;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        item::{Component, Slot, Stack, component},
        registry::{Builtin, Dynamic, Ids},
    };
    let mut stack = ItemStack::new("minecraft:stone", 1);
    stack.data_mut().set_other(
        "minecraft:creative_slot_lock",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    stack
        .data_mut()
        .set_other("minecraft:additional_trade_cost", Some(Tag::Int(7)));
    stack.data_mut().map_post_processing = Some(1);
    stack.data_mut().set_other(
        "!minecraft:map_post_processing",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(
        item_nbt::item_to_nbt(&stack, &Items::empty())
            .get("components")
            .is_none()
    );
    assert!(stack.carries_component(None, "minecraft:creative_slot_lock"));
    assert_eq!(
        stack
            .data()
            .unwrap()
            .other("minecraft:additional_trade_cost"),
        Some(&Tag::Int(7))
    );
    assert_eq!(stack.data().unwrap().map_post_processing, Some(1));
    let items = Items::empty();
    assert!(
        item_nbt::item_to_nbt(&stack, &items)
            .get("components")
            .is_none()
    );
    assert_eq!(item_data::encode_extra_components(&stack), None);
    let mut sent = Stack::new(1, 1);
    sent.added
        .push((component::CREATIVE_SLOT_LOCK, Component::Unit));
    sent.added.push((42, Component::Int(7))); // Pinned ADDITIONAL_TRADE_COST registry id.
    sent.added
        .push((component::MAP_POST_PROCESSING, Component::Int(1)));
    let mut writer = Writer::new();
    Slot::write(&Some(sent), &mut writer);
    let mut reader = Reader::new(&writer.data);
    let received = Slot::read(&mut reader).unwrap().unwrap();
    assert_eq!(reader.remaining(), 0);
    assert!(received.component(component::CREATIVE_SLOT_LOCK).is_some());
    assert!(received.component(42).is_some());
    assert!(received.component(component::MAP_POST_PROCESSING).is_some());
    let mut names = vec![String::new(); 51];
    names[20] = "minecraft:creative_slot_lock".into();
    names[42] = "minecraft:additional_trade_cost".into();
    names[50] = "minecraft:map_post_processing".into();
    let builtin = Builtin {
        registries: [
            (
                "minecraft:item".into(),
                Ids::from_names(vec!["minecraft:air".into(), "minecraft:stone".into()]),
            ),
            (
                "minecraft:data_component_type".into(),
                Ids::from_names(names),
            ),
        ]
        .into(),
    };
    let decoded = sunrise_client::item_sync::from_network(&received, &builtin, &Dynamic::default());
    let before = decoded.wire_patch().unwrap();
    assert_eq!(decoded.data().unwrap().map_post_processing, Some(1));
    assert!(
        item_nbt::item_to_nbt(&decoded, &Items::empty())
            .get("components")
            .is_none()
    );
    assert_eq!(decoded.wire_patch().unwrap(), before);
    let mut echoed = Writer::new();
    Slot::write(&Some(received), &mut echoed);
    assert_eq!(echoed.data, writer.data);
    let network =
        sunrise_client::lan::guest::stack_to_network(|_| 1, |_, _| None, &Some(stack)).unwrap();
    assert!(matches!(
        network.component(component::MAP_POST_PROCESSING),
        Some(Component::Int(1))
    ));
}

#[test]
fn persistent_nbt_and_component_syntax_refuse_transients_without_rejecting_runtime_data() {
    use sunrise_core::nbt::Tag;
    let patch = Tag::compound([(
        "minecraft:creative_slot_lock",
        Tag::compound([] as [(&str, Tag); 0]),
    )]);
    let saved = Tag::compound([
        ("id", Tag::String("minecraft:stone".into())),
        ("components", patch.clone()),
    ]);
    assert!(item_nbt::item_from_persistent_nbt(&saved).is_err());
    let (_, runtime) = item_data::decode_components(Some(&patch));
    assert!(
        runtime
            .unwrap()
            .other("minecraft:creative_slot_lock")
            .is_some()
    );
    let set = item_data::command_components("[additional_trade_cost=7]").unwrap_err();
    assert!(
        set.contains("minecraft:additional_trade_cost")
            && set.contains("not a persistent component")
    );
    let remove = item_data::command_components("[!map_post_processing]").unwrap_err();
    assert!(
        remove.contains("minecraft:map_post_processing")
            && remove.contains("not a persistent component")
    );
}

#[test]
fn named_modifier_and_predicate_files_refuse_the_load_before_loot_execution() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), "{}").unwrap();
    write(
        root.path(),
        "item_modifier",
        "bad_modifier",
        &json!([
        {"type":"minecraft:set_count","count":2},
        {"type":"minecraft:set_components","components":{"minecraft:creative_slot_lock":{}}}]),
    );
    write(
        root.path(),
        "item_modifier",
        "good_modifier",
        &json!({"type":"minecraft:set_count","count":3}),
    );
    write(
        root.path(),
        "predicate",
        "bad_predicate",
        &json!({"type":"minecraft:inverted",
        "term":{"type":"minecraft:match_tool","predicate":{"components":{"minecraft:additional_trade_cost":7}}}}),
    );
    write(
        root.path(),
        "predicate",
        "good_predicate",
        &json!({"type":"minecraft:all_of","terms":[]}),
    );
    write(
        root.path(),
        "item_modifier",
        "bad_number",
        &json!({"type":"minecraft:set_count","count":{
        "type":"minecraft:conditional","on_true":4,"on_false":5,"condition":{"type":"minecraft:match_tool",
        "predicate":{"components":{"minecraft:map_post_processing":"scale"}}}}}),
    );
    write(
        root.path(),
        "loot_table",
        "modifier_good",
        &json!({"modifier":"minecraft:good_modifier",
        "pools":[{"rolls":1,"condition":"minecraft:good_predicate","entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]}),
    );
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let Err(error) = LootData::load(root.path(), &PackOverlay::NONE, &defaults) else {
        panic!("refused modifiers and predicates loaded")
    };
    let error = error.to_string();
    for (entry, component) in [
        (
            "item modifier minecraft:bad_modifier",
            "minecraft:creative_slot_lock",
        ),
        (
            "predicate minecraft:bad_predicate",
            "minecraft:additional_trade_cost",
        ),
        (
            "item modifier minecraft:bad_number",
            "minecraft:map_post_processing",
        ),
    ] {
        let line = error
            .lines()
            .find(|line| line.starts_with(&format!("{entry}:")))
            .unwrap_or_else(|| panic!("{entry} not listed: {error}"));
        assert!(line.contains(component), "{line}");
    }
    for (kind, name) in [
        ("item_modifier", "bad_modifier"),
        ("predicate", "bad_predicate"),
        ("item_modifier", "bad_number"),
    ] {
        std::fs::remove_file(
            root.path()
                .join(format!("data/minecraft/{kind}/{name}.json")),
        )
        .unwrap();
    }
    let loot = LootData::load(root.path(), &PackOverlay::NONE, &defaults).unwrap();
    let mut random = sunrise_core::random::LegacyRandom::new(0);
    let good_modifier = loot.roll_split("minecraft:modifier_good", &mut random);
    assert_eq!(good_modifier.len(), 1);
    assert_eq!(good_modifier[0].count, 3);
}

#[test]
fn recipe_nested_text_item_values_are_persistent_too() {
    rejects_recipe(
        json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],
        "result":{"id":"minecraft:stick","components":{"minecraft:lore":[{
            "text":"hover","hover_event":{"action":"show_item","id":"minecraft:stone",
                "components":{"minecraft:creative_slot_lock":{}}}}]}}}),
        "minecraft:creative_slot_lock",
    );
}

#[test]
fn command_datastore_filters_winning_files_without_lower_pack_fallback() {
    use sunrise_client::{
        datapack::{DataPack, DataPacks, DataStore},
        resource_source::ResourceSource,
    };
    let root = tempfile::tempdir().unwrap();
    let low = root.path().join("low");
    let high = root.path().join("high");
    write(
        &low,
        "item_modifier",
        "override",
        &json!({"type":"minecraft:set_count","count":9}),
    );
    write(
        &high,
        "item_modifier",
        "override",
        &json!({"type":"minecraft:set_components",
        "components":{"!minecraft:additional_trade_cost":{}}}),
    );
    write(
        &high,
        "item_modifier",
        "z_good",
        &json!({"type":"minecraft:copy_components","source":"block_entity",
        "include":["minecraft:creative_slot_lock"]}),
    );
    write(
        &high,
        "predicate",
        "bad_predicate",
        &json!({"type":"minecraft:match_tool",
        "predicate":{"components":{"minecraft:map_post_processing":"scale"}}}),
    );
    write(
        &high,
        "predicate",
        "z_good",
        &json!({"type":"minecraft:match_tool",
        "predicate":{"predicates":{"minecraft:creative_slot_lock":{}}}}),
    );
    write(
        &high,
        "slot_source",
        "bad_slots",
        &json!({"type":"minecraft:filtered",
        "slot_source":{"type":"minecraft:slot_range","slots":"container.*"},
        "item_filter":{"components":{"minecraft:creative_slot_lock":{}}}}),
    );
    write(
        &high,
        "slot_source",
        "z_good",
        &json!({"type":"minecraft:filtered",
        "slot_source":{"type":"minecraft:slot_range","slots":"container.*"},
        "item_filter":{"predicates":{"minecraft:additional_trade_cost":{}}}}),
    );
    let low = DataPack::from_source("low".into(), ResourceSource::directory(low));
    let high = DataPack::from_source("high".into(), ResourceSource::directory(high));
    let empty_root = root.path().join("empty_root");
    std::fs::create_dir_all(&empty_root).unwrap();
    std::fs::write(empty_root.join("items.json"), "{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(&empty_root).unwrap();
    let store = DataStore::load(&DataPacks::new(&[&low, &high]));
    let files: Vec<_> = store.files.into_iter().collect();
    let overlay = sunrise_world::loot::PackOverlay::new(&files);
    let err = match sunrise_world::loot::LootData::load(&empty_root, &overlay, &defaults) {
        Ok(_) => panic!("expected refused transient components to fail LootData::load"),
        Err(err) => err.to_string(),
    };
    // The refused winners fail the reload, each listed; the lower pack's file does not return.
    for (entry, component) in [
        (
            "item modifier minecraft:override",
            "minecraft:additional_trade_cost",
        ),
        (
            "predicate minecraft:bad_predicate",
            "minecraft:map_post_processing",
        ),
        (
            "slot source minecraft:bad_slots",
            "minecraft:creative_slot_lock",
        ),
    ] {
        assert!(
            err.contains(entry) && err.contains(component),
            "{entry} / {component} missing in {err}"
        );
    }
}

#[test]
fn saving_nested_hover_items_prunes_transients_but_preserves_live_text_and_opaque_data() {
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::item::{Component, component};
    let parse = |source: &str| sunrise_core::nbt::parse_snbt(source).unwrap().0;
    let text = parse(
        r#"{text:"outer",hover_event:{action:"show_item",id:"minecraft:stone",components:{"minecraft:creative_slot_lock":{},"!minecraft:additional_trade_cost":{},"minecraft:custom_data":{keep:7}}}}"#,
    );
    let opaque = parse(r#"{components:{"minecraft:map_post_processing":"opaque"},keep:1}"#);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    stack
        .data_mut()
        .set_other("minecraft:custom_name", Some(text.clone()));
    stack
        .data_mut()
        .set_other("minecraft:custom_data", Some(opaque.clone()));
    let live = item_data::components_tag(0, stack.data());
    let saved = item_nbt::item_to_nbt(&stack, &Items::empty());
    let components = saved.get("components").unwrap();
    assert_eq!(components.get("minecraft:custom_data"), Some(&opaque));
    let saved_text = components.get("minecraft:custom_name").unwrap();
    assert_eq!(saved_text.get("text"), Some(&Tag::String("outer".into())));
    let hovered = saved_text
        .get("hover_event")
        .unwrap()
        .get("components")
        .unwrap();
    assert!(hovered.get("minecraft:creative_slot_lock").is_none());
    assert!(hovered.get("!minecraft:additional_trade_cost").is_none());
    assert_eq!(
        hovered.get("minecraft:custom_data"),
        Some(&parse("{keep:7}"))
    );
    assert_eq!(item_data::components_tag(0, stack.data()), live);
    assert_eq!(
        stack.data().unwrap().other("minecraft:custom_name"),
        Some(&text)
    );
    assert_eq!(item_nbt::item_to_nbt(&stack, &Items::empty()), saved);
    let reloaded = item_nbt::item_from_nbt(&saved).unwrap();
    assert_eq!(item_nbt::item_to_nbt(&reloaded, &Items::empty()), saved);
    let network =
        sunrise_client::lan::guest::stack_to_network(|_| 1, |_, _| None, &Some(stack)).unwrap();
    assert!(
        matches!(network.component(component::CUSTOM_NAME), Some(Component::Text(value)) if value == &text)
    );
}
#[test]
fn an_unknown_loot_function_type_refuses_the_load_naming_the_file() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), "{}").unwrap();
    write(root.path(), "loot_table", "good", &json!({"pools": []}));
    write(
        root.path(),
        "loot_table",
        "bad",
        &json!({"modifier":{"type":"minecraft:no_such_function"},"pools":[]}),
    );
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let Err(error) = LootData::load(root.path(), &PackOverlay::NONE, &defaults) else {
        panic!("an unknown loot function type loaded")
    };
    assert!(error.to_string().contains("minecraft:bad"), "{error}");
}
#[test]
fn an_advancement_requiring_a_missing_criterion_is_refused() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "advancement",
        "ghost",
        &json!({"criteria":{"a":{"trigger":"minecraft:tick"}},"requirements":[["ghost"]]}),
    );
    let tree = Tree::load(&root.path().join("data"));
    assert!(tree.list.is_empty());
    assert_eq!(tree.refused.len(), 1, "{:?}", tree.refused);
    assert!(tree.refused[0].contains("minecraft:ghost"));
}
