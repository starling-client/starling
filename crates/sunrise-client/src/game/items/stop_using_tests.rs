#![cfg(test)]
//! A golden apple's `ItemPart` hears `on_stop_using`
//! (`IItemExtension.onStopUsing`) once per stop, whatever stopped the use:
//! a finish, a release or another item in the hand.
use super::Game;
use crate::game::Screen;
use mod_api::{Hand, Hooks, HostSide, ItemPart, OnStopUsingEvent, Registrar, RegistrationContext};
use std::sync::Mutex;
use sunrise_client::items::ItemStack;

/// What the part heard: `(on the server, entity, main hand, time left)`.
static HEARD: Mutex<Vec<(bool, Option<i32>, bool, i32)>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Apple;
#[mod_api::sunrise_part]
impl ItemPart for Apple {
    fn on_stop_using(&self, host: HostSide, event: &mut OnStopUsingEvent<'_>) {
        HEARD.lock().unwrap().push((
            matches!(host, HostSide::Server(_)),
            event.entity,
            matches!(event.hand, Hand::Main),
            event.time_left,
        ));
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:golden_apple");
        r.item_part(key, Apple);
    }
}

fn heard() -> Vec<(bool, Option<i32>, bool, i32)> {
    HEARD.lock().unwrap().drain(..).collect()
}

/// Starts eating a golden apple with the part on, ticks the use five times,
/// then runs `body`.
fn eating(body: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("apple", Box::new(Parts))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        super::item_use::interact_vibration_tests::with_game("minecraft:golden_apple", |game| {
            game.creative = false;
            game.build_registries();
            game.screen = Screen::Playing;
            game.set_mapping_held("key.use", true);
            assert!(game.start_using_item());
            for _ in 0..5 {
                game.tick_using();
            }
            assert_eq!(game.using, Some((game.selected, 5)));
            assert!(heard().is_empty(), "a use going on isn't a stop");
            body(game);
        })
        .expect("prepared resources");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_hears_each_stop_of_its_items_use_once() {
    crate::game::in_own_process(
        "game::stop_using_tests::a_part_hears_each_stop_of_its_items_use_once",
        || {
            // A finished use: 0 ticks left, heard once the apple is eaten.
            eating(|game| {
                for _ in 0..64 {
                    if game.using.is_none() {
                        break;
                    }
                    game.tick_using();
                }
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, Some(game.own_entity_id()), true, 0)]);
            });
            // A release: the ticks of the use that were left.
            eating(|game| {
                let left = game.use_duration - 5;
                game.set_mapping_held("key.use", false);
                game.tick_using();
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, Some(game.own_entity_id()), true, left)]);
                assert_eq!(game.held().map(|s| s.count), Some(1), "not eaten");
            });
            // Another item in the hand: the apple's use stops, and the apple's part hears it.
            eating(|game| {
                let left = game.use_duration - 5;
                *game.held_mut() = Some(ItemStack::new("minecraft:bow", 1));
                game.tick_using();
                assert!(game.using.is_none());
                assert_eq!(heard(), [(true, Some(game.own_entity_id()), true, left)]);
                // Nothing left to stop: no second stop.
                game.tick_using();
                assert!(heard().is_empty());
            });
        },
    );
}

/// A server's client finishing a golden apple (`handleEntityEvent(9)`) whose slot no longer
/// holds it once `living_entity_use_item_finish` returns (a listener put another stack there):
/// the listener's stack stays, and `stopUsingItem` still ends the use, so the part hears
/// `on_stop_using` once (`LivingEntity.completeUsingItem`).
#[test]
#[ignore = "daily"]
fn a_replaced_finish_still_stops_the_use() {
    use crate::game::{execution_role::ExecutionRole, native_entity_refs::tests};
    use sunrise_client::motion::PlayerMotion;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::connection::Pipe;
    crate::game::in_own_process(
        "game::stop_using_tests::a_replaced_finish_still_stops_the_use",
        || {
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new("apple", Box::new(Parts))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                tests::with_game(|game| {
                    game.options.execution_role = ExecutionRole::Client;
                    let (local, _peer) = Pipe::pair();
                    game.connect_local(local, "StopUsingTest").unwrap();
                    let remote = game.remote.as_mut().unwrap();
                    remote.logged_in = true;
                    remote.entity_id = 71;
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                    tests::bind_local_player(game, 71, 0x0000_0001_0000_0002_0000_0003_0000_0004);
                    game.set_local_mode(sunrise_client::command::args::GameMode::Survival);
                    game.selected = 0;
                    game.screen = Screen::Playing;
                    heard();
                    let apple = ItemStack::new("minecraft:golden_apple", 1);
                    let info = game.items().get(&apple.item).unwrap().clone();
                    let replacement = ItemStack::new("minecraft:bread", 3);
                    game.inventory[0] = Some(replacement.clone());
                    game.remote_use_item_finished(0, &apple, &info);
                    assert_eq!(
                        game.inventory[0].as_ref(),
                        Some(&replacement),
                        "the listener's stack stays"
                    );
                    let stops = heard();
                    assert_eq!(stops.len(), 1, "one stop: {stops:?}");
                    assert!(!stops[0].0, "heard on the client");
                    assert_eq!(stops[0].3, 0, "a finished use has no time left");
                });
            });
        },
    );
}
