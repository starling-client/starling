use super::*;
use mod_api::__private::game::Fields;
use mod_api::{Hand, HostTransaction, ItemPlace, ItemResource, MenuSlotPlace, ServerHost};
use sunrise_client::{
    containers::{BlockEntity, ModBlockEntity, ModMenuType},
    crafting::{Ingredient, Placement},
    lan::menu::{Menu as Open, Slots},
    recipe_book::{
        settings_connection::Connection, settings_sync::Authority, slot_layout::CheckedSlotLayout,
    },
};
/// A guest with a three-slot mod menu open on a block, and the selection its client would send.
fn open_mod_menu(game: &mut Game<'_>) -> ((i32, i32, i32), MenuSnapshot, sync::Selection) {
    let pos = (3, 64, 5);
    let kind = Box::leak(Box::new(ModMenuType {
        id: "review:menu",
        owner: "review",
        slots: 3,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: vec![2],
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }));
    let mut block = ModBlockEntity::new("review:block".into(), Default::default());
    block.items = vec![
        Some(sunrise_client::items::ItemStack::new("minecraft:dirt", 1)),
        None,
        None,
    ];
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(block)));
    game.world.block_entities.set_mod_containers(
        [(
            game.world.block(pos.0, pos.1, pos.2).0,
            std::sync::Arc::new(sunrise_client::containers::ModContainer {
                slots: 3,
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
    game.world.modified.clear();
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.menu_generation = 1;
        g.menu = Some(Open {
            window: 1,
            kind: 0,
            registry_id: "review:menu".into(),
            pos: Some(pos),
            slots: Slots::Block(pos),
            size: 3,
            state: 0,
            sent: vec![],
            drag: None,
            sent_data: vec![],
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),

            mod_type: Some(kind),
            craft: None,
            links: vec![],
            asks_still_valid: false,
            opening_item: None,
        });
    });
    let before = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
    let catalog = sync::CatalogIdentity {
        epoch: 1,
        number: 1,
        window: 1,
        nonce: 1,
        instance: 1,
        generation: 1,
        menu: "review:menu".to_owned().try_into().unwrap(),
        book: "review:book".to_owned().try_into().unwrap(),
    };
    let selection = sync::Selection {
        catalog: catalog.clone(),
        number: 2,
        display: sync::DisplayId {
            recipe: "review:recipe".to_owned().try_into().unwrap(),
            source: 0,
        },
    };
    let mut server = super::super::native_book_placement::Server::new(1);
    server.peer.hello(1, sync::schema()).unwrap();
    server.peer.publish(catalog).unwrap();
    game.native_net.placement.servers.insert(12, server);
    let mut connection = Connection::default();
    connection.authority = Some(Authority::negotiate(
        1,
        vec!["review:book".into()],
        &["review:book".into()].into_iter().collect(),
    ));
    game.native_net.book_connections.insert(12, connection);
    (pos, before, selection)
}

#[test]
#[ignore = "daily"]
fn actual_game_stage_refuses_nested_frame_and_mutated_actor() {
    super::super::native_guest_turn_tests::with_guests(|game| {
        let (pos, before, selection) = open_mod_menu(game);
        let layout = CheckedSlotLayout::new(
            mod_api::RecipeBookLayout::Furnace {
                input: 0,
                fuel: 1,
                result: 2,
            },
            3,
        )
        .unwrap();
        let plan = MenuPlacement::new(
            "review:menu".into(),
            Placement::of(
                vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                None,
            ),
            &layout,
        )
        .unwrap();
        let root = game.transaction_open();
        assert!(game.mod_book_placement_context(&before, &selection));
        let child = game.transaction_open();
        assert_eq!(
            game.stage_mod_book_place(&before, &selection, &plan, false),
            Err(sync::Refusal::Changed)
        );
        game.transaction_close(&child, false);
        assert!(game.mod_book_placement_context(&before, &selection));
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.health = 1.0);
        assert_eq!(
            game.stage_mod_book_place(&before, &selection, &plan, false),
            Err(sync::Refusal::Changed)
        );
        assert_eq!(
            game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .count,
            1
        );
        assert!(game.world.modified.is_empty());
        game.transaction_close(&root, false);
    });
}
#[test]
#[ignore = "daily"]
fn a_mod_layout_without_a_listening_part_refuses_and_changes_nothing() {
    super::super::native_guest_turn_tests::with_guests(|game| {
        let (pos, before, selection) = open_mod_menu(game);
        let plan = MenuPlacement::for_mod(
            "review:menu".into(),
            Placement::of(
                vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                None,
            ),
        )
        .unwrap();
        let root = game.transaction_open();
        assert_eq!(
            game.stage_mod_book_place(&before, &selection, &plan, false),
            Err(sync::Refusal::Unsupported)
        );
        assert_eq!(
            game.world.block_entities.get(&pos).unwrap().slots()[0]
                .as_ref()
                .unwrap()
                .count,
            1
        );
        assert!(game.world.modified.is_empty());
        game.transaction_close(&root, false);
    });
}
#[test]
#[ignore = "daily"]
fn place_recipe_in_grid_places_once_per_click() {
    super::super::native_guest_turn_tests::with_guests(|game| {
        let (_, before, selection) = open_mod_menu(game);
        let plan = MenuPlacement::for_mod(
            "review:menu".into(),
            Placement::of(
                vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                None,
            ),
        )
        .unwrap();
        let known = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .recipe_book
            .clone();
        game.native_net.placement.session = Some(Session {
            recipe: selection.display.recipe.as_str().to_owned(),
            graph: Backings::capture(game, &before, &plan).unwrap(),
            menu: Box::new(Menu::new(Kind::Mod(before.kind))),
            plan,
            before,
            known,
            limited: false,
            use_max: false,
            placed: false,
        });
        let recipe = "review:recipe";
        let first = game.place_recipe_in_grid(recipe, 1, 1, vec![0], vec![0], false);
        assert_eq!(
            first,
            Ok(true),
            "no iron ore anywhere: the grid is cleared and ghosted"
        );
        assert_eq!(
            game.place_recipe_in_grid(recipe, 1, 1, vec![0], vec![0], false),
            Err("place_recipe_in_grid was already called for this click".to_owned())
        );
    });
}

struct BackedPack;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BackedPack {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let item = r.item_key("minecraft:shulker_box");
        r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
    }
}

/// The menu's part: it places the clicked recipe in a one-slot grid, as a mod would.
struct Placer;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Placer {
    fn recipe_book_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeBookPlaceEvent<'_>,
    ) {
        event.ghost = host
            .place_recipe_in_grid(event.recipe, 1, 1, vec![0], vec![0], event.use_max_items)
            .unwrap();
    }
}
struct PlacerMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for PlacerMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("review:menu", Placer);
    }
}
#[test]
#[ignore = "daily"]
fn a_listening_part_places_the_recipe_through_the_real_stage() {
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("backed_pack_fixture", Box::new(BackedPack)),
        mod_api::ModEntry::new("review", Box::new(PlacerMod)),
    ]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let (_, opened, selection) = open_mod_menu(game);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                let open = g.menu.as_mut().unwrap();
                open.slots = Slots::Native;
                open.pos = None;
                g.native_menu = vec![None; open.size];
                g.inventory[0] = Some(sunrise_client::items::ItemStack::new(
                    "minecraft:shulker_box",
                    1,
                ));
                g.inventory[9] = Some(sunrise_client::items::ItemStack::new(
                    "minecraft:iron_ore",
                    3,
                ));
            });
            game.player_menu_opening(opened.conn);
            let menu = game.menu(opened.uuid, opened.window).unwrap();
            let places = (0..3)
                .map(|slot| MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot,
                })
                .collect();
            assert_eq!(
                game.player_back_mod_menu_slots(menu.fields().player, 1, places),
                Ok(())
            );
            let before = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
            let plan = MenuPlacement::for_mod(
                "review:menu".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
            )
            .unwrap();
            let root = game.transaction_open();
            assert_eq!(
                game.stage_mod_book_place(&before, &selection, &plan, false),
                Ok(PostPlace::Nothing)
            );
            game.transaction_close(&root, true);
            let placed = {
                let host: &mut dyn ServerHost = game;
                ItemPlace::MenuSlot { menu, slot: 0 }
                    .with_stack(host, |stack| stack.project())
                    .flatten()
                    .unwrap()
            };
            assert_eq!(
                placed.item(),
                mod_api::__private::item_key("minecraft:iron_ore").unwrap()
            );
            assert_eq!(placed.count(), 1);
            let left = game.lan.as_ref().unwrap().hub.guest(12).unwrap().inventory[9]
                .as_ref()
                .unwrap()
                .count;
            assert_eq!(left, 2);

            let mut stale_contents = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
            let nested = ItemPlace::Nested {
                parent: Box::new(ItemPlace::PlayerHand {
                    player: stale_contents.uuid,
                    hand: Hand::Main,
                }),
                slot: 0,
            };
            let dirt =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:dirt").unwrap(), 1)
                    .unwrap();
            let dirt = ItemResource::of(&dirt);
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| {
                    let old = nested.resource(host).ok_or(())?;
                    let count = nested.amount(host);
                    if nested.extract(host, tx, &old, count) != count
                        || nested.insert(host, tx, &dirt, 1) != 1
                    {
                        return Err(());
                    }
                    Ok(())
                }),
                Ok(())
            );
            stale_contents.inventory = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .inventory
                .clone();
            let root = game.transaction_open();
            assert_eq!(
                game.stage_mod_book_place(&stale_contents, &selection, &plan, false),
                Err(sync::Refusal::Changed)
            );
            game.transaction_close(&root, false);

            let stale_identity = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
            let places = [4, 1, 2]
                .into_iter()
                .map(|slot| MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot,
                })
                .collect();
            assert_eq!(
                game.player_back_mod_menu_slots(menu.fields().player, 1, places),
                Ok(())
            );
            let root = game.transaction_open();
            assert_eq!(
                game.stage_mod_book_place(&stale_identity, &selection, &plan, false),
                Err(sync::Refusal::Changed)
            );
            game.transaction_close(&root, false);
        });
    });
}
