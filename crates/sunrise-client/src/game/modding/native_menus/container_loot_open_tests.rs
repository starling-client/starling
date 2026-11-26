//! A guest opening a mod container that still waits with a loot table
//! (`RandomizableContainerBlockEntity.canOpen` and `createMenu`, then
//! `RandomizableContainer.unpackLootTable(player)`). The menu fixture registers a menu type in
//! the process's table, so each test runs in a fresh process.
use super::super::lan_actions::MenuOpen;
use super::super::native_entity_refs::tests::run_fresh_cases;
use super::tests::{MOD_MENU, mod_menu_guest, sent, with_mod_menu_game};
use super::*;
use std::sync::Arc;
use sunrise_client::containers::{BlockEntity, ModContainer};
use sunrise_core::nbt::Tag;
use sunrise_protocol::packets::play::Clientbound as C;

const KEY: (i32, i32, i32) = (0, 64, 0);
/// Guest 1 of `mod_menu_guest`.
const CONN: u32 = 1;
/// One emerald, and one more per point of the roller's luck (`LootPool.bonusRolls`).
const LUCKY: &[u8] = br#"{"pools":[{"rolls":1,"bonus_rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]}]}"#;

/// A nine-slot mod container at `KEY` whose table `test:lucky` has not rolled, opened by a
/// Sunrise guest with a luck of 3, on a server with no player of its own.
fn with_pending_container(
    test: impl FnOnce(&mut Game<'_>, &std::sync::mpsc::Receiver<sunrise_client::lan::Out>),
) {
    with_mod_menu_game(|game| {
        let (rx, _) = mod_menu_guest(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(KEY.0, KEY.1, KEY.2, stone).unwrap();
        game.world.block_entities.set_mod_containers(
            [(
                stone.0,
                Arc::new(ModContainer {
                    slots: 9,
                    menu: Some(MOD_MENU.into()),
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        game.world
            .block_entities
            .insert(KEY, BlockEntity::Chest(vec![None; 9]));
        let files = [("loot_table/test:lucky".into(), vec![LUCKY.to_vec()])];
        let loot = sunrise_world::loot::LootData::load(
            &game.options.resources,
            &sunrise_world::loot::PackOverlay {
                files: &files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        game.world.with_chunk_extras(KEY, |extras| {
            extras.pending_loot.insert(
                KEY,
                sunrise_client::save::PendingContainerLoot {
                    table: Tag::String("test:lucky".into()),
                    seed: Some(Tag::Long(5)),
                },
            );
        });
        game.as_guest(CONN, |guest| {
            guest
                .player
                .as_mut()
                .unwrap()
                .set_modifier("minecraft:luck", "test:luck", 3.0, "add_value", true)
                .unwrap();
        })
        .unwrap();
        assert!(game.player.is_none(), "no host player stands in");
        sent(&rx);
        test(game, &rx);
    });
}

fn emeralds(game: &Game<'_>) -> u32 {
    game.world.block_entities[&KEY]
        .slots()
        .iter()
        .flatten()
        .filter(|stack| &*stack.item == "minecraft:emerald")
        .map(|stack| stack.count)
        .sum()
}

#[test]
#[ignore = "daily"]
fn a_refused_open_leaves_a_mod_containers_table_waiting() {
    if run_fresh_cases(
        "game::native_menus::container_loot_open_tests::a_refused_open_leaves_a_mod_containers_table_waiting",
        "SUNRISE_CONTAINER_LOOT_REFUSED_CHILD",
        &["refused"],
    ) {
        return;
    }
    with_pending_container(|game, rx| {
        // BaseContainerBlockEntity.canOpen: an empty hand does not open a diamond lock.
        let lock = Tag::compound([(
            "lock",
            Tag::compound([("items", Tag::String("minecraft:diamond".into()))]),
        )]);
        game.world.set_block_lock(KEY, &lock);
        assert_eq!(game.open_block_menu(CONN, KEY), MenuOpen::Refused);
        assert!(game.block_loot_pending(KEY), "a locked open rolls nothing");
        assert_eq!(emeralds(game), 0);
        game.world
            .set_block_lock(KEY, &Tag::Compound(Default::default()));
        sent(rx);

        // RandomizableContainerBlockEntity.canOpen: no spectator while the table waits.
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.game_mode = sunrise_client::lan::guest::SPECTATOR;
        });
        assert_eq!(game.open_block_menu(CONN, KEY), MenuOpen::Refused);
        assert!(game.block_loot_pending(KEY), "a spectator rolls nothing");
        assert_eq!(emeralds(game), 0);
        let told = sunrise_client::lan::data::component_nbt(
            &sunrise_client::text::Text::translate("container.spectatorCantOpen", &[])
                .with_color(sunrise_client::text::RED)
                .component(),
        );
        assert!(
            sent(rx).iter().any(|packet| matches!(
                packet,
                C::SystemChat(chat) if chat.overlay && chat.content == told
            )),
            "the spectator is told on the action bar"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_guests_open_rolls_a_mod_containers_table_with_the_guests_luck() {
    if run_fresh_cases(
        "game::native_menus::container_loot_open_tests::a_guests_open_rolls_a_mod_containers_table_with_the_guests_luck",
        "SUNRISE_CONTAINER_LOOT_GUEST_CHILD",
        &["guest"],
    ) {
        return;
    }
    with_pending_container(|game, _| {
        assert_eq!(game.open_block_menu(CONN, KEY), MenuOpen::Opened);
        assert!(!game.block_loot_pending(KEY));
        // One roll and one per point of the guest's luck of 3; the state outside the guest's
        // turn has no luck and would give one.
        assert_eq!(emeralds(game), 4);
    });
}
