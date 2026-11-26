use super::*;
use crate::game::{
    native_entity_refs::tests::with_client_game, native_guest_turn_tests::with_guests,
};
use std::sync::mpsc::{Receiver, channel};
use sunrise_client::lan::{Out, PlayerSender, guest::InboxItem};
use sunrise_protocol::{buf::Reader, connection::ConnReader};

const OWNER: u32 = 12;
const START: Vec3 = Vec3::new(4.5, 64.0, 4.5);

/// The arena's one chunk carries plains in every quart `BiomeManager.getBiome` can pick for
/// the blocks `Entity.isInRain` samples at either start height: the feet blocks (4, 64, 4) and
/// (4, 65, 4) and the tops (4, 65, 4) and (4, 66, 4), whose corners are the quarts with x and
/// z in {0, 1} and y in 15..=17.
fn arena(game: &mut Game<'_>, roof: bool) {
    let plains = game.res.biomes().id("minecraft:plains");
    let mut chunk = Chunk::empty(0, 0);
    let bottom = game.world.bottom_y() >> 2;
    for x in 0..=1 {
        for y in 15..=17 {
            for z in 0..=1 {
                chunk.set_biome_cell(x, y - bottom, z, plains);
            }
        }
    }
    game.streaming.insert_remote(&mut game.world, chunk);
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    for z in 4..=12 {
        game.world.set(4, 63, z, stone).unwrap();
    }
    if roof {
        // A low roof blocks rain; use water at the feet for this collision case.
        let water = registry.default_state("minecraft:water").unwrap();
        game.world.set(4, 64, 4, water).unwrap();
        game.world.set(4, 66, 4, stone).unwrap();
    }
    game.world.weather.raining = true;
    game.world.weather.settle_levels();
}

/// Transfer the client's real play packets to the integrated server's normal
/// connection drain. The test drives ticks without scheduling two game loops.
fn client_tick_to_server(
    client: &Game<'_>,
    peer: &mut ConnReader,
    server: &mut Game<'_>,
) -> Vec<S> {
    client.send(S::ClientTickEnd(Empty));
    let mut packets = Vec::new();
    loop {
        let (id, bytes) = peer.recv().unwrap();
        let packet = S::decode(id, &bytes).unwrap();
        let end = matches!(packet, S::ClientTickEnd(_));
        packets.push(packet);
        if end {
            break;
        }
    }
    let hub = &mut server.lan.as_mut().unwrap().hub;
    hub.update_guest(OWNER, |g| {
        g.inbox
            .extend(packets.iter().cloned().map(|p| InboxItem::Packet(p, None)));
    })
    .unwrap();
    server.level_pass = Default::default();
    server.tick_lan_packets();
    let hub = &mut server.lan.as_mut().unwrap().hub;
    hub.end_tick();
    hub.guest(OWNER).unwrap().sender.flush();
    packets
}

fn server_to_client(rx: &Receiver<Out>, client: &mut Game<'_>) -> Vec<play::PlayerPosition> {
    let mut teleports = Vec::new();
    for body in rx.try_iter().flat_map(|out| match out {
        Out::Packet(body) => vec![body],
        Out::Packets(bodies) => bodies,
        Out::Disconnect(_) => panic!("owner disconnected"),
    }) {
        let mut wire = Reader::new(&body);
        let id = wire.varint().unwrap();
        let packet = C::decode(id, wire.rest()).unwrap();
        if let C::PlayerPosition(ref position) = packet {
            teleports.push(position.clone());
        }
        client.handle_packet(packet).unwrap();
    }
    teleports
}

#[test]
#[ignore = "daily"]
fn riptide_ground_release_keeps_launch_velocity() {
    crate::game::in_own_process(
        "game::remote::riptide_tests::riptide_ground_release_keeps_launch_velocity",
        || {
            with_guests(|server| {
                server.install_live_features();
                with_client_game(|client| {
                    let (mut peer, _writer) = tests::connect_menu_test(client);
                    tests::spawn_menu_test(client);
                    client.screen = Screen::Playing;
                    let remote = client.remote.as_mut().unwrap();
                    remote.loaded_sent = true;
                    remote.entity_id = OWNER as i32;
                    remote.profile = u128::from(OWNER);
                    remote.level_load.finish();

                    let (tx, rx) = channel();
                    let mut stack = ItemStack::new("minecraft:trident", 1);
                    stack.enchant("minecraft:riptide", 3);
                    for (grounded, roof) in [(true, false), (false, false), (true, true)] {
                        arena(server, roof);
                        arena(client, roof);
                        let start = if grounded {
                            START
                        } else {
                            Vec3::new(START.x, 65.0, START.z)
                        };
                        let mut body = PlayerMotion::new(client.motion, start).unwrap();
                        body.entity_id = OWNER as i32;
                        body.ground.on_ground = grounded;
                        client.player = Some(body);
                        client.bind_local_player_residency();
                        *client.held_mut() = Some(stack.clone());
                        assert!(
                            client.use_trident(&stack),
                            "the client must see water or rain"
                        );
                        client.using.as_mut().unwrap().1 = 20;

                        let hub = &mut server.lan.as_mut().unwrap().hub;
                        hub.singleplayer_owner = Some(u128::from(OWNER));
                        hub.update_guest(OWNER, |g| {
                            g.sender = PlayerSender::new(tx.clone(), Arc::default());
                            g.load_timeout = 0;
                            g.permission = 4;
                            g.position = start.components();
                            g.on_ground = grounded;
                            g.rests_on_ground = grounded;
                            g.motion = None;
                            g.inventory[0] = Some(stack.clone());
                        })
                        .unwrap();
                        server
                            .as_guest(OWNER, |game| {
                                assert!(
                                    game.use_trident(&stack),
                                    "the server must see water or rain"
                                );
                                game.using.as_mut().unwrap().1 = 20;
                            })
                            .unwrap();

                        client.release_remote_use();
                        let packets = client_tick_to_server(client, &mut peer, server);
                        assert!(
                            packets
                                .iter()
                                .any(|p| matches!(p, S::PlayerAction(a) if a.action == 6))
                        );
                        let teleports = server_to_client(&rx, client);
                        let body = client.player.as_ref().unwrap();
                        assert_eq!(body.auto_spin_attack_ticks, 20);
                        assert_eq!(
                            body.velocity,
                            Vec3::new(0.0, 0.0, 3.0),
                            "server cleared the Riptide push: {teleports:?}"
                        );
                        assert!(teleports.is_empty(), "a self-move is not a teleport");
                        let y_after_lift = match (grounded, roof) {
                            (true, false) => 64.0 + f64::from(1.1999999f32),
                            (true, true) => 66.0 - f64::from(1.8f32),
                            (false, _) => start.y,
                        };
                        assert_eq!(body.position.y, y_after_lift);
                        assert!(!body.ground.on_ground);
                        let guest = server.lan.as_ref().unwrap().hub.guest(OWNER).unwrap();
                        assert_eq!(guest.position, body.position.components());
                        assert_eq!(guest.motion.as_ref().unwrap().velocity, body.velocity);
                        assert!(!guest.on_ground && !guest.rests_on_ground);

                        for tick in 0..2 {
                            let z_before = client.player.as_ref().unwrap().position.z;
                            let input = client.tick_remote_own_player().unwrap().unwrap();
                            client.remote_send_state(input);
                            let packets = client_tick_to_server(client, &mut peer, server);
                            assert!(packets.iter().any(|p| matches!(
                                p,
                                S::MovePlayerPos(_) | S::MovePlayerPosRot(_)
                            )));
                            assert!(
                                server_to_client(&rx, client).is_empty(),
                                "server corrected launch tick {tick}"
                            );
                            let body = client.player.as_ref().unwrap();
                            assert!(body.position.z > z_before);
                            if tick == 0 && !roof {
                                assert_eq!(body.position.z, START.z + 3.0);
                            }
                            let guest = server.lan.as_ref().unwrap().hub.guest(OWNER).unwrap();
                            assert_eq!(guest.position, body.position.components());
                            assert_eq!(guest.on_ground, body.ground.on_ground);
                        }
                    }

                    // Chorus fruit still uses inferred position sync; commands explicitly
                    // mark the teleport. Neither is an ordinary self move.
                    server
                        .as_guest(OWNER, |g| {
                            assert!(g.random_teleport(Vec3::new(4.5, 70.0, 11.5), None));
                        })
                        .unwrap();
                    let teleports = server_to_client(&rx, client);
                    assert_eq!(teleports.len(), 1);
                    assert_eq!(teleports[0].position, [4.5, 64.0, 11.5]);
                    client_tick_to_server(client, &mut peer, server);
                    assert!(server_to_client(&rx, client).is_empty());

                    server
                        .as_guest(OWNER, |g| assert!(g.run_command("tp @s 8.5 72 10.5")))
                        .unwrap();
                    let teleports = server_to_client(&rx, client);
                    assert_eq!(teleports.len(), 1);
                    assert_eq!(teleports[0].position, [8.5, 72.0, 10.5]);
                    assert_eq!(
                        client.player.as_ref().unwrap().position.components(),
                        [8.5, 72.0, 10.5]
                    );
                });
            });
        },
    );
}
