use super::tests::{MOD_MENU, mod_menu_guest, sent};
use crate::game::{
    Game,
    api_values::{self, TryIntoApi},
    native_entity_refs::tests::with_game,
};
use mod_api::{
    Flow, Hand, Hooks, HostTransaction, ItemPlace, ItemResource, MenuClickedEvent, MenuClosedEvent,
    MenuItemsRefusal, MenuKey, MenuPart, MenuRef, MenuReplace, MenuSlotBackRefusal,
    MenuSlotBacking, MenuSlotPlace, MenuStillValidEvent, ModEntry, Mods, Registrar, ServerHost,
};
use std::sync::mpsc;
use sunrise_client::{items::ItemStack, lan::Out};
use sunrise_protocol::packets::play::{
    Clientbound as C, ContainerClick, ContainerClose, Serverbound as S, SetCursorItem,
};

thread_local! {
    static SCRIPTED_CLICK: std::cell::Cell<Option<Flow>> = const { std::cell::Cell::new(None) };
    static WINDOW_REQUEST: std::cell::Cell<Option<(MenuKey, bool)>> = const { std::cell::Cell::new(None) };
    static CLOSE_NOTICES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCK_FIRST_SLOT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct Part;
#[mod_api::sunrise_part]
impl MenuPart for Part {
    fn menu_still_valid(&self, _: &dyn mod_api::LoopContext, event: &mut MenuStillValidEvent<'_>) {
        if let Some(item) = event.opening_item {
            assert_eq!(item.hand, Hand::Off);
            event.valid = item.stack_matches;
        }
    }
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut mod_api::MenuSlotRulesEvent<'_>) {
        if LOCK_FIRST_SLOT.get() && event.slot == 0 {
            assert_eq!(event.context.player, Some(123));
            assert!(
                event
                    .context
                    .slot(mod_api::MenuContainer::Main, 0)
                    .is_some()
            );
            event.may_pickup = false;
            event.may_place = false;
        }
    }

    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        if let Some((key, by_handle)) = WINDOW_REQUEST.get() {
            let title = mod_api::Text::literal("Would replace the current menu");
            assert_eq!(
                host.open_menu(
                    event.uuid,
                    1,
                    title.clone(),
                    Vec::new(),
                    MenuReplace::InPlace
                ),
                Ok(None)
            );
            let player = host.player(event.uuid).unwrap();
            assert_eq!(
                player.open_mod_menu(host, mod_api::ModMenuOpen::new(key, title)),
                Err(mod_api::OpenMenuError::ClickInProgress)
            );
            if by_handle {
                let menu = player.container_menu(host).unwrap();
                assert!(menu.close_container(host));
            } else {
                host.close_menu(event.uuid);
            }
            assert!(
                host.menu(event.uuid, event.window).is_some(),
                "close is deferred"
            );
            return Flow::Cancel;
        }
        if let Some(flow) = SCRIPTED_CLICK.get() {
            return flow;
        }
        if event.slot != 1 {
            return Flow::Continue;
        }
        let Some(menu) = host.menu(event.uuid, event.window) else {
            return Flow::Continue;
        };
        let Some(cursor) = menu.carried(host).flatten() else {
            return Flow::Continue;
        };
        host.transaction(|host, tx| {
            ItemPlace::MenuSlot { menu, slot: 1 }.insert(host, tx, &ItemResource::of(&cursor), 1);
            Ok::<_, ()>(())
        })
        .unwrap();
        Flow::Cancel
    }
}

struct Listener {
    return_contents: bool,
}

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(MOD_MENU, Part);
        r.register_menu_screen(
            MOD_MENU,
            std::sync::Arc::new(
                |_: &dyn mod_api::ClientHost, _: mod_api::MenuScreenOpen<'_>| {
                    Some(mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![Some(mod_api::ScreenSlot::default()); 2],
                        ..Default::default()
                    }))
                },
            ),
        )
        .unwrap();
    }

    fn menu_closed(&self, host: &mut dyn ServerHost, event: &mut MenuClosedEvent<'_>) {
        CLOSE_NOTICES.set(CLOSE_NOTICES.get() + 1);
        if !self.return_contents {
            return;
        }
        assert_eq!(event.opening_hand, Some(Hand::Off));
        assert!(host.menu(event.uuid, event.window).is_none());
        let player = host.player(event.uuid).unwrap();
        host.transaction(|host, _| {
            for stack in event.owned_slots.iter().flatten() {
                assert!(player.give_stack(host, stack.clone()).unwrap().is_none());
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}

fn with_owned_menu(
    return_contents: bool,
    test: impl FnOnce(&mut Game<'_>, mpsc::Receiver<Out>, MenuKey),
) {
    with_owned_layout(return_contents, false, test);
}

fn with_owned_layout(
    return_contents: bool,
    player_inventory: bool,
    test: impl FnOnce(&mut Game<'_>, mpsc::Receiver<Out>, MenuKey),
) {
    let mods: Mods = vec![ModEntry::new(
        "listen",
        Box::new(Listener { return_contents }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        api_values::install_parts(&mods).unwrap();
        let _callbacks = sunrise_client::native_client_hooks::scoped_menu_validity_hooks(
            api_values::menu_still_valid,
            |menu| mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu),
        );
        sunrise_client::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                2,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                player_inventory,
                Some(vec![1, 4]),
            )],
            || {
                with_game(|game| {
                    let (rx, key) = mod_menu_guest(game);
                    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender.menu_extra_rev = sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                        g.offhand = Some(ItemStack::new("minecraft:paper", 2));
                    });
                    test(game, rx, key);
                });
            },
        );
    });
}

fn open(game: &mut Game<'_>, key: MenuKey) -> (i32, MenuRef) {
    let player = game.player(123).unwrap();
    // Open from the acting player's item action, when the hand lives in the game,
    // not in the guest's record.
    let window = game
        .as_guest(1, |game| {
            player.open_mod_menu(game, {
                let mut request =
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Owned slots"));
                request.opening_hand = Some(Hand::Off);
                request
            })
        })
        .unwrap()
        .unwrap()
        .unwrap();
    (window, game.player_container_menu(player).unwrap())
}

fn click(game: &mut Game<'_>, container: i32, slot: i16) {
    game.lan_guest_packet(
        1,
        S::ContainerClick(ContainerClick {
            container,
            state: 0,
            slot,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        }),
    );
    game.run_native_menu_events();
}

#[test]
#[ignore = "daily"]
fn an_item_opened_menu_has_two_empty_owned_slots_with_individual_caps() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let packets = sent(&rx);
        assert!(packets.iter().any(|p| matches!(p,
            C::ContainerSetContent(content) if content.container == window
                && content.items == vec![None, None]
        )));
        assert_eq!(menu.slot_count(game), Some(2));
        let player = game.player(123).unwrap();
        assert_eq!(
            player.back_mod_menu_slots(
                game,
                window,
                vec![MenuSlotPlace::HeldItem {
                    hand: Hand::Off,
                    slot: 0
                }],
            ),
            Err(MenuSlotBackRefusal::UnsupportedMenu),
        );
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let resource = ItemResource::of(&stick);
        for (slot, cap) in [1, 4].into_iter().enumerate() {
            assert_eq!(menu.slot(game, slot), Some(None));
            assert_eq!(
                menu.slot_backing(game, slot),
                Some(MenuSlotBacking::Owned(slot))
            );
            assert_eq!(menu.slot_holds(game, slot, &stick), Some(cap));
            let host: &mut dyn ServerHost = game;
            host.transaction(|host, tx| {
                let place = ItemPlace::MenuSlot { menu, slot };
                assert_eq!(place.insert(host, tx, &resource, 8), cap);
                assert_eq!(place.with_stack(host, |stack| stack.count()), Some(cap));
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        let host: &mut dyn ServerHost = game;
        assert_eq!(
            host.menu_set_items(123, window, vec![]),
            Err(MenuItemsRefusal::OwnedSlots)
        );
        assert_eq!(menu.slot(host, 0), Some(Some(stick)));
    });
}

#[test]
#[ignore = "daily"]
fn opening_an_owned_menu_with_a_result_keeps_the_old_window() {
    with_owned_menu(false, |game, rx, key| {
        let (_, menu) = open(game, key);
        sent(&rx);
        // Bypass package validation to exercise the binding boundary itself.
        sunrise_client::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                2,
                0,
                vec![1],
                vec![],
                None,
                None,
                false,
                Some(vec![1, 4]),
            )],
            || {
                let player = game.player(123).unwrap();
                assert_eq!(
                    player.open_mod_menu(
                        game,
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Invalid result")),
                    ),
                    Err(mod_api::OpenMenuError::OwnedSlotsWithResults)
                );
                assert_eq!(game.player_container_menu(player), Some(menu));
                assert!(sent(&rx).is_empty());
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_cursor_cleared_outside_a_click_is_sent_once_on_the_next_broadcast() {
    with_owned_menu(false, |game, rx, key| {
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.carried = Some(ItemStack::new("minecraft:stick", 2));
        });
        let (window, menu) = open(game, key);
        sent(&rx);
        let cursor = menu.cursor();
        let stick = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        let resource = ItemResource::of(&stick);
        game.transaction(|host, tx| {
            assert_eq!(cursor.insert(host, tx, &resource, 3), 3);
            Ok::<_, ()>(())
        })
        .unwrap();

        // Establish what the client sees through a full menu sync, not a click prediction.
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.resend_menu(1, window, &game.world);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerSetContent(content) if content.container == window
                && content.carried.as_ref().map(|item| item.count) == Some(3)
        )));
        hub.sync_containers(&game.world);
        assert!(
            sent(&rx).is_empty(),
            "the full sync already sent the cursor"
        );

        game.transaction(|host, tx| {
            assert_eq!(cursor.extract(host, tx, &resource, 3), 3);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(sent(&rx).is_empty(), "nothing before the broadcast");

        let hub = &mut game.lan.as_mut().unwrap().hub;
        // A repair of window 0 does not tell the open window about the clear.
        hub.send_inventory(0);
        assert!(matches!(&sent(&rx)[..], [C::ContainerSetContent(content)]
            if content.container == 0 && content.carried.as_ref().map(|item| item.count) == Some(2)));
        hub.sync_containers(&game.world);
        let packets = sent(&rx);
        assert!(
            matches!(
                &packets[..],
                [C::SetCursorItem(SetCursorItem { item: None })]
            ),
            "{packets:?}"
        );
        hub.sync_containers(&game.world);
        assert!(sent(&rx).is_empty(), "the empty cursor was already sent");
    });
}

#[test]
#[ignore = "daily"]
fn owned_click_prediction_applies_caps_and_the_server_corrects_an_override() {
    use crate::game::{execution_role::ExecutionRole, inventory_slots::MOD_MENU_SLOT_BASE};
    use sunrise_client::menu::Input;
    use sunrise_protocol::item::Hashed;
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let opening = sent(&rx);
        let state = opening
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => Some(p.state),
                _ => None,
            })
            .unwrap();
        let stack = |count| Some(ItemStack::new("minecraft:stick", count));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        let builtin = hub.sync_data().builtin.clone();
        let network = [1, 3, 4, 6, 7].map(|count| hub.stack(&stack(count)).unwrap());
        let [one, three, four, six, seven] = &network;
        let prediction = |slot, placed, left| {
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot,
                button: 0,
                input: 0,
                changed: vec![(slot, Some(Hashed::of(placed).unwrap()))],
                carried: Some(Hashed::of(left).unwrap()),
            })
        };
        let set_server_cursor = |game: &mut Game<'_>| {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = stack(7));
        };
        set_server_cursor(game);
        game.lan_guest_packet(1, prediction(0, one, six));
        assert_eq!(
            menu.slot(game, 0),
            Some(Some(stack(1).unwrap().try_into_api().unwrap()))
        );
        assert_eq!(
            menu.carried(game),
            Some(Some(stack(6).unwrap().try_into_api().unwrap()))
        );
        assert!(
            sent(&rx).is_empty(),
            "the server agreed with the ordinary click"
        );

        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot: 0,
                button: 0,
                input: sunrise_client::lan::menu::input::QUICK_MOVE,
                changed: Vec::new(),
                carried: Some(Hashed::of(six).unwrap()),
            }),
        );
        assert_eq!(
            menu.slot(game, 0),
            Some(Some(stack(1).unwrap().try_into_api().unwrap())),
            "shift-click must not move an item to an omitted player inventory"
        );
        assert!(sent(&rx).is_empty());

        set_server_cursor(game);
        game.lan_guest_packet(1, prediction(1, four, three));
        assert_eq!(
            menu.slot(game, 1),
            Some(Some(stack(1).unwrap().try_into_api().unwrap()))
        );
        assert_eq!(
            menu.carried(game),
            Some(Some(stack(7).unwrap().try_into_api().unwrap()))
        );
        let corrections = sent(&rx);
        let corrected_state = corrections
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => {
                    assert_eq!(p.items, [Some(one.clone()), Some(one.clone())]);
                    assert_eq!(p.carried.as_ref(), Some(seven));
                    Some(p.state)
                }
                _ => None,
            })
            .unwrap();

        // The server's copy and cursor now differ only in components from what the
        // packet claims. Matching item IDs and counts is not enough to skip correction.
        let mut named = stack(7).unwrap();
        named.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Filter".into()));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.guests.update(0, |g| {
            g.native_menu[1] = None;
            g.carried = Some(named.clone());
        });
        let named_one = ItemStack {
            count: 1,
            ..named.clone()
        };
        let named_one_net = hub.stack(&Some(named_one.clone()));
        let named_seven_net = hub.stack(&Some(named.clone()));
        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state: corrected_state,
                slot: 1,
                button: 0,
                input: 0,
                changed: vec![(1, Some(Hashed::of(one).unwrap()))],
                carried: Some(Hashed::of(seven).unwrap()),
            }),
        );
        let component_correction = sent(&rx);
        let state = component_correction
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => {
                    assert_eq!(p.items[1], named_one_net);
                    assert_eq!(p.carried, named_seven_net);
                    Some(p.state)
                }
                _ => None,
            })
            .unwrap();

        // No player slots means this quick move does nothing. The prediction has the correct
        // cursor, but claims the slot kept a named stick instead of an ordinary one.
        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot: 0,
                button: 0,
                input: sunrise_client::lan::menu::input::QUICK_MOVE,
                changed: vec![(0, named_one_net.as_ref().map(|s| Hashed::of(s).unwrap()))],
                carried: named_seven_net.as_ref().map(|s| Hashed::of(s).unwrap()),
            }),
        );
        let slot_correction = sent(&rx);
        assert!(matches!(&slot_correction[..], [C::ContainerSetSlot(p)]
            if p.container == window && p.slot == 0 && p.item.as_ref() == Some(one)
        ));

        // Run the real client click path and apply those authoritative packets. Both sides
        // know the declared caps; only the server executes the part's override.
        game.lan = None;
        game.options.execution_role = ExecutionRole::Client;
        let _peer = crate::game::native_client_session::tests::logged_client(game);
        game.remote.as_mut().unwrap().builtin = std::sync::Arc::new(builtin);
        for packet in opening {
            game.handle_packet(packet).unwrap();
        }
        assert_eq!(game.remote_menu_type(), Some(MOD_MENU));
        let [first, second] = [MOD_MENU_SLOT_BASE, MOD_MENU_SLOT_BASE + 1];
        assert_eq!(game.slots(), [first, second]);
        let cursor = || {
            C::SetCursorItem(SetCursorItem {
                item: Some(seven.clone()),
            })
        };
        game.handle_packet(cursor()).unwrap();
        assert!(game.menu_ops(&[(Some(first), 0, Input::Pickup)]));
        assert_eq!(*game.slot_mut(first), stack(1));
        assert_eq!(game.carried, stack(6));
        assert!(game.menu_ops(&[(Some(first), 0, Input::QuickMove)]));
        assert_eq!(*game.slot_mut(first), stack(1));

        game.handle_packet(cursor()).unwrap();
        assert!(game.menu_ops(&[(Some(second), 0, Input::Pickup)]));
        assert_eq!(*game.slot_mut(second), stack(4));
        assert_eq!(game.carried, stack(3));
        for packet in corrections {
            game.handle_packet(packet).unwrap();
        }
        assert_eq!(*game.slot_mut(first), stack(1));
        assert_eq!(*game.slot_mut(second), stack(1));
        assert_eq!(game.carried, stack(7));
        for packet in component_correction {
            game.handle_packet(packet).unwrap();
        }
        assert_eq!(*game.slot_mut(second), Some(named_one));
        assert_eq!(game.carried, Some(named));
    });
}

#[test]
#[ignore = "daily"]
fn closing_discards_owned_slots_unless_the_listener_returns_their_contents() {
    for return_contents in [false, true] {
        with_owned_menu(return_contents, |game, _rx, key| {
            let (window, menu) = open(game, key);
            let stick =
                mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
            let host: &mut dyn ServerHost = game;
            host.transaction(|host, tx| {
                assert_eq!(
                    ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                        host,
                        tx,
                        &ItemResource::of(&stick),
                        3
                    ),
                    3
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: window }));
            game.run_native_menu_events();
            assert_eq!(menu.slot(game, 1), None);
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>(),
                if return_contents {
                    vec![ItemStack::new("minecraft:stick", 3)]
                } else {
                    vec![]
                }
            );
            assert!(game.dropped.is_empty());
            let (_, reopened) = open(game, key);
            assert_eq!(reopened.slot(game, 1), Some(None));
        });
    }
}

#[test]
#[ignore = "daily"]
fn abort_after_replacement_restores_only_committed_contents_to_the_old_close() {
    with_owned_menu(true, |game, _rx, key| {
        let (_, old) = open(game, key);
        let player = game.player(123).unwrap();
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let resource = ItemResource::of(&stick);
        let old_slot = ItemPlace::MenuSlot { menu: old, slot: 1 };
        let host: &mut dyn ServerHost = game;
        host.transaction(|host, tx| {
            assert_eq!(old_slot.insert(host, tx, &resource, 2), 2);
            Ok::<_, ()>(())
        })
        .unwrap();

        let aborted: Result<(), ()> = host.transaction(|host, tx| {
            assert_eq!(old_slot.insert(host, tx, &resource, 1), 1);
            player
                .open_mod_menu(host, {
                    let mut request =
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Replacement"));
                    request.opening_hand = Some(Hand::Off);
                    request.replace = MenuReplace::InPlace;
                    request
                })
                .unwrap()
                .unwrap();
            let new = player.container_menu(host).unwrap();
            let new_slot = ItemPlace::MenuSlot { menu: new, slot: 1 };
            assert_eq!(new_slot.insert(host, tx, &resource, 4), 4);
            Err(())
        });
        assert_eq!(aborted, Err(()));
        let new = game.player_container_menu(player).unwrap();
        assert_eq!(old.slot(game, 1), None);
        assert_eq!(new.slot(game, 1), Some(None), "no leakage between windows");
        game.run_native_menu_events();
        let guest = &game.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(
            guest
                .inventory
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            [ItemStack::new("minecraft:stick", 2)],
            "the old close returned only the committed value"
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_part_closes_the_menu_when_the_opening_stack_leaves_its_hand() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let change_hand = |game: &mut Game<'_>, change: fn(&mut ItemStack)| {
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                change(g.offhand.as_mut().unwrap());
            });
        };
        change_hand(game, |s| s.count = 3);
        click(game, window, 1);
        assert_eq!(
            menu.container_id(game),
            Some(window),
            "count does not affect the comparison"
        );

        // Same item, different components: the opening stack is no longer held.
        change_hand(game, |s| {
            s.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Different".into()));
        });
        click(game, window, 1);
        assert_eq!(menu.container_id(game), None);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerClose(p) if p.container == window
        )));

        let (window, menu) = open(game, key);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.offhand = None);
        click(game, window, 1);
        assert_eq!(menu.container_id(game), None);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerClose(p) if p.container == window
        )));
    });
}

#[test]
#[ignore = "daily"]
fn scoped_owned_clicks_replace_or_continue_each_input_mode() {
    use sunrise_client::lan::menu::input::*;
    with_owned_layout(false, true, |game, rx, key| {
        let stone = |count| Some(ItemStack::new("minecraft:stone", count));
        let dirt = Some(ItemStack::new("minecraft:dirt", 1));
        let player = game.player(123).unwrap();
        let mut cases: Vec<_> = (0..=8)
            .chain([40])
            .map(|button| (SWAP, button, 0))
            .collect();
        cases.extend([
            (CLONE, 2, 0),
            (THROW, 0, 0),
            (QUICK_MOVE, 0, 0),
            (PICKUP_ALL, 0, 0),
            (PICKUP, 0, -999),
        ]);
        for (input, button, slot) in cases {
            for flow in [Flow::Cancel, Flow::Continue] {
                let window = player
                    .open_mod_menu(
                        game,
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Click matrix")),
                    )
                    .unwrap()
                    .unwrap();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.guests.update(0, |g| {
                    g.game_mode = i32::from(input == CLONE);
                    g.inventory.fill(None);
                    g.offhand = (button == 40).then(|| dirt.clone()).flatten();
                    if input == SWAP && button <= 8 {
                        g.inventory[button as usize] = dirt.clone();
                    }
                    g.native_menu[0] = (!matches!(input, PICKUP_ALL | PICKUP))
                        .then(|| stone(1))
                        .flatten();
                    g.native_menu[1] = (input == PICKUP_ALL).then(|| stone(2)).flatten();
                    g.carried = matches!(input, PICKUP_ALL | PICKUP)
                        .then(|| stone(2))
                        .flatten();
                });
                hub.resend_menu(1, window, &game.world);
                sent(&rx);
                let before = game.dropped.len();
                send_scoped_click(game, flow, window, slot, button, input);
                let packets = sent(&rx);
                let g = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                let expected = if matches!(flow, Flow::Cancel) {
                    assert!(
                        packets.iter().any(
                            |p| matches!(p, C::ContainerSetContent(p) if p.container == window)
                        ),
                        "{input}:{button}"
                    );
                    assert_eq!(
                        g.native_menu[0],
                        (!matches!(input, PICKUP_ALL | PICKUP))
                            .then(|| stone(1))
                            .flatten()
                    );
                    assert_eq!(
                        g.native_menu[1],
                        (input == PICKUP_ALL).then(|| stone(2)).flatten()
                    );
                    assert_eq!(
                        g.carried,
                        matches!(input, PICKUP_ALL | PICKUP)
                            .then(|| stone(2))
                            .flatten()
                    );
                    assert_eq!(game.dropped.len(), before);
                    dirt.clone()
                } else {
                    match input {
                        SWAP => assert_eq!(g.native_menu[0], dirt),
                        CLONE => {
                            assert_eq!(g.native_menu[0], stone(1));
                            assert_eq!(g.carried, stone(64));
                        }
                        THROW | PICKUP => {
                            assert_eq!(g.native_menu[0], None);
                            assert_eq!(g.carried, None);
                            assert_eq!(game.dropped.len(), before + 1);
                            assert_eq!(
                                game.dropped[before].stack.count,
                                if input == THROW { 1 } else { 2 }
                            );
                        }
                        QUICK_MOVE => {
                            assert_eq!(g.native_menu[0], None);
                            assert_eq!(g.inventory[8], stone(1));
                        }
                        PICKUP_ALL => {
                            assert_eq!(g.native_menu[1], None);
                            assert_eq!(g.carried, stone(4));
                        }
                        _ => unreachable!(),
                    }
                    stone(1)
                };
                if input == SWAP {
                    if button == 40 {
                        assert_eq!(g.offhand, expected);
                        assert_eq!(g.menu_sent[45], expected);
                        let expected = game.lan.as_mut().unwrap().hub.stack(&expected);
                        assert_eq!(packets.iter().filter(|p| matches!(p, C::SetPlayerInventory(p) if p.slot == 40 && p.item == expected)).count(), 1);
                    } else {
                        assert_eq!(g.inventory[button as usize], expected);
                    }
                }
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_replaced_or_aborted_drag_step_keeps_the_existing_drag() {
    use sunrise_client::lan::menu::input::QUICK_CRAFT;
    with_owned_menu(false, |game, rx, key| {
        let stack = |count| Some(ItemStack::new("minecraft:stone", count));
        let (window, _) = open(game, key);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.carried = stack(6));
        sent(&rx);
        for (slot, step) in [(-999, 0), (0, 1), (1, 1), (-999, 2)] {
            send_scoped_click(game, Flow::Cancel, window, slot, step, QUICK_CRAFT);
            assert!(
                sent(&rx)
                    .iter()
                    .any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window))
            );
            let g = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(g.native_menu, [None, None]);
            assert_eq!(g.carried, stack(6));
            // An enclosing abort must discard this step and its publications as well.
            let seal = game.transaction_open();
            send_scoped_click(game, Flow::Continue, window, slot, step, QUICK_CRAFT);
            assert!(sent(&rx).is_empty());
            game.transaction_close(&seal, false);
            assert!(sent(&rx).is_empty());
            send_scoped_click(game, Flow::Continue, window, slot, step, QUICK_CRAFT);
            sent(&rx);
        }
        let g = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(g.native_menu, [stack(1), stack(3)]);
        assert_eq!(g.carried, stack(2));
    });
}

fn send_scoped_click(
    game: &mut Game<'_>,
    flow: Flow,
    window: i32,
    slot: i16,
    button: i8,
    input: i32,
) {
    let state = game
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(1)
        .unwrap()
        .menu
        .as_ref()
        .unwrap()
        .state;
    SCRIPTED_CLICK.set(Some(flow));
    game.lan_guest_packet(
        1,
        S::ContainerClick(ContainerClick {
            container: window,
            state,
            slot,
            button,
            input,
            changed: Vec::new(),
            carried: None,
        }),
    );
    SCRIPTED_CLICK.set(None);
}

#[test]
#[ignore = "daily"]
fn a_click_refuses_opens_and_closes_only_on_commit() {
    with_owned_menu(true, |game, rx, key| {
        for by_handle in [false, true] {
            let (window, menu) = open(game, key);
            let stored = ItemStack::new("minecraft:stick", 3);
            let carried = ItemStack::new("minecraft:dirt", 2);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.update(0, |g| {
                g.inventory.fill(None);
                g.native_menu[1] = Some(stored.clone());
                g.carried = Some(carried.clone());
            });
            hub.resend_menu(1, window, &game.world);
            sent(&rx);
            CLOSE_NOTICES.set(0);
            WINDOW_REQUEST.set(Some((key, by_handle)));

            let seal = game.transaction_open();
            click(game, window, 0);
            assert_eq!(game.menu(123, window), Some(menu));
            assert!(sent(&rx).is_empty());
            game.transaction_close(&seal, false);
            game.run_native_menu_events();
            assert_eq!(game.menu(123, window), Some(menu));
            let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(guest.native_menu, [None, Some(stored.clone())]);
            assert_eq!(guest.carried, Some(carried.clone()));
            assert!(guest.inventory.iter().all(Option::is_none));
            assert!(sent(&rx).is_empty());
            assert_eq!(CLOSE_NOTICES.get(), 0);

            click(game, window, 0);
            WINDOW_REQUEST.set(None);
            assert!(game.menu(123, window).is_none());
            let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert!(guest.carried.is_none());
            assert!(guest.native_menu.is_empty());
            assert_eq!(
                guest.inventory.iter().flatten().collect::<Vec<_>>(),
                [&carried, &stored],
                "the close returned the cursor and detached owned inputs once"
            );
            assert_eq!(CLOSE_NOTICES.get(), 1);
            let packets = sent(&rx);
            assert_eq!(
                packets
                    .iter()
                    .filter(|p| matches!(p, C::ContainerClose(p) if p.container == window))
                    .count(),
                1
            );
            assert!(!packets.iter().any(|p| matches!(p, C::OpenScreen(_))));
        }
    });
}

#[test]
#[ignore = "daily"]
fn owned_slot_queries_use_the_rule_a_click_will_apply() {
    with_owned_menu(false, |game, _rx, key| {
        let (window, menu) = open(game, key);
        let stored = ItemStack::new("minecraft:stick", 1);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.native_menu[0] = Some(stored.clone()));
        LOCK_FIRST_SLOT.set(true);
        assert_eq!(
            menu.slot_holds(game, 0, &stored.clone().try_into_api().unwrap()),
            Some(0)
        );
        assert_eq!(menu.slot_gives(game, 0), Some(false));
        click(game, window, 0);
        LOCK_FIRST_SLOT.set(false);
        let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(guest.native_menu[0], Some(stored));
        assert!(guest.carried.is_none());
    });
}
