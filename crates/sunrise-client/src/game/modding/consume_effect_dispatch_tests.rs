//! Explicit living consume effects reach real native listeners, including nested kinds.
#![cfg(test)]
use crate::game::{api_values::TryIntoApi, native_entity_refs::tests::with_game};
use mod_api::{ConsumeEffectEvent, Hooks, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_client::{
    items::{ConsumeEffect, ItemStack},
    mobs::Mob,
};
use sunrise_core::{geometry::Vec3, nbt::Tag};

struct First(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for First {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        // Both listeners subscribe to all custom consume entries. The engine,
        // not a guard in the listener, must keep other kinds out of this callback.
        assert_eq!(event.kind, "first:effect");
        self.0.lock().unwrap().push("first");
        let source = host.entity(21).unwrap();
        assert_eq!(event.user, source);
        assert_eq!(event.stack.count(), 1);
        assert_eq!(
            event.stack.item(),
            host.item_key(&mod_api::Identifier::parse("minecraft:pufferfish").unwrap())
                .unwrap()
        );
        let living = host.living(source).unwrap();
        let same = ItemStack::new("minecraft:pufferfish", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &same, 0), Ok(Some(false)));
        let other = ItemStack::new("minecraft:honey_bottle", 1)
            .try_into_api()
            .unwrap();
        assert_eq!(living.apply_consume_effect(host, &other, 0), Ok(Some(true)));
        self.0.lock().unwrap().push("first_after");
    }
}
struct Second(Arc<Mutex<Vec<&'static str>>>);
#[mod_api::__subscriptions]
impl Hooks for Second {
    fn consume_effect(&self, _: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        assert_eq!(event.kind, "second:effect");
        self.0.lock().unwrap().push("second");
    }
}

#[test]
#[ignore = "daily"]
fn a_living_handle_dispatches_custom_kinds_with_same_kind_recursion_refused() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("first", Box::new(First(trace.clone()))),
        ModEntry::new("second", Box::new(Second(trace.clone()))),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut items = game.items().clone();
            for (item, kind) in [
                ("minecraft:pufferfish", "first:effect"),
                ("minecraft:honey_bottle", "second:effect"),
                ("minecraft:apple", "unclaimed:effect"),
            ] {
                items
                    .items
                    .get_mut(item)
                    .unwrap()
                    .consumable
                    .as_mut()
                    .unwrap()
                    .effects = vec![ConsumeEffect::Native {
                    kind: kind.to_owned(),
                    data: Arc::new(Tag::Compound(Default::default())),
                    registered: None,
                }];
            }
            game.server_items = Some(Arc::new(items));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs
                .push(Mob::new(21, kind, Vec3::new(0.0, 64.0, 0.0), 0.0));
            let living = game.living(game.entity(21).unwrap()).unwrap();
            let stack = ItemStack::new("minecraft:pufferfish", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(living.apply_consume_effect(game, &stack, 0), Ok(Some(true)));
            assert_eq!(*trace.lock().unwrap(), ["first", "second", "first_after"]);
            let unclaimed = ItemStack::new("minecraft:apple", 1).try_into_api().unwrap();
            living.apply_consume_effect(game, &unclaimed, 0).unwrap();
            assert_eq!(*trace.lock().unwrap(), ["first", "second", "first_after"]);
            assert_eq!(living.apply_consume_effect(game, &stack, 0), Ok(Some(true)));
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    "first",
                    "second",
                    "first_after",
                    "first",
                    "second",
                    "first_after"
                ]
            );
        });
    });
}

struct PlayerConsumption(Arc<Mutex<Vec<(mod_api::EntityRef, u32)>>>);
#[mod_api::__subscriptions]
impl Hooks for PlayerConsumption {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        if event.kind == "player_meal:effect" {
            assert_eq!(host.entity(event.user.id()), Some(event.user));
            self.0
                .lock()
                .unwrap()
                .push((event.user, event.stack.count()));
        }
    }
}

#[test]
#[ignore = "daily"]
fn consuming_player_is_supplied_outside_a_guest_action() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "player_meal",
            Box::new(PlayerConsumption(heard.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let mut body = sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    Vec3::new(0.0, 65.0, 0.0),
                )
                .unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                game.bind_local_player_residency();
                assert!(game.native_acting_player().is_none());
                let user = game.entity(7).unwrap();
                let stack = ItemStack::new("minecraft:pufferfish", 4);
                let info = game.items().get(&stack.item).unwrap().clone();
                let mut consumable = game.consumable(&stack).unwrap();
                consumable.effects = vec![ConsumeEffect::Native {
                    kind: "player_meal:effect".into(),
                    data: Arc::new(Tag::Compound(Default::default())),
                    registered: None,
                }];
                game.creative = false;
                game.inventory[game.selected] = Some(stack.clone());
                game.finish_consuming(&info, &consumable, &stack);
                assert_eq!(*heard.lock().unwrap(), [(user, 4)]);
                assert_eq!(game.held().unwrap().count, 3);
            })
        },
    );
}

struct ReplaceDrink;
#[mod_api::__subscriptions]
impl Hooks for ReplaceDrink {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let expected =
            host.item_key(&mod_api::Identifier::parse("minecraft:honey_bottle").unwrap());
        assert_eq!(Some(event.stack.item()), expected);
        let living = host.living(event.user).unwrap();
        let replacement = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        assert_eq!(
            living.set_hand_item(host, mod_api::Hand::Main, Some(replacement)),
            Ok(true)
        );
    }
}

#[test]
#[ignore = "daily"]
fn replacing_the_consumed_hand_keeps_the_new_item_and_emits_no_remainder() {
    let mods = vec![ModEntry::new("replace_drink", Box::new(ReplaceDrink))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut body =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                    .unwrap();
            body.entity_id = 7;
            game.player = Some(body);
            game.bind_local_player_residency();
            game.creative = false;
            game.inventory.fill(None);
            let mut stack = ItemStack::new("minecraft:honey_bottle", 1);
            stack.data_mut().set_other(
                "minecraft:consumable",
                Some(Tag::compound([(
                    "on_consume_effects",
                    Tag::List(vec![Tag::compound([(
                        "type",
                        Tag::String("replace_drink:drink".into()),
                    )])]),
                )])),
            );
            let info = game.items().get(&stack.item).unwrap().clone();
            let consumable = game.consumable(&stack).unwrap();
            game.inventory[game.selected] = Some(stack.clone());
            game.finish_consuming(&info, &consumable, &stack);
            assert_eq!(game.held(), Some(&ItemStack::new("minecraft:stick", 1)));
            assert_eq!(game.inventory.iter().flatten().count(), 1);
            assert!(game.dropped.is_empty());
        });
    });
}

struct ObservePotion(Arc<Mutex<Vec<(String, bool)>>>);
#[mod_api::__subscriptions]
impl Hooks for ObservePotion {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        let living = host.living(event.user).unwrap();
        let breathing = host.effect_key("minecraft:water_breathing").unwrap();
        let has_effect = living
            .effects(host)
            .unwrap()
            .iter()
            .any(|effect| effect.id == breathing);
        assert_eq!(event.stack.count(), 1);
        self.0
            .lock()
            .unwrap()
            .push((event.kind.to_owned(), has_effect));
    }
}

#[test]
#[ignore = "daily"]
fn potion_finishes_with_its_patched_effects_after_potion_contents() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "potion_order",
        Box::new(ObservePotion(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut body =
                sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 65.0, 0.0))
                    .unwrap();
            body.entity_id = 7;
            game.player = Some(body);
            game.bind_local_player_residency();
            game.creative = false;
            let mut potion =
                ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water_breathing");
            potion.data_mut().set_other(
                "minecraft:consumable",
                Some(Tag::compound([(
                    "on_consume_effects",
                    Tag::List(
                        [
                            "potion_order:before_clear",
                            "minecraft:clear_all_effects",
                            "potion_order:after_clear",
                        ]
                        .map(|id| Tag::compound([("type", Tag::String(id.into()))]))
                        .into(),
                    ),
                )])),
            );
            let info = game.items().get(&potion.item).unwrap().clone();
            let slot = game.selected;
            game.inventory[slot] = Some(potion.clone());
            game.complete_using_item(slot, &potion, &info);
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    ("potion_order:before_clear".into(), true),
                    ("potion_order:after_clear".into(), false)
                ]
            );
            assert_eq!(
                game.held(),
                Some(&ItemStack::new("minecraft:glass_bottle", 1))
            );
        });
    });
}

struct FoxConsumption(Arc<Mutex<Vec<(mod_api::EntityRef, mod_api::ItemStack)>>>);
#[mod_api::__subscriptions]
impl Hooks for FoxConsumption {
    fn consume_effect(&self, host: &mut dyn ServerHost, event: &mut ConsumeEffectEvent<'_>) {
        match event.kind {
            "fox_meal:discard" => assert!(event.user.discard(host)),
            "fox_meal:remember" => {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.user, event.stack.clone()));
            }
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_foxs_later_custom_effect_keeps_the_original_user_and_stack_after_discard() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "fox_meal",
        Box::new(FoxConsumption(heard.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let mut items = game.items().clone();
            let effects = &mut items
                .items
                .get_mut("minecraft:chorus_fruit")
                .unwrap()
                .consumable
                .as_mut()
                .unwrap()
                .effects;
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, ConsumeEffect::TeleportRandomly(_)))
            );
            let custom = |kind: &str| ConsumeEffect::Native {
                kind: kind.into(),
                data: Arc::new(Tag::Compound(Default::default())),
                registered: None,
            };
            effects.insert(0, custom("fox_meal:discard"));
            effects.push(ConsumeEffect::PlaySound("minecraft:entity.fox.eat".into()));
            effects.push(custom("fox_meal:remember"));
            game.server_items = Some(Arc::new(items));

            let kind = game.res.mob_types().get("minecraft:fox").unwrap().clone();
            let mut fox = Mob::new(21, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
            fox.ground.on_ground = true;
            fox.extra.ints[1] = 601;
            let mut stack = ItemStack::new("minecraft:chorus_fruit", 7);
            stack
                .data_mut()
                .set_loot_name("minecraft:custom_name", &serde_json::json!("Fox fruit"));
            let expected_stack = stack.try_into_api().unwrap();
            fox.extra.set_main_hand_stack(Some(stack));
            let decision =
                sunrise_client::mobs::fox::decide_consumption(&fox, &game.mob_world_base(&[]))
                    .unwrap();
            game.mobs.push(fox);
            let user = game.entity(21).unwrap();
            assert!(!game.apply_fox_consumption(&decision));
            assert!(game.entity(21).is_none());
            assert_eq!(*heard.lock().unwrap(), [(user, expected_stack)]);
        });
    });
}
