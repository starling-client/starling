use super::*;
use crate::game::native_client_session::tests::{finish_login, logged_client};
use crate::game::native_entity_refs::tests::{bind_acting_guest, with_client_game, with_game};
use mod_api::{Collider, ColliderBox};
use std::sync::Arc;
use sunrise_client::{chunk::Chunk, entities::ItemEntity, items::ItemStack, motion::PlayerMotion};
use sunrise_core::nbt::Tag;
use sunrise_protocol::connection::Connection;
use sunrise_protocol::packets::play::{self, Clientbound, Serverbound};

const PLAYER: u32 = 42;
const CONN: u32 = 1;
const PIG_Y: f64 = 73.0;

fn empty_chunk(game: &mut Game<'_>) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
}

fn shape() -> Collider {
    Collider {
        boxes: vec![ColliderBox {
            min: [-0.25, -3.0, -0.75],
            max: [0.25, -1.0, 0.75],
        }],
        ..Collider::default()
    }
}

fn holder(game: &mut Game<'_>, x: f64) -> u32 {
    game.options.dedicated = Some(Vec::new());
    game.native_level_opened(game.dim);
    game.native_joins_skip();
    let id = game
        .summon(
            game.dim.id(),
            "minecraft:pig",
            [x, PIG_Y, 4.5],
            [
                ("NoGravity".into(), Tag::Byte(1)),
                ("NoAI".into(), Tag::Byte(1)),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap();
    game.entity(id)
        .unwrap()
        .set_collider(game, Some(shape()))
        .unwrap();
    id
}

fn shove(game: &mut Game<'_>, holder: u32, x: f64) {
    let start = game.placed_moving_colliders();
    let i = game.mobs.iter().position(|m| m.id == holder).unwrap();
    game.mobs.update(i, |m| m.position.x = x);
    game.carry_pass(&start);
}

fn item(id: u32, x: f64, z: f64, velocity: Vec3) -> ItemEntity {
    ItemEntity::new(
        id,
        ItemStack::new("minecraft:stone", 1),
        Vec3::new(x, 70.25, z),
        velocity,
        0.0,
    )
}

#[test]
#[ignore = "daily"]
fn a_collider_shoves_a_body_crossed_only_mid_sweep() {
    crate::game::in_own_process(
        "game::piston_push::carrying::shoving::tests::a_collider_shoves_a_body_crossed_only_mid_sweep",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 2.5);
                let velocity = Vec3::new(-0.125, 0.0, 0.0625);
                game.dropped.push(item(80, 4.5, 4.5, velocity));
                // The other item only grazes the sweep in Z.
                game.dropped.push(item(81, 4.5, 5.375, velocity));
                shove(game, holder, 6.5);
                // The box ends at x=6.75; the item's half width is 0.125.
                assert_eq!(
                    game.dropped[0].position,
                    Vec3::new(6.75 + 0.125 + 0.01, 70.25, 4.5)
                );
                assert_eq!(game.dropped[0].velocity, velocity);
                assert_eq!(game.dropped[1].position, Vec3::new(4.5, 70.25, 5.375));
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_collider_shove_stops_at_a_wall_without_impact_damage() {
    crate::game::in_own_process(
        "game::piston_push::carrying::shoving::tests::a_collider_shove_stops_at_a_wall_without_impact_damage",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 2.5);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(6, 70, 4, stone).unwrap();
                let item = item(80, 4.5, 4.5, Vec3::ZERO);
                let health = item.health;
                game.dropped.push(item);
                shove(game, holder, 6.5);
                assert_eq!(game.dropped[0].position, Vec3::new(6.0 - 0.125, 70.25, 4.5));
                assert_eq!(game.dropped[0].health, health);
                assert_eq!(game.collider_origin(holder).unwrap().x, 6.5);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_later_shove_collides_with_a_holder_already_displaced() {
    crate::game::in_own_process(
        "game::piston_push::carrying::shoving::tests::a_later_shove_collides_with_a_holder_already_displaced",
        || {
            with_game(|game| {
                empty_chunk(game);
                let source = holder(game, 2.5);
                let first = game.take_entity_id();
                let second = game.take_entity_id();
                game.dropped.push(item(first, 4.0, 4.125, Vec3::ZERO));
                game.dropped.push(item(second, 4.5, 4.875, Vec3::ZERO));
                game.entity(first)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider {
                            boxes: vec![ColliderBox {
                                min: [-1.0, -0.25, 0.5],
                                max: [-0.75, 0.75, 1.0],
                            }],
                            ..Collider::default()
                        }),
                    )
                    .unwrap();
                shove(game, source, 6.5);
                let first_x = 6.75 + 0.125 + 0.01;
                assert_eq!(game.dropped[0].position.x, first_x);
                // The first item's box began behind the second item. After the first shove,
                // its west face stops the second item before the source's end position.
                assert_eq!(game.dropped[1].position.x, first_x - 1.0 - 0.125);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_predicted_collider_shove_passes_the_server_move_check() {
    crate::game::in_own_process(
        "game::piston_push::carrying::shoving::tests::a_predicted_collider_shove_passes_the_server_move_check",
        || {
            let from = Vec3::new(5.0, 70.25, 4.5);
            let (packet, to) = {
                let mut report = None;
                with_client_game(|game| {
                    let mut wire = Connection::local(logged_client(game));
                    wire.reader
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    finish_login(game, &mut wire);
                    game.title = None;
                    game.screen = crate::game::Screen::Playing;
                    empty_chunk(game);
                    let mut player = PlayerMotion::new(game.motion, from).unwrap();
                    player.entity_id = PLAYER as i32;
                    player.client_side = true;
                    game.player = Some(player);
                    let source = 80;
                    game.spawn_entity(source, "minecraft:pig", [4.0, PIG_Y, 4.5], [0.0; 3]);
                    let remote = game.remote.as_mut().unwrap();
                    remote.entity_id = PLAYER as i32;
                    remote.loaded_sent = true;
                    remote.level_load.finish();
                    remote.colliders.insert(
                        source as u32,
                        sunrise_client::colliders::ColliderState::from_api(&shape()).unwrap(),
                    );
                    // The last client tick saw the old pose; packet intake precedes the next.
                    game.tick().unwrap();
                    loop {
                        let (id, bytes) = wire.recv().unwrap();
                        if matches!(
                            Serverbound::decode(id, &bytes).unwrap(),
                            Serverbound::ClientTickEnd(_)
                        ) {
                            break;
                        }
                    }
                    game.handle_packet(Clientbound::EntityPositionSync(play::EntityPositionSync {
                        id: source,
                        path: play::PositionPath::Linear([4.75, PIG_Y, 4.5]),
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: false,
                    }))
                    .unwrap();
                    game.tick().unwrap();
                    let player = game.player.as_ref().unwrap();
                    let to = player.position.components();
                    assert_eq!(player.bounds().min_position().x, 5.0 + 0.01);
                    let mut movement = None;
                    loop {
                        let (id, bytes) = wire.recv().unwrap();
                        let packet = Serverbound::decode(id, &bytes).unwrap();
                        let sent = match &packet {
                            Serverbound::MovePlayerPos(p) => [p.x, p.y, p.z],
                            Serverbound::MovePlayerPosRot(p) => [p.x, p.y, p.z],
                            Serverbound::ClientTickEnd(_) => break,
                            _ => continue,
                        };
                        assert_eq!(sent, to, "the tick's own report includes the shove");
                        assert!(movement.replace(packet).is_none());
                    }
                    report = Some((movement.expect("position before ClientTickEnd"), to));
                });
                report.unwrap()
            };
            with_game(|game| {
                empty_chunk(game);
                holder(game, 4.75);
                game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
                game.lan_acting = Some(CONN);
                game.lan_acting_entity = Some(PLAYER);
                game.lan_acting_uuid = Some(11);
                game.lan_acting_name = Some("pushed-player".into());
                bind_acting_guest(game);
                // The packet belongs to the hub guest, not a second local body.
                game.remove_local_player_body();
                game.lan_acting = None;
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.update_guest(CONN, |g| {
                    g.loaded = true;
                    g.load_timeout = 0;
                    g.inbox
                        .push_back(sunrise_client::lan::guest::InboxItem::Packet(packet, None));
                });
                let placed = game.placed_moving_colliders();
                moving_colliders::in_level(placed, || game.tick_lan_packets());
                let guest = game.lan.as_ref().unwrap().hub.guest(CONN).unwrap();
                assert_eq!(
                    guest.received_moves, 1,
                    "the normal movement handler read the report"
                );
                assert_eq!(
                    guest.awaiting_teleport, None,
                    "the normal movement check did not correct it"
                );
                assert_eq!(guest.position, to);
            });
        },
    );
}
