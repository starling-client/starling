//! A client's `LevelRef::brightness`, `EntityRef::set_sprinting` and the server's
//! `tick_rate_is_frozen`.

use super::native_entity_refs::tests::with_game;
use super::native_guest_turn_tests::with_guests;
use super::{Game, execution_role::ExecutionRole};
use mod_api::LightLayer;

/// A client reads the light of the level it shows, only while logged in.
#[test]
#[ignore = "daily"]
fn a_client_reads_the_light_of_its_level() {
    use std::sync::Arc;
    use sunrise_client::chunk::{Chunk, Light};
    use sunrise_protocol::connection::Pipe;
    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "BrightnessTest").unwrap();
        game.remote.as_mut().unwrap().logged_in = true;
        let mut chunk = Chunk::empty(0, 0);
        Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xC5);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        let level = game.level("minecraft:overworld").unwrap();
        let lit = [3, -60, 5];
        assert_eq!(
            game.level_brightness(level, LightLayer::Block, lit),
            Some(5)
        );
        assert_eq!(game.level_brightness(level, LightLayer::Sky, lit), Some(12));
        assert_eq!(
            game.level_brightness(level, LightLayer::Sky, [16, -60, 0]),
            None
        );
        assert_eq!(level.can_see_sky(game, lit), Some(false));
        game.remote.as_mut().unwrap().logged_in = false;
        assert_eq!(game.level_brightness(level, LightLayer::Block, lit), None);
    });
}

/// A client's restricted level and handle read its local support and light without loading chunks.
#[test]
#[ignore = "daily"]
fn restricted_level_support_and_sky_reads() {
    use super::native_entity_refs::tests::with_client_game;
    use mod_api::{BlockFace, LevelHost, SupportType};
    use std::sync::Arc;
    use sunrise_client::chunk::{Chunk, Light};
    use sunrise_protocol::connection::Pipe;
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SupportReadTest").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;
            let pos = [3, -60, 5];
            let unloaded = [32, -60, 5];
            // Beyond the top light padding of the Overworld, not a block position.
            let above = [3, game.world.shape.max_y() + 16, 5];
            let mut chunk = Chunk::empty(0, 0);
            Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xF0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
            let level = game.native_client_level().unwrap();
            assert_eq!(
                level.block_face_sturdy(game, pos, BlockFace::Up, SupportType::Full),
                Some(true)
            );
            assert_eq!(level.can_see_sky(game, pos), Some(true));
            assert_eq!(level.can_see_sky(game, above), Some(true));
            assert_eq!(level.can_see_sky(game, unloaded), Some(true));
            game.with_level(|_, level, _| {
                for support in [SupportType::Full, SupportType::Center, SupportType::Rigid] {
                    assert_eq!(
                        level.block_face_sturdy(pos, BlockFace::Up, support),
                        Some(true)
                    );
                    assert_eq!(
                        level.block_face_sturdy([4, -60, 5], BlockFace::Up, support),
                        Some(false)
                    );
                    assert_eq!(
                        level.block_face_sturdy(unloaded, BlockFace::Up, support),
                        None
                    );
                }
                assert_eq!(
                    level.block_face_sturdy(above, BlockFace::Up, SupportType::Full),
                    None
                );
                assert_eq!(level.can_see_sky(pos), Some(true));
                assert_eq!(level.can_see_sky(unloaded), Some(true));
                assert_eq!(level.can_see_sky(above), Some(true));
            });
            // A dimension type with the same range but no sky layer cannot see the sky.
            game.world.shape.sky_light = false;
            assert_eq!(level.can_see_sky(game, above), Some(false));
            game.with_level(|_, level, _| assert_eq!(level.can_see_sky(above), Some(false)));
            assert!(!game.world.loaded(unloaded[0], unloaded[2]));
            // A view without a live client level still refuses, even with a column in memory.
            game.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(level.can_see_sky(game, pos), None);
            assert_eq!(
                level.block_face_sturdy(game, pos, BlockFace::Up, SupportType::Rigid),
                None
            );
            game.with_level(|_, level, _| {
                assert_eq!(
                    level.block_face_sturdy(pos, BlockFace::Up, SupportType::Rigid),
                    None
                );
                assert_eq!(level.can_see_sky(pos), None);
            });
        });
    });
}

/// A guest's sprinting is written to its row, so it survives the turn boundary the body is
/// rebuilt at, and its speed modifier stays with the flag; a stale handle sets nothing.
#[test]
#[ignore = "daily"]
fn a_guests_sprinting_survives_a_turn() {
    with_guests(|game: &mut Game<'_>| {
        let entity = game.player(13).unwrap().entity();
        // A guest has no body until its first turn.
        assert!(game.as_guest(13, |_| ()).is_some());
        assert_eq!(game.entity_is_sprinting(entity), Some(false));
        let stored = |game: &mut Game<'_>| {
            let lan = game.lan.as_mut().unwrap();
            let flag = lan.hub.update_guest(13, |g| g.sprinting).unwrap();
            let modifier = lan.hub.guests.motion_mut(13).unwrap().has_sprint_modifier();
            (flag, modifier)
        };
        assert!(game.entity_set_sprinting(entity, true));
        assert_eq!(stored(game), (true, true));
        assert_eq!(game.entity_is_sprinting(entity), Some(true));
        // The next turn starts from the row, and ends with the row still set.
        let mut in_turn = None;
        game.as_guest(13, |game| {
            in_turn = Some(game.entity_is_sprinting(entity));
        });
        assert_eq!(in_turn, Some(Some(true)));
        assert_eq!(stored(game), (true, true));
        // Inside a turn the write goes to the row as well.
        game.as_guest(13, |game| assert!(game.entity_set_sprinting(entity, false)));
        assert_eq!(stored(game), (false, false));
        let stale = mod_api::EntityRef::new(
            mod_api::WorldEpoch::new(entity.epoch().get() + 1),
            entity.id(),
        );
        assert!(!game.entity_set_sprinting(stale, true));
    });
}

/// The server's frozen state follows `/tick freeze`; a client has none.
#[test]
#[ignore = "daily"]
fn the_tick_rate_is_frozen_on_a_server_only() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.clock.set_frozen(true);
        assert_eq!(game.tick_rate_is_frozen(), Some(true));
        game.clock.set_frozen(false);
        assert_eq!(game.tick_rate_is_frozen(), Some(false));
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;
        assert_eq!(game.tick_rate_is_frozen(), None);
    });
}

/// Menu reads follow the local client's prediction and reject every other player.
#[test]
#[ignore = "daily"]
fn a_client_reads_only_its_own_menu() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        super::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "MenuReadTest").unwrap();
            game.remote.as_mut().unwrap().logged_in = true;
            let player = game.local_player().unwrap();
            let other = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(player.entity().epoch(), player.entity().id() + 1),
                42,
            );
            assert_eq!(game.player_container_menu(other), None);
            let stack = sunrise_client::items::ItemStack::new("minecraft:stone", 7);
            game.inventory[0] = Some(stack.clone());
            game.carried = Some(stack.clone());
            let inventory = game.player_container_menu(player).unwrap();
            assert_eq!(game.menu_slot_count(inventory), Some(46));
            assert_eq!(game.menu_slot(inventory, 36), Some(Some(stack.clone())));
            assert_eq!(game.menu_carried(inventory), Some(Some(stack.clone())));
            game.remote
                .as_mut()
                .unwrap()
                .menu
                .set(Some((1, "minecraft:generic_9x3".into())));
            let open = game.player_container_menu(player).unwrap();
            assert_eq!(game.menu_slot_count(open), Some(63));
            assert_eq!(game.menu_slot(open, 54), Some(Some(stack)));
            assert_eq!(game.menu_slot_count(inventory), None);
            game.remote.as_mut().unwrap().menu.set(None);
            assert_eq!(game.menu_slot_count(open), None);
            assert_eq!(
                game.menu_slot_count(game.player_container_menu(player).unwrap()),
                Some(46)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_merchant_offers_follow_the_open_menu() {
    use super::native_client_session::tests::logged_client;
    use super::native_entity_refs::tests::with_client_game;
    use mod_api::__private::game::Fields;
    use mod_api::ClientHost;
    use sunrise_client::text::Text;
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        item::{Component, Stack, component::STORED_ENCHANTMENTS},
        packets::{
            configuration::{RegistryData, RegistryEntry},
            play,
        },
        registry::Builtin,
    };
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let remote = game.remote.as_mut().unwrap();
            remote.builtin = std::sync::Arc::new(
                Builtin::parse(
                    r#"{
                "minecraft:item":{"entries":{
                    "minecraft:emerald":{"protocol_id":1},
                    "minecraft:book":{"protocol_id":2},
                    "minecraft:enchanted_book":{"protocol_id":3}
                }},
                "minecraft:data_component_type":{"entries":{
                    "minecraft:stored_enchantments":{"protocol_id":45}
                }}
            }"#,
                )
                .unwrap(),
            );
            std::sync::Arc::make_mut(&mut remote.registries).add(RegistryData {
                registry: "minecraft:enchantment".into(),
                entries: vec![RegistryEntry {
                    id: "minecraft:mending".into(),
                    data: None,
                }],
            });
            let current_menu =
                |host: &dyn ClientHost| host.local_player().unwrap().container_menu(host).unwrap();
            let inventory = current_menu(game);
            assert_eq!(inventory.merchant_offers(game), Ok(None));

            let window = 3;
            game.remote_open_merchant(window, String::new(), Text::literal("Merchant"));
            let menu = current_menu(game);
            assert_eq!(menu.merchant_offers(game), Ok(Some(vec![])));
            let player = menu.fields().player;
            let foreign = mod_api::__private::game::menu_ref(
                mod_api::__private::game::player_ref(player.entity(), 99),
                window,
                menu.fields().generation,
            );
            assert_eq!(foreign.merchant_offers(game), Ok(None));

            let discounted_book = play::NetOffer {
                cost_a: Stack::new(1, 10).into(),
                cost_b: Some(Stack::new(2, 1).into()),
                result: Stack {
                    added: vec![(STORED_ENCHANTMENTS, Component::Enchantments(vec![(0, 1)]))],
                    ..Stack::new(3, 1)
                },
                out_of_stock: false,
                uses: 2,
                max_uses: 12,
                xp: 5,
                special_price: -3,
                price_multiplier: 0.5,
                demand: 1,
            };
            let exhausted = play::NetOffer {
                result: Stack::new(2, 1),
                cost_b: None,
                out_of_stock: true,
                ..discounted_book.clone()
            };
            game.handle_packet(play::Clientbound::MerchantOffers(play::MerchantOffers {
                container: window,
                offers: vec![discounted_book, exhausted],
                level: 2,
                xp: 10,
                show_progress: true,
                can_restock: true,
            }))
            .unwrap();
            let offers = menu.merchant_offers(game).unwrap().unwrap();
            let [book, exhausted] = offers.as_slice() else {
                panic!("expected both synchronized offers");
            };
            let item = |id| mod_api::__private::item_key(id).unwrap();
            assert_eq!(book.base_cost_a.stack.item(), item("minecraft:emerald"));
            assert_eq!(book.base_cost_a.stack.count(), 10);
            let second = &book.cost_b.as_ref().unwrap().stack;
            assert_eq!((second.item(), second.count()), (item("minecraft:book"), 1));
            assert_eq!(
                (book.result.item(), book.result.count()),
                (item("minecraft:enchanted_book"), 1)
            );
            assert_eq!(
                book.result
                    .components()
                    .unwrap()
                    .as_nbt()
                    .get(&"minecraft:stored_enchantments".into()),
                Some(&Tag::compound([("minecraft:mending", Tag::Int(1))])),
            );
            assert_eq!((book.uses, book.max_uses, book.xp), (2, 12, 5));
            assert_eq!(
                (book.special_price_diff, book.demand, book.price_multiplier),
                (-3, 1, 0.5)
            );
            assert_eq!(book.cost_a_count(game), Some(12));
            assert!(!book.out_of_stock());
            assert_eq!(exhausted.result.item(), item("minecraft:book"));
            assert!(exhausted.cost_b.is_none());
            assert!(exhausted.out_of_stock());

            // The server may reuse the window id, but not the handle's generation.
            game.remote_open_merchant(window, String::new(), Text::literal("Merchant"));
            let replacement = current_menu(game);
            assert_eq!(menu.merchant_offers(game), Ok(None));
            assert_eq!(replacement.merchant_offers(game), Ok(Some(vec![])));
            game.remote_close_local();
            assert_eq!(replacement.merchant_offers(game), Ok(None));
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_player_state_reads_its_own_copy_only() {
    use super::native_client_session::tests::logged_client;
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client::{items::ItemStack, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let local = game.local_player().unwrap();
            let entity = local.entity();
            let other = mod_api::EntityRef::new(entity.epoch(), 99);
            let other_player = mod_api::__private::game::player_ref(other, 99);
            game.inventory[5] = Some(ItemStack::new("minecraft:stone", 3));
            for flag in [true, false] {
                let body = game.player.as_mut().unwrap();
                body.ground.on_ground = flag;
                body.fluids.was_touching_water = flag;
                body.swimming = flag;
                body.air = if flag { 87 } else { 12 };
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(entity.on_ground(host), Some(flag));
                assert_eq!(entity.is_in_water(host), Some(flag));
                assert_eq!(entity.is_swimming(host), Some(flag));
                assert_eq!(entity.air_supply(host), Some(if flag { 87 } else { 12 }));
                assert_eq!(entity.max_air_supply(host), Some(300));
                assert_eq!(other.on_ground(host), None);
                assert_eq!(other.is_in_water(host), None);
                assert_eq!(other.is_swimming(host), None);
                assert_eq!(other.air_supply(host), None);
                assert_eq!(other.max_air_supply(host), None);
                local
                    .with_inventory(host, |view| {
                        let mut slots = view.iter();
                        let (slot, stack) = slots.next().unwrap();
                        assert_eq!((slot, stack.count()), (5, 3));
                        assert!(slots.next().is_none());
                    })
                    .unwrap();
                assert_eq!(other_player.with_inventory(host, |_| ()), None);
                assert_eq!(local.with_ender_chest(host, |_| ()), None);
            }
        });
    });
}

/// The HUD clock is a client clock: pause stops it, server freeze does not.
#[test]
#[ignore = "daily"]
fn gui_ticks_follow_unpaused_client_ticks() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        super::native_entity_refs::tests::with_client_game(|game| {
            // Isolate the client's own per-tick work from simulation and network setup.
            game.agent.hold_server = true;
            assert_eq!(game.gui_ticks(), 0);
            game.client_tick_steps(2, None).unwrap();
            assert_eq!(game.gui_ticks(), 2);
            game.pause = true;
            game.client_tick_steps(3, None).unwrap();
            assert_eq!(game.gui_ticks(), 2);
            game.pause = false;
            game.clock.set_frozen(true);
            game.client_tick_steps(1, None).unwrap();
            assert_eq!(game.gui_ticks(), 3);
            game.gui_ticks = i32::MAX;
            game.client_tick_steps(1, None).unwrap();
            assert_eq!(game.gui_ticks(), i32::MIN);
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_abilities_follow_predicted_flying_between_packets() {
    use super::native_client_session::tests::logged_client;
    use super::native_entity_refs::tests::with_client_game;
    use sunrise_client::motion::PlayerMotion;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play;
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.handle_packet(play::Clientbound::PlayerAbilities(play::PlayerAbilities {
                flags: 4,
                flying_speed: 0.1,
                walking_speed: 0.2,
            }))
            .unwrap();
            let local = game.local_player().unwrap();
            for flying in [true, false] {
                game.player.as_mut().unwrap().flying = flying;
                let abilities = local.abilities(&*game as &dyn mod_api::ClientHost).unwrap();
                assert_eq!(abilities.flying, flying);
                assert!(abilities.may_fly);
                assert!(!abilities.invulnerable);
                assert!(!abilities.instabuild);
                assert_eq!((abilities.fly_speed, abilities.walk_speed), (0.1, 0.2));
            }
        });
    });
}
