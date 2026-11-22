use super::api_values::TryIntoApi;
use super::native_guest_turn_tests::with_guests;
use mod_api::__private::game::Fields;
use mod_api::{HostTransaction, ItemPlace, ServerHost};
use sunrise_client::{items::ItemStack, lan::playerdata};
use sunrise_core::nbt::Tag;

struct Listener;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Listener {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x1", InventoryPart);
    }
}

struct InventoryPart;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for InventoryPart {
    fn menu_clicked(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::MenuClickedEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.uuid, 12);
        assert_eq!(
            ItemPlace::PlayerSlot {
                player: event.uuid,
                slot: 9
            }
            .with_stack(host, |stack| stack.count())
            .unwrap(),
            10
        );
        assert_eq!(
            ItemPlace::PlayerHand {
                player: event.uuid,
                hand: mod_api::Hand::Main
            }
            .with_stack(host, |stack| stack.count())
            .unwrap(),
            3
        );
        let menu = host
            .player(event.uuid)
            .unwrap()
            .container_menu(host)
            .unwrap();
        let stone = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap(),
        );
        let dirt =
            mod_api::ItemResource::of(&ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap());
        let pearl = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:ender_pearl", 1)
                .try_into_api()
                .unwrap(),
        );
        let take = ItemPlace::MenuSlot { menu, slot: 9 };
        let give = ItemPlace::MenuSlot { menu, slot: 10 };
        let partial = ItemPlace::MenuSlot { menu, slot: 11 };
        host.transaction(|host, tx| {
            assert_eq!(take.extract(host, tx, &stone, 4), 4);
            assert_eq!(give.insert(host, tx, &dirt, 5), 5);
            assert_eq!(partial.insert(host, tx, &pearl, 10), 4);
            Ok::<_, ()>(())
        })
        .unwrap();
        mod_api::Flow::Cancel
    }
}

fn click(game: &mut super::Game<'_>, window: i32) {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    game.lan_guest_packet(
        12,
        Serverbound::ContainerClick(ContainerClick {
            container: window,
            state: 0,
            slot: 9,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        }),
    );
}

fn exercise(reopen: bool) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("inventory", Box::new(Listener))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            let root = tempfile::tempdir().unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                    g.inventory[9] = Some(ItemStack::new("minecraft:stone", 10));
                    g.inventory[11] = Some(ItemStack::new("minecraft:ender_pearl", 12));
                    if reopen {
                        let mut level = sunrise_client::save::Level {
                            player_uuid: Some(playerdata::uuid_ints(g.uuid)),
                            ..Default::default()
                        };
                        playerdata::store(g, &mut level, None);
                        level.write(root.path(), &Default::default()).unwrap();
                        playerdata::save_now(g, root.path(), &Default::default()).unwrap();
                        g.inventory.fill(None);
                        let reopened = sunrise_client::save::Level::read(root.path()).unwrap();
                        assert!(playerdata::load_owner(g, root.path(), Some(&reopened)));
                        assert_eq!(g.uuid, 12);
                    }
                })
                .unwrap();
            let window = game
                .lan
                .as_mut()
                .unwrap()
                .hub
                .open_native_menu(12, 1, Tag::String("inventory".into()), vec![None; 9])
                .unwrap();
            let player = game.player(12).unwrap();
            let menu = player.container_menu(game).unwrap();
            let stone = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap(),
            );
            let dirt = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap(),
            );
            let take = ItemPlace::MenuSlot { menu, slot: 9 };
            let give = ItemPlace::MenuSlot { menu, slot: 10 };
            let partial = ItemPlace::MenuSlot { menu, slot: 11 };
            // The hook runs without a guest swapped in; use that same Host state.
            assert!(game.lan_acting.is_none());
            let hand = ItemPlace::PlayerHand {
                player: 12,
                hand: mod_api::Hand::Main,
            };
            assert_eq!(hand.with_stack(game, |stack| stack.count()).unwrap(), 3);
            let direct = ItemPlace::PlayerSlot {
                player: 12,
                slot: 9,
            };
            assert_eq!(direct.with_stack(game, |stack| stack.count()).unwrap(), 10);
            let foreign_player = mod_api::__private::game::player_ref(player.entity(), 13);
            let foreign = mod_api::__private::game::menu_ref(
                foreign_player,
                window,
                menu.fields().generation,
            );
            let foreign = ItemPlace::MenuSlot {
                menu: foreign,
                slot: 9,
            };
            (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    assert_eq!(foreign.extract(host, tx, &stone, 4), 0);
                    assert_eq!(foreign.insert(host, tx, &dirt, 5), 0);
                    Ok::<_, ()>(())
                })
                .unwrap();
            click(game, window);
            assert_eq!(take.with_stack(game, |stack| stack.count()).unwrap(), 6);
            assert_eq!(give.with_stack(game, |stack| stack.count()).unwrap(), 5);
            assert_eq!(partial.with_stack(game, |stack| stack.count()).unwrap(), 16);
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(give.insert(host, tx, &dirt, 3), 3);
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(give.with_stack(game, |stack| stack.count()).unwrap(), 5);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .open_native_menu(12, 1, Tag::String("replacement".into()), vec![None; 9])
                .unwrap();
            (game as &mut dyn ServerHost)
                .transaction(|host, tx| {
                    assert_eq!(take.extract(host, tx, &stone, 4), 0);
                    assert_eq!(give.insert(host, tx, &dirt, 5), 0);
                    Ok::<_, ()>(())
                })
                .unwrap();
            assert_ne!(
                player.container_menu(game).unwrap().container_id(game),
                Some(window)
            );
            assert_eq!(direct.with_stack(game, |stack| stack.count()).unwrap(), 6);
        });
    });
}

#[test]
#[ignore = "daily"]
fn menu_player_inventory_transactions() {
    exercise(false);
}

#[test]
#[ignore = "daily"]
fn menu_player_inventory_after_world_save_reload() {
    exercise(true);
}

/// #2671: single player, where the world's owner is also a guest of the server thread.
mod owner {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// Each extract and insert the hook made, then the amounts read after.
        static SEEN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    struct Owner;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Owner {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x5", OwnerPart);
        }
    }

    struct OwnerPart;
    #[mod_api::sunrise_part]
    impl mod_api::MenuPart for OwnerPart {
        fn menu_clicked(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::MenuClickedEvent<'_>,
        ) -> mod_api::Flow {
            let menu = host.menu(event.uuid, event.window).unwrap();
            let diamond = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:diamond", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let player_slot = ItemPlace::PlayerSlot {
                player: event.uuid,
                slot: 0,
            };
            let menu_slot = ItemPlace::MenuSlot { menu, slot: 72 };
            let own_slot = ItemPlace::MenuSlot { menu, slot: 9 };
            let cursor = menu.cursor();
            let mut seen = Vec::new();
            host.transaction(|host, tx| {
                seen.push(player_slot.extract(host, tx, &diamond, 1));
                seen.push(menu_slot.extract(host, tx, &diamond, 1));
                seen.push(own_slot.insert(host, tx, &diamond, 1));
                seen.push(cursor.insert(host, tx, &diamond, 1));
                seen.push(player_slot.amount(host));
                seen.push(menu_slot.amount(host));
                Ok::<_, ()>(())
            })
            .unwrap();
            SEEN.with(|s| *s.borrow_mut() = seen);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn menu_clicked_moves_the_owners_hotbar_slot_and_cursor() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("owner", Box::new(Owner))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            with_guests(|game| {
                game.level.player_uuid = Some(playerdata::uuid_ints(12));
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| {
                        g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 2));
                    })
                    .unwrap();
                let window = game
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .open_native_menu(12, 5, Tag::String("owner".into()), vec![None; 45])
                    .unwrap();
                click(game, window);
                // Each extract takes 1, the own slot and the cursor take 1; then 0 left
                // in the first slot's pair of reads (2 - 1 - 1).
                assert_eq!(SEEN.with(|s| s.borrow().clone()), [1, 1, 1, 1, 0, 0]);
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert!(guest.inventory[0].is_none());
                assert_eq!(guest.carried.as_ref().map(|s| s.count), Some(1));
                assert_eq!(guest.native_menu[9].as_ref().map(|s| s.count), Some(1));
            });
        });
    }

    /// A hook running in another level than the owner's reaches the owner's own inventory,
    /// never the `Game`'s player: a full slot takes nothing, nothing drops, and the abort puts
    /// the giver's diamond back.
    #[test]
    #[ignore = "daily"]
    fn a_hook_in_another_level_leaves_the_owners_inventory_and_drops_alone() {
        with_guests(|game| {
            game.level.player_uuid = Some(playerdata::uuid_ints(12));
            assert_ne!(game.dim.id(), "minecraft:the_nether");
            // The `Game`'s own player, living and with room: not the owner's, who is a guest.
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(8.5, 64.0, 8.5),
                )
                .unwrap(),
            );
            game.inventory.fill(None);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.dimension = "minecraft:the_nether".to_owned();
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)));
                })
                .unwrap();
            let diamond = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:diamond", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let slot = ItemPlace::PlayerSlot {
                player: 12,
                slot: 0,
            };
            // A diamond of another player moves to the owner, as a mod's transfer does: what
            // the owner can't take goes back, here by aborting the transaction.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 1))
                })
                .unwrap();
            let source = ItemPlace::PlayerSlot {
                player: 13,
                slot: 0,
            };
            let dropped = game.dropped.len();
            let moved: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                let taken = source.extract(host, tx, &diamond, 1);
                assert_eq!(taken, 1);
                assert_eq!(slot.insert(host, tx, &diamond, taken), 0);
                Err(())
            });
            assert!(moved.is_err());
            assert!(game.inventory.iter().all(Option::is_none));
            let giver = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!(giver.inventory[0].as_ref().map(|s| s.count), Some(1));
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert!(guest.inventory.iter().all(|s| {
                s.as_ref()
                    .is_some_and(|s| s.count == 64 && &*s.item == "minecraft:stone")
            }));
            assert_eq!(game.dropped.len(), dropped);
        });
    }

    /// A transaction that clears a stack of a guest in the Nether and aborts restores it, and
    /// a stack given to them with a full inventory comes back to the mod, dropped nowhere.
    #[test]
    #[ignore = "daily"]
    fn a_non_owner_guest_in_another_level_keeps_their_stack_through_an_abort() {
        use mod_api::prelude::*;
        with_guests(|game| {
            game.level.player_uuid = Some(playerdata::uuid_ints(12));
            assert_ne!(game.dim.id(), "minecraft:the_nether");
            // A level is open while a player is in it.
            game.enter_level(super::super::Dim::Nether).unwrap();
            game.enter_level(super::super::Dim::Overworld).unwrap();
            let stack = |game: &super::super::Game<'_>, snbt: &str| {
                let host: &dyn mod_api::Host = game;
                host.item_from_snbt(snbt).unwrap()
            };
            let stone = stack(game, r#"{id:"minecraft:stone",count:64}"#);
            let diamond = stack(game, r#"{id:"minecraft:diamond",count:1}"#);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.dimension = "minecraft:the_nether".to_owned();
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 64));
                })
                .unwrap();
            let player = game.players().server_players()[1];
            assert_eq!(player.uuid(), 13);
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
                player.set_inventory_slot(host, 0, None).unwrap();
                Err(())
            });
            assert!(aborted.is_err());
            player
                .with_inventory(game, |view| {
                    let mut slots = view.iter();
                    let (slot, stack) = slots.next().unwrap();
                    assert_eq!(slot, 0);
                    assert_eq!(stack.project(), Some(stone));
                    assert!(slots.next().is_none());
                })
                .unwrap();

            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)));
                })
                .unwrap();
            let dropped = game.dropped.len();
            let given = (game as &mut dyn ServerHost)
                .transaction(|host, _| Ok::<_, ()>(player.give_stack(host, diamond.clone())));
            assert_eq!(given, Ok(Ok(Some(diamond))));
            assert_eq!(game.dropped.len(), dropped);
            game.enter_level(super::super::Dim::Nether).unwrap();
            assert_eq!(game.dropped.len(), 0);
        });
    }
}
