//! `menu_slot_rules` through a real `MenuPart` on the read-only `Host`: what a click asks, in
//! what order, how often, and what a click does without a listener, with a failing one and with
//! one that answers out of range. The fixtures need prepared resources.
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_guest_turn_tests::with_guests;
use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
use std::cell::RefCell;
use sunrise_client::{
    items::ItemStack,
    menu::{Input, Kind, Menu},
};

thread_local! {
    /// Every question the part was asked: slot, item and count.
    static ASKED: RefCell<Vec<(usize, String, u32)>> = const { RefCell::new(Vec::new()) };
    /// What the part answers for `maximum`, and whether it panics on dirt.
    static ANSWER: RefCell<(Option<u32>, bool)> = const { RefCell::new((None, false)) };
    /// Whether the guest 12 resolved through the `Host`, at each question.
    static SAW_GUEST: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

struct Rules;
#[mod_api::sunrise_part]
impl MenuPart for Rules {
    fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        SAW_GUEST.with(|saw| saw.borrow_mut().push(host.player(12).is_some()));
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        ASKED.with(|asked| {
            asked
                .borrow_mut()
                .push((event.slot, item.clone(), event.stack.count()))
        });
        let (maximum, panic) = ANSWER.with(|answer| *answer.borrow());
        if panic && item == "minecraft:dirt" {
            event.max_stack = 0;
            panic!("a listener failed");
        }
        if let Some(maximum) = maximum {
            event.max_stack = maximum;
        }
    }
}
struct Listener;
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", Rules);
    }
}
/// A mod with no part for any menu.
struct Quiet;
#[mod_api::__subscriptions]
impl Hooks for Quiet {}

fn reset(maximum: Option<u32>, panic: bool) {
    SAW_GUEST.with(|saw| saw.borrow_mut().clear());
    ASKED.with(|asked| asked.borrow_mut().clear());
    ANSWER.with(|answer| *answer.borrow_mut() = (maximum, panic));
}
fn asked() -> Vec<(usize, String, u32)> {
    ASKED.with(|asked| asked.borrow().clone())
}

/// A chest menu with its type registered, empty, the cursor holding `count` of `item`.
fn chest(carried: (&str, u32)) -> Menu {
    let mut menu = Menu::new(Kind::Chest(3));
    menu.set_registry_id(Some("minecraft:generic_9x3"));
    menu.carried = Some(ItemStack::new(carried.0, carried.1));
    menu
}
/// What a pickup click on slot 0 leaves: the slot and the cursor.
fn pickup(game: &crate::game::Game<'_>, menu: &mut Menu) -> (u32, u32) {
    menu.click(&game.menu_env(), 0, 0, Input::Pickup).unwrap();
    (
        menu.slot(0).map_or(0, |s| s.count),
        menu.carried.as_ref().map_or(0, |s| s.count),
    )
}

fn with_mods(mods: Vec<ModEntry>, test: impl FnOnce()) {
    let mods: Mods = mods.into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

struct StagedReader;
#[mod_api::sunrise_part]
impl MenuPart for StagedReader {
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
        use mod_api::MenuContainer::{Main, Player};
        assert!(event.context.slot(Main, 2).is_none()); // unknown item
        assert!(event.context.slot(Main, usize::MAX).is_none());
        // Live player inventory, armor (36..40 feet-to-head) and offhand (40) are readable.
        let player_stone = event.context.slot(Player, 0).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(player_stone.item()),
            Some("minecraft:stone")
        );
        assert_eq!(player_stone.count(), 1);
        assert!(event.context.slot(Player, 1).is_none());
        let boots = event.context.slot(Player, 36).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(boots.item()),
            Some("minecraft:iron_boots")
        );
        let offhand = event.context.slot(Player, 40).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(offhand.item()),
            Some("minecraft:shield")
        );
        let ingredient = event.context.slot(Main, 1).unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(ingredient.item()),
            Some("minecraft:oak_planks")
        );
        event.max_stack = ingredient.count();
    }
}

struct StagedListener;
#[mod_api::__subscriptions]
impl Hooks for StagedListener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x3", StagedReader);
    }
}

#[test]
#[ignore = "daily"]
fn staged_slot_rule_reads_hide_unrepresentable_and_player_stacks() {
    with_mods(
        vec![ModEntry::new("rules", Box::new(StagedListener))],
        || {
            with_game(|game| {
                let mut menu = chest(("minecraft:stone", 8));
                menu.load_slot(1, Some(ItemStack::new("minecraft:oak_planks", 3)));
                menu.load_slot(2, Some(ItemStack::new("test:unregistered", 1)));
                menu.set_player_item(0, Some(ItemStack::new("minecraft:stone", 1)));
                menu.set_player_item(36, Some(ItemStack::new("minecraft:iron_boots", 1)));
                menu.set_player_item(40, Some(ItemStack::new("minecraft:shield", 1)));
                assert_eq!(pickup(game, &mut menu), (3, 5));
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_click_asks_the_part_for_each_slot_and_stack_it_consults_in_order() {
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            reset(None, false);
            let mut menu = chest(("minecraft:stone", 8));
            // Slot.safeInsert asks separately about placement and capacity.
            assert_eq!(pickup(game, &mut menu), (8, 0));
            let stone = |_| (0, "minecraft:stone".to_owned(), 8);
            assert_eq!(asked(), [stone(()), stone(())]);
            // Taking the stack back asks once: `Slot.allowModification` is `mayPickup` and
            // `mayPlace` of the slot's own stack, eight stone in slot 0.
            reset(None, false);
            menu.carried = None;
            assert_eq!(pickup(game, &mut menu), (0, 8));
            assert_eq!(asked(), [(0, "minecraft:stone".to_owned(), 8)]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_answers_are_bounded_to_one_to_ninety_nine_and_the_items_limit() {
    use sunrise_client::lan::menu::Slots;
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            // A maximum of 0 holds none (leaving the slot empty), 3 holds three, and a huge one
            // holds what the item stacks to: 64 stone, not 99 and not the answer.
            for (answer, held) in [(0, 0), (3, 3), (u32::MAX, 64)] {
                reset(Some(answer), false);
                let mut menu = chest(("minecraft:stone", 64));
                assert_eq!(pickup(game, &mut menu), (held, 64 - held), "{answer}");
                if answer == 0 {
                    assert!(menu.slot(0).is_none());
                }
            }
        });
        // A LAN view click with max_stack == 0 likewise places nothing and leaves no zero-count
        // stack behind.
        with_guests(|game| {
            let pos = (1, 64, 1);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
            game.world.block_entities.insert(
                pos,
                sunrise_client::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let host = game.lan.as_mut().unwrap();
            host.hub.open_container(
                12,
                "minecraft:generic_9x3",
                sunrise_core::nbt::Tag::String("test".into()),
                pos,
                Slots::Block(pos),
                vec![None; 27],
                None,
            );
            let window = host.hub.guests[0].menu.as_ref().unwrap().window;
            host.hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stone", 64))
            });
            reset(Some(0), false);
            game.lan_guest_packet(
                12,
                Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            assert!(game.world.block_entities.get(&pos).unwrap().slots()[0].is_none());
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref()
                    .map(|s| s.count),
                Some(64)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_failing_slot_rule_does_not_override_or_disable_the_other_parts() {
    struct Healthy;
    #[mod_api::__subscriptions]
    impl Hooks for Healthy {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x3", Healthy);
        }
    }
    #[mod_api::sunrise_part]
    impl MenuPart for Healthy {
        fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            assert!(
                event.max_stack > 0,
                "the failed part's answer must not be visible"
            );
            event.may_place = false;
        }
    }
    with_mods(
        vec![
            ModEntry::new("rules", Box::new(Listener)),
            ModEntry::new("zz_healthy_rules", Box::new(Healthy)),
        ],
        || {
            with_game(|game| {
                reset(None, true);
                let menu = chest(("minecraft:dirt", 8));
                let dirt = menu.carried.as_ref().unwrap();
                let env = game.menu_env();
                // One live check: only the healthy part's refusal takes effect.
                assert_eq!(menu.slot_holds(&env, 0, dirt), 0);
                assert_eq!(asked(), [(0, "minecraft:dirt".to_owned(), 8)]);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "rules",
                        hook: "menu_slot_rules",
                    }]
                );
                reset(None, true);
                assert_eq!(menu.slot_holds(&env, 0, dirt), 0);
                assert!(asked().is_empty());
                assert!(mod_api::__private::take_hook_panics().is_empty());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn without_a_part_nothing_is_asked_and_the_click_is_the_same() {
    // The same click with no mod installed, with a mod that has no part, and with a part on
    // another menu type: the final state is identical and the part is never asked.
    let run = |game: &crate::game::Game<'_>, registry: &str| {
        let mut menu = chest(("minecraft:stone", 8));
        menu.set_registry_id(Some(registry));
        pickup(game, &mut menu)
    };
    let baseline = RefCell::new(None);
    with_game(|game| {
        reset(None, false);
        *baseline.borrow_mut() = Some(run(game, "minecraft:generic_9x3"));
        assert_eq!(asked(), []);
    });
    with_mods(vec![ModEntry::new("quiet", Box::new(Quiet))], || {
        with_game(|game| {
            reset(None, false);
            assert_eq!(Some(run(game, "minecraft:generic_9x3")), *baseline.borrow());
            assert_eq!(asked(), []);
        });
    });
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_game(|game| {
            reset(None, false);
            // A menu type the part is not attached to: not asked, same answer.
            assert_eq!(Some(run(game, "minecraft:generic_9x6")), *baseline.borrow());
            assert_eq!(asked(), []);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_lan_hub_click_asks_the_same_questions_in_the_same_order() {
    use sunrise_client::lan::menu::Slots;
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    with_mods(vec![ModEntry::new("rules", Box::new(Listener))], || {
        with_guests(|game| {
            let pos = (1, 64, 1);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
            game.world.block_entities.insert(
                pos,
                sunrise_client::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let host = game.lan.as_mut().unwrap();
            host.hub.open_container(
                12,
                "minecraft:generic_9x3",
                sunrise_core::nbt::Tag::String("test".into()),
                pos,
                Slots::Block(pos),
                vec![None; 27],
                None,
            );
            let window = host.hub.guests[0].menu.as_ref().unwrap().window;
            host.hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stone", 8))
            });
            reset(None, false);
            game.lan_guest_packet(
                12,
                Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            let lan = asked();
            // The hub's click ran with the LAN host lifted out of the game, and is back. The
            // `Host` read meanwhile answered as if no LAN were open: the guest is not found.
            assert!(game.lan.is_some());
            assert_eq!(SAW_GUEST.with(|saw| saw.borrow().clone()), [false, false]);
            assert!(game.player(12).is_some());
            // The local click of the same state asks the same, in the same order.
            reset(None, false);
            let mut menu = chest(("minecraft:stone", 8));
            assert_eq!(pickup(game, &mut menu), (8, 0));
            assert_eq!(lan, asked());
            assert_eq!(lan.len(), 2);
            // Both questions on the local route can read the guest through the Host.
            assert_eq!(SAW_GUEST.with(|saw| saw.borrow().clone()), [true, true]);
            let placed = game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .map_or(0, |s| s.count);
            assert_eq!(placed, 8);
        });
    });
}

#[test]
#[ignore = "daily"]
fn nested_menu_reads_use_base_slot_rules() {
    use crate::game::api_values::test_snapshot;
    use sunrise_client::{chunk::Chunk, containers::BlockEntity, lan::menu::Slots};

    thread_local! {
        static CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    struct ReadsOwnMenu;
    #[mod_api::sunrise_part]
    impl MenuPart for ReadsOwnMenu {
        fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            let calls = CALLS.get() + 1;
            CALLS.set(calls);
            // Without the guard, make reentry fail the assertions rather than overflow.
            if calls > 2 {
                event.max_stack = 1;
                return;
            }
            let Some(menu) = event
                .context
                .player
                .and_then(|uuid| host.player(uuid))
                .and_then(|player| player.container_menu(host))
            else {
                return;
            };
            if matches!(menu.slot(host, 0), Some(None))
                && menu.carried(host).flatten().map(|stack| stack.count()) == Some(8)
                && menu.slot_holds(host, 0, event.stack) == Some(64)
                && menu.slot_holds(host, 1, event.stack) == Some(64)
            {
                event.max_stack = 3;
            }
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for ReadsOwnMenu {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x3", ReadsOwnMenu);
        }
    }

    crate::game::in_own_process(
        "game::slot_rules_tests::nested_menu_reads_use_base_slot_rules",
        || {
            with_mods(vec![ModEntry::new("rules", Box::new(ReadsOwnMenu))], || {
                with_guests(|game| {
                    let pos = (1, 64, 1);
                    game.world
                        .chunks
                        .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                    let state = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:chest")
                        .unwrap();
                    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                    game.world
                        .block_entities
                        .insert(pos, BlockEntity::Chest(vec![None; 27]));
                    let host = game.lan.as_mut().unwrap();
                    host.hub.open_container(
                        12,
                        "minecraft:generic_9x3",
                        sunrise_core::nbt::Tag::String("test".into()),
                        pos,
                        Slots::Block(pos),
                        vec![None; 27],
                        None,
                    );
                    host.hub.guests.update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:stone", 8));
                    });
                    let player = game.player(12).unwrap();
                    let menu = player.container_menu(game).unwrap();
                    let stone = test_snapshot(&ItemStack::new("minecraft:stone", 8));
                    CALLS.set(0);
                    assert_eq!(menu.slot_holds(game, 0, &stone), Some(3));
                    // One live placement check and one live capacity check.
                    assert_eq!(CALLS.get(), 2);
                });
            });
        },
    );
}
