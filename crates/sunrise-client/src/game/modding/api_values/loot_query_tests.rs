use super::*;
use mod_api::{Hooks, ItemPart, Registrar, RegistrationContext};
struct Reader;
#[mod_api::__subscriptions]
impl Hooks for Reader {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_loot_condition_type(
            &mod_api::Identifier::parse("reader:query").unwrap(),
            Reader,
        );
    }
}
impl mod_api::ModLootCondition for Reader {
    fn validate(&self, _: &mod_api::LootEntryData) -> Result<(), String> {
        Ok(())
    }
    fn test(
        &self,
        _: &mod_api::LootEntryData,
        context: &mut dyn mod_api::LootTypeContext,
    ) -> Result<bool, String> {
        let snapshot = context.snapshot();
        assert_eq!(snapshot.queried_table_id, "minecraft:blocks/short_grass");
        let stack = snapshot.queried_stack.as_ref().unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(stack.item()).unwrap(),
            "minecraft:stick"
        );
        assert_eq!(stack.count(), 3);
        assert_eq!(stack.damage(), 2);
        Ok(true)
    }
}
#[test]
fn queried_parameters_reach_native_mod_condition() {
    crate::game::api_values::prepare_stack_boundary_tests();
    let ctx = engine::LootContext {
        queried_table_id: Some("minecraft:blocks/short_grass".into()),
        queried_stack: Some(engine::LootStack {
            item: "minecraft:stick".into(),
            count: 3,
            damage: 2,
            enchantments: vec![],
            potion: None,
            map: false,
            exploration: None,
            ominous_amplifier: None,
            stew_effects: vec![],
            names: vec![],
            components: vec![],
        }),
        entity: None,
        damage: None,
        player: None,
        tool: None,
        block: None,
        luck: 0.0,
        biome: None,
        seed: 0,
        place: None,
    };
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("reader", Box::new(Reader))].into(),
        || {
            crate::game::api_values::prepare_stack_boundary_tests();
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            crate::game::api_values::resolve_stack_boundary_test_items();
            let mut random = sunrise_core::random::LegacyRandom::new(0);
            let holds = crate::game::loot_types::capture()
                .test(
                    "reader:query",
                    &serde_json::json!({}),
                    &ctx,
                    &mut random,
                    &Tags,
                )
                .unwrap();
            assert!(holds);
        },
    );
}
#[derive(Clone)]
struct Ability(bool);
#[mod_api::sunrise_part]
impl ItemPart for Ability {
    fn can_perform_action(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::CanPerformActionEvent<'_>,
    ) {
        if event.ability == "shears_dig" {
            event.can_perform = self.0;
        }
    }
}
struct Abilities;
#[mod_api::__subscriptions]
impl Hooks for Abilities {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let grant = r.item_key("minecraft:stick");
        let deny = r.item_key("minecraft:shears");
        r.item_part(grant, Ability(true));
        r.item_part(deny, Ability(false));
    }
}
#[test]
#[ignore = "daily"]
fn queried_ability_parts_grant_and_deny() {
    crate::game::in_own_process(
        "game::api_values::loot_values::query_tests::queried_ability_parts_grant_and_deny",
        queried_ability_parts_grant_and_deny_isolated,
    );
}
fn queried_ability_parts_grant_and_deny_isolated() {
    crate::game::api_values::prepare_stack_boundary_tests();
    assert!(
        sunrise_world::native_load_hooks::install(
            sunrise_world::native_load_hooks::NativeLoadHooks {
                item_ability_default: sunrise_items::item_abilities::vanilla_can_perform,
                loot_ability: dispatch_loot_ability,
                loot_table_load: |_, _, _| {},
                modify_default_components: |_, _, _| {},
            }
        )
        .is_ok()
    );
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("abilities", Box::new(Abilities))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        mod_api::__private::resolve_items([
            "minecraft:stick",
            "minecraft:shears",
            "minecraft:stone_pickaxe",
        ])
        .unwrap();
        for (item, initial, expected) in [
            ("minecraft:stick", false, true),
            ("minecraft:shears", true, false),
            ("minecraft:stone_pickaxe", false, false),
        ] {
            let tool = engine::LootStack {
                item: item.into(),
                count: 1,
                damage: 0,
                enchantments: vec![],
                potion: None,
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: vec![],
                names: vec![],
                components: vec![],
            };
            let items = sunrise_items::items::Items::for_tests(Default::default());
            let mut holds = items.vanilla_can_perform(item, "shears_dig");
            assert_eq!(holds, initial);
            dispatch_loot_ability(&tool, "shears_dig", &mut holds);
            assert_eq!(holds, expected);
            let query = engine::query::LootQuery::default();
            for ability in ["shears_dig", "unknown_ability", "minecraft:shears_dig"] {
                let condition = serde_json::json!({"type":"neoforge:can_item_perform_ability","ability":ability});
                let answer =
                    engine::query::condition(&condition, &query, Some(&tool), |item, ability| {
                        items.vanilla_can_perform(item, ability)
                    });
                assert_eq!(answer, Some(ability == "shears_dig" && expected));
                assert_eq!(
                    engine::query::condition(&condition, &query, None, |item, ability| items
                        .vanilla_can_perform(item, ability)),
                    Some(false)
                );
            }
        }
    });
}

struct Tags;
impl engine::LootTags for Tags {
    fn members(&self, _: &str, _: &str) -> Option<Vec<String>> {
        None
    }
}
