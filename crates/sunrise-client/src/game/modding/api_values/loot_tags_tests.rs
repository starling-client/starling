struct TestTags;
impl engine::LootTags for TestTags {
    fn members(&self, _: &str, _: &str) -> Option<Vec<String>> {
        None
    }
}
struct TagReader;
#[mod_api::__subscriptions]
impl Hooks for TagReader {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_loot_function_type(
            &mod_api::Identifier::parse("tag_reader:count").unwrap(),
            TagReader,
        );
        r.register_loot_condition_type(
            &mod_api::Identifier::parse("tag_reader:member").unwrap(),
            TagReader,
        );
    }
}
impl ModLootFunction for TagReader {
    fn validate(&self, _: &LootEntryData) -> Result<(), String> {
        Ok(())
    }
    fn apply(
        &self,
        data: &LootEntryData,
        mut stack: LootStack,
        context: &mut dyn LootTypeContext,
    ) -> Result<LootStack, String> {
        let tags = context.tags();
        let id = data.value()["tag"].as_str().unwrap_or("test:ingredients");
        let tag = tags.key(&mod_api::Identifier::parse(id).unwrap()).unwrap();
        stack.count = match tags.items(tag) {
            mod_api::TagMembers::Members(members) if members.is_empty() => 1,
            mod_api::TagMembers::Members(members) => {
                assert_eq!(
                    members
                        .iter()
                        .map(|key| mod_api::__private::item_key_id(*key).unwrap())
                        .collect::<Vec<_>>(),
                    ["minecraft:potion", "minecraft:stick"]
                );
                2
            }
            mod_api::TagMembers::Missing => 0,
            mod_api::TagMembers::Unavailable => panic!("loot has a loaded item table"),
            other => panic!("unexpected tag lookup result: {other:?}"),
        };
        Ok(stack)
    }
}
impl mod_api::ModLootCondition for TagReader {
    fn validate(&self, _: &LootEntryData) -> Result<(), String> {
        Ok(())
    }
    fn test(&self, _: &LootEntryData, context: &mut dyn LootTypeContext) -> Result<bool, String> {
        let tags = context.tags();
        let tag = tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let holds = tags.is_in_tag(stick, tag).unwrap();
        let missing = tags
            .key(&mod_api::Identifier::parse("test:missing").unwrap())
            .unwrap();
        assert_eq!(tags.is_in_tag(stick, missing), Some(false));
        assert_eq!(tags.items(missing), mod_api::TagMembers::Missing);
        Ok(holds)
    }
}
#[test]
fn loot_types_read_current_membership_and_order_after_tag_reload() {
    use std::collections::HashMap;
    use sunrise_items::items::{Items, TagRegistry};
    super::super::prepare_stack_boundary_tests();
    let mut items = Items::for_tests(HashMap::new());
    with_mods(
        vec![ModEntry::new("tag_reader", Box::new(TagReader))].into(),
        || {
            super::super::prepare_stack_boundary_tests();
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            super::super::resolve_stack_boundary_test_items();
            for (members, expected_count, expected_holds) in [
                (
                    Some(vec![
                        "minecraft:potion".to_owned(),
                        "minecraft:stick".to_owned(),
                    ]),
                    2,
                    true,
                ),
                (Some(Vec::new()), 1, false),
                (None, 0, false),
            ] {
                items.set_tags(
                    TagRegistry::Item,
                    members
                        .into_iter()
                        .map(|members| ("test:ingredients".into(), members))
                        .collect(),
                );
                let mut value = stack();
                let mut holds = false;
                apply_type(
                    "tag_reader:count",
                    &serde_json::json!({}),
                    &context(),
                    &mut value,
                    &items,
                );
                test_type(
                    "tag_reader:member",
                    &serde_json::json!({}),
                    &context(),
                    &mut holds,
                    &items,
                );
                assert_eq!(value.count, expected_count);
                assert_eq!(holds, expected_holds);
            }
        },
    );
}

#[test]
fn registered_loot_function_reads_only_tags_bound_by_the_pack_loader() {
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::loot::{LootData, PackOverlay};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let cases = crate::game::native_tags::PACK_BINDING_CASES;
    let mut files = Vec::new();
    for (name, bytes, _) in cases {
        files.push((format!("tags/binding:item/{name}"), vec![bytes.to_vec()]));
        let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}],
            "modifier":{"type":"tag_reader:count","tag":format!("binding:{name}")}});
        files.push((
            format!("loot_table/binding:{name}"),
            vec![serde_json::to_vec(&table).unwrap()],
        ));
    }
    with_mods(
        vec![ModEntry::new("tag_reader", Box::new(TagReader))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let loot = LootData::load_with_types(
                root.path(),
                &PackOverlay::new(&files),
                &defaults,
                crate::game::loot_types::capture(),
            )
            .unwrap();
            for (name, _, is_bound) in cases {
                let drops = loot.roll(
                    &format!("binding:{name}"),
                    &mut LegacyRandom::new(0),
                    0.0,
                    None,
                    None,
                );
                assert_eq!(
                    drops.iter().map(|stack| stack.count).sum::<i32>(),
                    i32::from(is_bound),
                    "{name}"
                );
            }
        },
    );
}
