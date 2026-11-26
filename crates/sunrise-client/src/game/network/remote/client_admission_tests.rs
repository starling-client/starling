use super::super::{C, Game, play};
use sunrise_client::sound_engine::{SoundEntity, SoundInstance};
use sunrise_core::random::LegacyRandom;

use crate::game::{
    native_entity_refs::tests::with_client_game, native_hotbar_slot::hotbar_slot_tests::logged_in,
};

fn with_isolated_client(name: &str, body: impl FnOnce(&mut Game<'_>)) {
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    crate::game::in_own_process(&test, || with_client_game(body));
}

fn registry_id(game: &Game<'_>, registry: &str, name: &str) -> i32 {
    game.remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

fn roundtrip(packet: C) -> C {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    C::decode(reader.varint().unwrap(), reader.rest()).unwrap()
}

fn deliver(game: &mut Game<'_>, packet: C) {
    game.handle_packet(roundtrip(packet)).unwrap();
}

fn add_entity(game: &Game<'_>, id: i32, uuid: u128, kind: &str, position: [f64; 3]) -> C {
    C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: registry_id(game, "minecraft:entity_type", kind),
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}

#[test]
#[ignore = "daily"]
fn add_player_requires_info_before_replacing_a_tracked_entity() {
    with_isolated_client(
        "add_player_requires_info_before_replacing_a_tracked_entity",
        |game| {
            let _reader = logged_in(game);
            let id = 42;
            let uuid = 123;
            let player_at = [8.0, 65.0, 8.0];
            let cart_at = [2.0, 64.0, 2.0];
            let player = add_entity(game, id, uuid, "minecraft:player", player_at);
            deliver(game, player.clone());
            assert_eq!(game.remote.as_ref().unwrap().tracked(id), None);
            assert!(
                !game
                    .remote
                    .as_ref()
                    .unwrap()
                    .level_players
                    .contains_key(&id)
            );

            deliver(
                game,
                add_entity(game, id, 124, "minecraft:minecart", cart_at),
            );
            deliver(game, C::SetCamera(play::SetCamera { camera_id: id }));
            assert_eq!(game.live_remote_camera_id(), Some(id as u32));
            let cart = game
                .carts
                .iter()
                .find(|cart| cart.id == id as u32)
                .unwrap()
                .join;
            let sound = SoundEntity::new(id as u32, cart_at, false);
            game.audio.engine.add(
                SoundInstance::entity_bound(
                    "minecraft:fixture.bound",
                    6,
                    1.0,
                    1.0,
                    LegacyRandom::new(42),
                    sound.clone(),
                ),
                true,
            );
            deliver(game, player.clone());
            assert_eq!(game.live_remote_camera_id(), Some(id as u32));
            assert!(!sound.state().removed);
            assert_eq!(
                game.remote.as_ref().unwrap().tracked(id),
                Some(("minecraft:minecart", cart_at))
            );
            assert_eq!(
                game.carts
                    .iter()
                    .find(|cart| cart.id == id as u32)
                    .unwrap()
                    .join,
                cart
            );

            deliver(
                game,
                C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1 | 8,
                    entries: vec![play::PlayerInfo {
                        uuid,
                        name: Some("Unlisted".into()),
                        listed: Some(false),
                        ..Default::default()
                    }],
                }),
            );
            let remote = game.remote.as_ref().unwrap();
            assert!(!remote.listed.contains(&uuid));
            assert_eq!(remote.tracked(id), Some(("minecraft:minecart", cart_at)));
            assert!(!remote.level_players.contains_key(&id));

            deliver(game, player.clone());
            let remote = game.remote.as_ref().unwrap();
            assert_eq!(remote.tracked(id), Some(("minecraft:player", player_at)));
            assert!(sound.state().removed);
            assert_eq!(remote.level_player(id), Some((uuid, Some("Unlisted"))));
            assert_eq!(game.live_remote_camera_id(), None);
            assert_eq!(game.remote_camera_id(), Some(id as u32));
            assert!(game.carts.iter().all(|cart| cart.id != id as u32));

            deliver(
                game,
                C::RemoveEntities(play::RemoveEntities { ids: vec![id] }),
            );
            deliver(
                game,
                C::PlayerInfoRemove(play::PlayerInfoRemove { uuids: vec![uuid] }),
            );
            deliver(game, player);
            assert_eq!(game.remote.as_ref().unwrap().tracked(id), None);
        },
    );
}
