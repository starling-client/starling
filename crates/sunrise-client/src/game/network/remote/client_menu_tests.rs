use super::*;

#[test]
#[ignore = "daily"]
fn inventory_repair_does_not_replace_an_open_menus_predicted_cursor() {
    use crate::game::native_entity_refs::tests::with_game;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        item::Stack,
        packets::play::{self, Clientbound as C},
    };

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        let table = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting")
            .unwrap();
        let stone = game.item_id("minecraft:stone");
        let stick = game.item_id("minecraft:stick");
        game.handle_packet(C::OpenScreen(play::OpenScreen {
            container: 7,
            menu: table,
            title: Tag::String("Crafting".into()),
        }))
        .unwrap();
        let mut items = vec![None; 46];
        items[2] = Some(Stack::new(stone, 4));
        game.handle_packet(C::ContainerSetContent(play::ContainerSetContent {
            container: 7,
            state: 12,
            items,
            carried: None,
        }))
        .unwrap();
        let menu = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        let pickup = mod_api::MenuClick::Pickup {
            slot: Some(2),
            button: mod_api::MouseButton::Left,
        };
        assert!(menu.click(game, std::slice::from_ref(&pickup)));
        assert_eq!(menu.slot(game, 2), Some(None));

        // An in-flight inventory repair arrives before the server can acknowledge the pickup.
        let mut items = vec![None; 46];
        items[2] = Some(Stack::new(stick, 2));
        game.handle_packet(C::ContainerSetContent(play::ContainerSetContent {
            container: 0,
            state: 45,
            items,
            carried: Some(Stack::new(stick, 1)),
        }))
        .unwrap();
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 4);
        assert_eq!(menu.slot(game, 2), Some(None));
        assert!(menu.click(game, &[pickup]));
        assert_eq!(menu.slot(game, 2).unwrap().unwrap().count(), 4);
        assert_eq!(menu.carried(game), Some(None));
        for count in [Some(4), None] {
            let click = (0..32)
                .find_map(|_| {
                    let (id, body) = reader.recv().unwrap();
                    match play::Serverbound::decode(id, &body).unwrap() {
                        play::Serverbound::ContainerClick(click) => Some(click),
                        _ => None,
                    }
                })
                .unwrap();
            assert_eq!((click.container, click.state, click.slot), (7, 12, 2));
            assert_eq!(click.carried.map(|hash| hash.count), count);
        }
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 7 }))
            .unwrap();
        assert_eq!(
            game.carried.as_ref().unwrap().item.as_ref(),
            "minecraft:stick"
        );
        assert_eq!(game.remote.as_ref().unwrap().state_id, 45);
        let inventory = game
            .player_container_menu(game.local_player().unwrap())
            .unwrap();
        assert_eq!(inventory.slot(game, 2).unwrap().unwrap().count(), 2);
    });
}

#[test]
#[ignore = "daily"]
fn client_menu_reads_follow_packets_and_lifetimes() {
    use crate::game::native_entity_refs::tests::with_game;
    use crate::game::remote::RemoteDimensionEntry;
    use crate::game::{Screen, execution_role::ExecutionRole};
    use sunrise_client::level_loading::Reason;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        buf::Reader,
        connection::Pipe,
        item::{Component, Stack, component},
        packets::play::{self, Clientbound as C},
    };

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        let spawn = spawn_menu_test(game);
        let menu = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(menu), Some(46));
        assert_eq!(game.menu_slot(menu, 46), None);
        assert_eq!(game.menu_slot(menu, 36), Some(None));
        let stone = Some(Stack::new(game.item_id("minecraft:stone"), 3));
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_slot(menu, 36).unwrap().unwrap().count, 3);
        use mod_api::{DragKind, MenuClick, MouseButton, SwapWith};
        let pickup = |slot| MenuClick::Pickup {
            slot: Some(slot),
            button: MouseButton::Left,
        };
        for invalid in [
            pickup(46),
            pickup(usize::MAX),
            MenuClick::Swap {
                slot: 36,
                with: SwapWith::Hotbar(9),
            },
            MenuClick::Clone { slot: 36 },
            MenuClick::Drag {
                kind: DragKind::Clone,
                slots: vec![36],
            },
            MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![36, usize::MAX],
            },
        ] {
            assert!(!menu.click(game, &[invalid]));
            assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
            assert_eq!(menu.carried(game), Some(None));
        }
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(!menu.click(game, &[pickup(36)]));
        game.options.execution_role = ExecutionRole::Client;
        let screen = game.screen;
        game.screen = Screen::Creative;
        assert!(!menu.click(game, &[pickup(36)]));
        game.screen = screen;
        game.remote.as_mut().unwrap().state_id = 17;

        // MapPostProcessing.SCALE has a stream codec but no persistent codec to hash.
        let mut unhashable = Stack::new(game.item_id("minecraft:filled_map"), 1);
        unhashable
            .added
            .push((component::MAP_POST_PROCESSING, Component::Int(1)));
        let inventory = |slot, item| C::SetPlayerInventory(play::SetPlayerInventory { slot, item });
        let packet = inventory(0, Some(unhashable)).encode();
        let mut packet = Reader::new(&packet);
        game.handle_packet(C::decode(packet.varint().unwrap(), packet.rest()).unwrap())
            .unwrap();
        game.handle_packet(inventory(1, stone.clone())).unwrap();
        let before = (
            game.inventory[0].clone(),
            game.inventory[1].clone(),
            game.carried.clone(),
        );
        assert!(before.0.as_ref().unwrap().wire_patch().is_some());
        assert!(game.hashed_stack(&before.0).is_err());
        // The second pickup would put the unhashable map on the cursor.
        // Submit both clicks in order, without committing or sending any prediction.
        assert!(menu.click(game, &[pickup(37), pickup(36)]));
        assert_eq!(
            (&game.inventory[0], &game.inventory[1], &game.carried),
            (&before.0, &before.1, &before.2)
        );
        for slot in [37, 36] {
            let (id, body) = reader.recv().unwrap();
            let play::Serverbound::ContainerClick(click) =
                play::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected an unpredicted pickup for slot {slot}");
            };
            assert_eq!(
                (
                    click.container,
                    click.state,
                    click.slot,
                    click.button,
                    click.input
                ),
                (0, 17, slot, 0, 0)
            );
            assert!(click.changed.is_empty());
            assert!(click.carried.is_none());
        }
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        assert!(reader.recv().unwrap_err().is_timed_out());
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        game.handle_packet(inventory(0, stone.clone())).unwrap();
        game.handle_packet(inventory(1, None)).unwrap();

        assert!(menu.click(game, &[pickup(36)]));
        let (id, body) = reader.recv().unwrap();
        let play::Serverbound::ContainerClick(sent) = play::Serverbound::decode(id, &body).unwrap()
        else {
            panic!("expected the first valid click, not a refused request");
        };
        assert_eq!(
            (
                sent.container,
                sent.state,
                sent.slot,
                sent.button,
                sent.input
            ),
            (0, 17, 36, 0, 0)
        );
        assert_eq!(sent.changed.len(), 1);
        assert_eq!(sent.changed[0].0, 36);
        assert!(sent.changed[0].1.is_none());
        assert_eq!(sent.carried.unwrap().count, 3);
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
        assert!(!menu.click(game, &[pickup(36), pickup(usize::MAX),]));
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.carried(game).unwrap().unwrap().count(), 3);
        assert!(menu.click(
            game,
            &[MenuClick::Drag {
                kind: DragKind::Split,
                slots: vec![37]
            },]
        ));
        assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 3);
        assert_eq!(menu.carried(game), Some(None));
        assert!(menu.click(game, &[pickup(37)]));
        assert!(menu.click(game, &[pickup(36)]));
        assert_eq!(menu.carried(game), Some(None));
        assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 3);
        game.handle_packet(C::SetPlayerInventory(play::SetPlayerInventory {
            slot: 0,
            item: Some(Stack::new(game.item_id("minecraft:stone"), 2)),
        }))
        .unwrap();
        assert_eq!(menu.slot(game, 36).unwrap().unwrap().count(), 2);
        assert!(menu.click(game, &[MenuClick::QuickMove { slot: 36 }]));
        assert_eq!(menu.slot(game, 36), Some(None));
        assert_eq!(menu.slot(game, 9).unwrap().unwrap().count(), 2);
        assert!(menu.click(
            game,
            &[MenuClick::Swap {
                slot: 9,
                with: SwapWith::Hotbar(1)
            }]
        ));
        assert_eq!(menu.slot(game, 9), Some(None));
        assert_eq!(menu.slot(game, 37).unwrap().unwrap().count(), 2);
        assert!(menu.click(
            game,
            &[MenuClick::Throw {
                slot: 37,
                whole_stack: true
            }]
        ));
        assert_eq!(menu.slot(game, 37), Some(None));
        game.handle_packet(C::SetCursorItem(play::SetCursorItem {
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_carried(menu).unwrap().unwrap().count, 3);
        let crafting_id = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:crafting")
            .unwrap();
        let open = || {
            C::OpenScreen(play::OpenScreen {
                container: 7,
                menu: crafting_id,
                title: Tag::String("Crafting".into()),
            })
        };
        game.handle_packet(open()).unwrap();
        assert_eq!(game.menu_slot_count(menu), None);
        assert!(!menu.click(game, &[pickup(36)]));
        let crafting = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(crafting), Some(46));
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 1,
            item: stone.clone(),
        }))
        .unwrap();
        assert_eq!(game.menu_slot(crafting, 1).unwrap().unwrap().count, 3);
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 0,
            item: stone,
        }))
        .unwrap();
        assert_eq!(game.menu_slot(crafting, 0).unwrap().unwrap().count, 3);
        game.handle_packet(open()).unwrap();
        assert_eq!(game.menu_slot_count(crafting), None);
        assert!(!crafting.click(game, &[pickup(1)]));
        let crafting = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.menu_slot_count(crafting), None);
        assert!(!crafting.click(game, &[pickup(1)]));
        let inventory = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_ne!(inventory, menu);
        let chest_id = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:generic_9x3")
            .unwrap();
        game.handle_packet(C::OpenScreen(play::OpenScreen {
            container: 7,
            menu: chest_id,
            title: Tag::String("Chest".into()),
        }))
        .unwrap();
        let chest = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(chest), Some(63));
        let stone = Some(Stack::new(game.item_id("minecraft:stone"), 4));
        game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
            container: 7,
            state: 0,
            slot: 2,
            item: stone,
        }))
        .unwrap();
        assert_eq!(game.menu_slot(chest, 2).unwrap().unwrap().count, 4);
        assert_eq!(game.menu_slot(chest, 3), Some(None));
        game.handle_packet(C::SetCursorItem(play::SetCursorItem { item: None }))
            .unwrap();
        assert!(chest.click(game, &[pickup(2)]));
        assert_eq!(chest.slot(game, 2), Some(None));
        assert_eq!(chest.carried(game).unwrap().unwrap().count(), 4);
        assert!(chest.click(game, &[pickup(2)]));
        assert_eq!(chest.slot(game, 2).unwrap().unwrap().count(), 4);
        assert_eq!(chest.carried(game), Some(None));
        for expected in [
            (0, 17, -999, 0, 5),
            (0, 17, 37, 1, 5),
            (0, 17, -999, 2, 5),
            (0, 17, 37, 0, 0),
            (0, 17, 36, 0, 0),
            (0, 17, 36, 0, 1),
            (0, 17, 9, 1, 2),
            (0, 17, 37, 1, 4),
            (7, 0, 2, 0, 0),
            (7, 0, 2, 0, 0),
        ] {
            let sent = (0..32)
                .find_map(|_| {
                    let (id, body) = reader.recv().unwrap();
                    match play::Serverbound::decode(id, &body).unwrap() {
                        play::Serverbound::ContainerClick(sent) => Some(sent),
                        _ => None,
                    }
                })
                .expect("expected click packet");
            assert_eq!(
                (
                    sent.container,
                    sent.state,
                    sent.slot,
                    sent.button,
                    sent.input
                ),
                expected
            );
            if expected == (0, 17, -999, 2, 5) {
                assert!(sent.carried.is_none());
                assert_eq!(sent.changed.len(), 1);
                assert_eq!(sent.changed[0].0, 37);
                assert_eq!(sent.changed[0].1.as_ref().unwrap().count, 3);
            }
        }
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        assert!(reader.recv().unwrap_err().is_timed_out());
        game.remote
            .as_mut()
            .unwrap()
            .menu
            .set(Some((7, "minecraft:unknown".into())));
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        assert_eq!(game.menu_slot_count(inventory), None);
        assert_eq!(game.menu_slot_count(chest), None);
        game.remote.as_mut().unwrap().menu.set(None);
        let before_replacement = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        let mut other = spawn;
        other.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&other, RemoteDimensionEntry::Respawn, Reason::Other);
        assert_eq!(game.menu_slot_count(before_replacement), None);
        assert!(!before_replacement.click(game, &[pickup(36)]));
        let before_logout = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        game.client_logging_out();
        assert_eq!(game.menu_slot_count(before_logout), None);
        assert!(!before_logout.click(game, &[pickup(36)]));
        assert!(
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .is_none()
        );
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "MenuReadTest").unwrap();
        game.enter_remote_dimension(&other, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        let reconnected = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.menu_slot_count(before_logout), None);
        assert_eq!(game.menu_slot_count(reconnected), Some(46));
        assert_ne!(reconnected, before_logout);
    });
}

/// `MenuRef::close` and `click_button` go through the screen's own close
/// and the button packet, and refuse a menu that is not open or stale.
#[test]
#[ignore = "daily"]
fn client_menu_close_and_button_click_reach_the_server() {
    use crate::game::native_entity_refs::tests::with_game;
    use crate::game::{Screen, execution_role::ExecutionRole};
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    with_game(|game| {
        let (mut reader, _writer) = connect_menu_test(game);
        spawn_menu_test(game);
        let sent = |reader: &mut sunrise_protocol::connection::ConnReader| {
            (0..32).find_map(|_| {
                let (id, body) = reader.recv().ok()?;
                match play::Serverbound::decode(id, &body).unwrap() {
                    p @ (play::Serverbound::ContainerClose(_)
                    | play::Serverbound::ContainerButtonClick(_)) => Some(p),
                    _ => None,
                }
            })
        };
        // The inventory menu while playing: no screen to close, no buttons.
        let inventory = game
            .local_player()
            .and_then(|player| game.player_container_menu(player))
            .unwrap();
        assert_eq!(game.screen, Screen::Playing);
        assert!(!inventory.close(game));
        assert!(!inventory.click_button(game, 0));
        let loom = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:loom")
            .unwrap();
        let open = |game: &mut super::Game<'_>| {
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 5,
                menu: loom,
                title: Tag::String("Loom".into()),
            }))
            .unwrap();
            game.local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap()
        };
        let menu = open(game);
        assert_eq!(game.screen, Screen::Inventory);
        // The old inventory handle is stale now that the loom replaced it.
        assert!(!inventory.close(game));
        assert!(!inventory.click_button(game, 0));
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(!menu.click_button(game, 3));
        assert!(!menu.close(game));
        game.options.execution_role = ExecutionRole::Client;
        assert_eq!(game.screen, Screen::Inventory);
        assert!(menu.click_button(game, 3));
        let Some(play::Serverbound::ContainerButtonClick(click)) = sent(&mut reader) else {
            panic!("expected the button click, not a refused request");
        };
        assert_eq!((click.container, click.button), (5, 3));
        assert!(menu.close(game));
        assert_eq!(game.screen, Screen::Playing);
        let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
            panic!("expected the close");
        };
        assert_eq!(close.container, 5);
        assert!(game.remote.as_ref().unwrap().menu.as_ref().is_none());
        // Closed: the handle is stale and nothing more is sent.
        assert!(!menu.close(game));
        assert!(!menu.click_button(game, 3));
        // A server-side close leaves a reopened menu's old handle stale too.
        let first = open(game);
        game.handle_packet(C::ContainerClose(play::ContainerClose { container: 5 }))
            .unwrap();
        let second = open(game);
        assert!(!first.close(game));
        assert!(!first.click_button(game, 3));
        assert!(second.close(game));
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        let Some(play::Serverbound::ContainerClose(close)) = sent(&mut reader) else {
            panic!("expected the second close");
        };
        assert_eq!(close.container, 5);
        assert!(sent(&mut reader).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn retained_effects_rebind_on_reconfiguration_and_refusals_do_not_block_clicks() {
    use crate::game::native_entity_refs::tests::with_game;
    use std::sync::Arc;
    use sunrise_core::{nbt::Tag, vanilla_bytes::Writer};
    use sunrise_protocol::{
        buf::{Codec, Reader},
        item::{Component, Hashed, Slot, Stack, component, consume_effects as stream},
        packets::{
            configuration::{RegistryData, RegistryEntry},
            play,
        },
        registry::{Dynamic, SyncedNames},
    };
    use sunrise_world::component_value::consume_effects::{self, Type};

    consume_effects::with_test_registry(|| {
        consume_effects::install(
            ["reconfigure:first", "reconfigure:second"]
                .into_iter()
                .map(|name| {
                    (
                        Arc::from(name),
                        Type {
                            schema: 17,
                            load: Box::new(|_| {
                                unreachable!("a retained network value must not reload NBT")
                            }),
                            read: Box::new(|bytes, project| {
                                let byte = *bytes.first().ok_or("missing charge")?;
                                // Emulate the mod save adapter refusing a value but consuming its Wire byte.
                                Ok((
                                    1,
                                    (project && byte != 0xff).then(|| {
                                        [("charge".into(), Tag::Int(i32::from(byte)))].into()
                                    }),
                                ))
                            }),
                        },
                    )
                })
                .collect(),
        )
        .unwrap();
        let configure = |swap: bool| {
            let mut entries: Vec<_> = stream::configuration()
                .map(|(id, schema)| RegistryEntry {
                    id: id.into(),
                    data: Some(Tag::Long(schema as i64)),
                })
                .collect();
            if swap {
                entries.swap(5, 6);
            }
            let mut registry = Dynamic::default();
            registry
                .add_consume_effects(RegistryData {
                    registry: stream::REGISTRY.into(),
                    entries,
                })
                .unwrap();
            Arc::new(registry)
        };
        with_game(|game| {
            let (mut reader, _writer) = connect_menu_test(game);
            spawn_menu_test(game);
            let item = game.item_id("minecraft:stone");
            let protection = |count, bytes| {
                let mut stack = Stack::new(item, count);
                stack
                    .added
                    .push((component::DEATH_PROTECTION, Component::Raw(bytes)));
                stack
            };
            let receive = |stack, mapping: &Dynamic| {
                let mut wire = Writer::new();
                Slot::write(&Some(stack), &mut wire);
                Slot::read(&mut Reader::new(&wire.data).with_names(mapping)).unwrap()
            };
            let old = configure(false);
            game.set_registries(old.clone());
            let retained = game.to_stack(&receive(protection(2, vec![1, 5, 7]), &old));
            assert!(retained.as_ref().unwrap().wire_patch().is_some());
            let new = configure(true);
            game.set_registries(new.clone());
            let sent = game.to_network(&retained).unwrap();
            assert_eq!(
                sent.component(component::DEATH_PROTECTION),
                Some(&Component::Raw(vec![1, 6, 7]))
            );
            let expected_hash = |stack, game: &crate::game::Game<'_>| {
                let names = SyncedNames {
                    builtin: &game.remote.as_ref().unwrap().builtin,
                    dynamic: &new,
                };
                Hashed::create(&stack, &names).unwrap()
            };
            assert_eq!(
                game.hashed_stack(&retained).unwrap(),
                Some(expected_hash(protection(2, vec![1, 6, 7]), game))
            );
            assert_ne!(
                game.hashed_stack(&retained).unwrap(),
                Some(expected_hash(protection(2, vec![1, 5, 7]), game))
            );

            // The refused custom effect precedes a valid built-in. It must not poison
            // structural framing or either side's hash of the ordinary survivor.
            let received = receive(protection(2, vec![2, 6, 0xff, 2]), &new);
            let projected = expected_hash(protection(2, vec![1, 2]), game);
            assert_eq!(expected_hash(received.clone().unwrap(), game), projected);
            game.carried = game.to_stack(&received);
            assert_eq!(game.hashed_stack(&game.carried).unwrap(), Some(projected));
            let menu = game
                .local_player()
                .and_then(|player| game.player_container_menu(player))
                .unwrap();
            assert!(menu.click(
                game,
                &[mod_api::MenuClick::Pickup {
                    slot: Some(36),
                    button: mod_api::MouseButton::Left
                }]
            ));
            let (id, body) = reader.recv().unwrap();
            let play::Serverbound::ContainerClick(click) =
                play::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected an ordinary click with the admitted projection");
            };
            assert_eq!(click.slot, 36);
            assert!(click.carried.is_none());
            assert_eq!(
                click.changed,
                vec![(36, Some(expected_hash(protection(2, vec![1, 2]), game)))]
            );
        });
    });
}
