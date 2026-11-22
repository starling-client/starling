use super::*;
fn state() -> Holds<Vec<u8>> {
    let mut h = Holds::default();
    h.sync(Some((7, 3)));
    h
}

#[test]
fn release_preserves_a_b_c_and_later_d_and_drop_sends_none() {
    let mut h = state();
    let id = h.start().unwrap();
    let mut sent = Vec::new();
    for packet in [b"A", b"B", b"C", b"D"] {
        assert_eq!(
            h.queue(packet.to_vec(), |p| {
                sent.push(p);
                Ok(())
            }),
            Ok(0)
        );
    }
    assert!(sent.is_empty());
    let packets = h.take(id).unwrap();
    assert_eq!(
        Holds::publish(packets, |p| {
            sent.push(p);
            Ok(())
        }),
        Ok(4)
    );
    assert_eq!(
        sent,
        [b"A".to_vec(), b"B".to_vec(), b"C".to_vec(), b"D".to_vec()]
    );
    assert_eq!(h.take(id), Err(HoldError::Unknown));
    let drop_id = h.start().unwrap();
    h.queue(b"discard".to_vec(), |_| panic!("held packet sent"))
        .unwrap();
    drop(h.take(drop_id).unwrap());
    assert_eq!(sent.len(), 4);
}

#[test]
fn packet_overflow_flushes_256_then_next_and_reports_literal_bounds_once() {
    let mut h = state();
    let id = h.start().unwrap();
    let mut sent = Vec::new();
    for n in 0..256u16 {
        h.queue(n.to_be_bytes().to_vec(), |_| panic!("premature send"))
            .unwrap();
    }
    assert_eq!(
        h.queue(vec![1, 0], |p| {
            sent.push(p);
            Ok(())
        }),
        Ok(257)
    );
    h.queue(b"later".to_vec(), |p| {
        sent.push(p);
        Ok(())
    })
    .unwrap();
    assert_eq!(sent.len(), 258);
    assert_eq!(&sent[..3], &[vec![0, 0], vec![0, 1], vec![0, 2]]);
    assert_eq!(&sent[255..], &[vec![0, 255], vec![1, 0], b"later".to_vec()]);
    assert_eq!(
        h.take(id),
        Err(HoldError::Limit {
            packets: 256,
            bytes: 1_048_576
        })
    );
    assert_eq!(h.take(id), Err(HoldError::Unknown));
}

#[test]
fn byte_overflow_flushes_oldest_first_and_drop_reports_limit() {
    let mut h = state();
    let id = h.start().unwrap();
    let mut sent = Vec::new();
    h.queue(vec![42; 1_048_576], |_| panic!("premature send"))
        .unwrap();
    h.queue(vec![43], |p| {
        sent.push(p);
        Ok(())
    })
    .unwrap();
    assert_eq!(sent, [vec![42; 1_048_576], vec![43]]);
    assert_eq!(
        h.take(id),
        Err(HoldError::Limit {
            packets: 256,
            bytes: 1_048_576
        })
    );
}

#[test]
fn send_failure_reports_one_sent_and_discards_the_tail() {
    let mut h = state();
    let id = h.start().unwrap();
    for p in [b"A", b"B", b"C"] {
        h.queue(p.to_vec(), |_| panic!("premature send")).unwrap();
    }
    let mut sent = Vec::new();
    let result = Holds::publish(h.take(id).unwrap(), |p| {
        if p == b"B" {
            return Err(SendPacketError::Backpressure);
        }
        sent.push(p);
        Ok(())
    });
    assert_eq!(
        result,
        Err(HoldError::Send {
            sent: 1,
            error: SendPacketError::Backpressure
        })
    );
    assert_eq!(sent, [b"A".to_vec()]);
    assert_eq!(h.take(id), Err(HoldError::Unknown));
}

#[test]
fn disconnect_and_protocol_incarnation_change_drop_and_invalidate() {
    let mut h = state();
    let id = h.start().unwrap();
    h.queue(b"old".to_vec(), |_| panic!("premature send"))
        .unwrap();
    h.sync(None);
    assert_eq!(h.take(id), Err(HoldError::Unknown));
    assert_eq!(h.start(), Err(HoldError::NoWorld));
    h.sync(Some((7, 5)));
    let next = h.start().unwrap();
    h.queue(b"new".to_vec(), |_| panic!("premature send"))
        .unwrap();
    h.sync(Some((7, 7)));
    assert_eq!(h.take(next), Err(HoldError::Unknown));
}

#[test]
#[ignore = "daily"]
fn physical_hold_orders_engine_and_mod_packets_and_bypasses_network_keepalive() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use sunrise_protocol::{connection::Pipe, packets::configuration::Ping};
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HoldTest").unwrap();
        let (session, mut peer, keepalive) = sunrise_client::net::Session::packet_hold_probe();
        let remote = game.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        let id = game.hold_packets().unwrap();
        assert_eq!(
            game.hold_packets(),
            Err(HoldError::Limit {
                packets: 256,
                bytes: 1_048_576
            })
        );
        for n in [1, 2, 3] {
            assert!(game.send_play(S::Pong(Ping { id: n })));
        }
        game.send_packet(ServerboundPlayPacket::new(
            mod_api::Identifier::parse("minecraft:pong").unwrap(),
            vec![0, 0, 0, 4],
        ))
        .unwrap();
        keepalive(9);
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::KEEP_ALIVE,
                vec![0, 0, 0, 0, 0, 0, 0, 9]
            )
        );
        assert_eq!(game.release_held(id), Ok(4));
        for body in [[0, 0, 0, 1], [0, 0, 0, 2], [0, 0, 0, 3], [0, 0, 0, 4]] {
            assert_eq!(
                peer.recv().unwrap(),
                (
                    sunrise_protocol::ids::play::serverbound::PONG,
                    body.to_vec()
                )
            );
        }
        let dropped = game.hold_packets().unwrap();
        game.send_play(S::Pong(Ping { id: 5 }));
        assert_eq!(game.drop_held(dropped), Ok(()));
        game.send_play(S::Pong(Ping { id: 6 }));
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::PONG,
                vec![0, 0, 0, 6]
            )
        );
        let disconnected = game.hold_packets().unwrap();
        game.send_play(S::Pong(Ping { id: 7 }));
        let mut old = game.remote.take().unwrap();
        old.session.disconnect();
        assert_eq!(game.release_held(disconnected), Err(HoldError::Unknown));
    });
}

#[test]
fn no_hold_sends_immediately_and_unknown_id_preserves_an_active_queue() {
    let mut h = state();
    let mut sent = Vec::new();
    assert_eq!(
        h.queue(b"A".to_vec(), |p| {
            sent.push(p);
            Ok(())
        }),
        Ok(1)
    );
    assert_eq!(sent, [b"A".to_vec()]);
    let id = h.start().unwrap();
    h.queue(b"B".to_vec(), |_| panic!("premature send"))
        .unwrap();
    assert_eq!(h.take(HoldId::new(0)), Err(HoldError::Unknown));
    assert_eq!(h.take(id), Ok(vec![b"B".to_vec()]));
}

#[test]
fn second_hold_returns_literal_limit_and_first_releases_a_b_c_in_order() {
    let mut h = state();
    let first = h.start().unwrap();
    h.queue(b"A".to_vec(), |_| panic!("premature send"))
        .unwrap();
    assert_eq!(
        h.start(),
        Err(HoldError::Limit {
            packets: 256,
            bytes: 1_048_576
        })
    );
    for packet in [b"B", b"C"] {
        h.queue(packet.to_vec(), |_| panic!("premature send"))
            .unwrap();
    }
    let mut sent = Vec::new();
    assert_eq!(
        Holds::publish(h.take(first).unwrap(), |packet| {
            sent.push(packet);
            Ok(())
        }),
        Ok(3)
    );
    assert_eq!(sent, [b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]);
}

#[test]
#[ignore = "daily"]
fn generation_change_after_a_drops_b_and_invalidates_the_hold() {
    use sunrise_protocol::packets::configuration::Ping;
    let (session, mut peer, _keepalive) = sunrise_client::net::Session::packet_hold_probe();
    let incarnation = session.hold_incarnation().unwrap();
    let mut h = Holds::default();
    h.sync(Some(incarnation));
    let id = h.start().unwrap();
    for value in [65, 66] {
        h.queue(S::Pong(Ping { id: value }).try_encode().unwrap(), |_| {
            panic!("held packet sent")
        })
        .unwrap();
    }
    let mut published = 0;
    let result = Holds::publish(h.take(id).unwrap(), |body| {
        session.send_held_body(body, incarnation)?;
        published += 1;
        if published == 1 {
            session.packet_hold_rollover();
        }
        Ok(())
    });
    assert_eq!(
        result,
        Err(HoldError::Send {
            sent: 1,
            error: SendPacketError::NotInPlay
        })
    );
    assert_eq!(published, 1);
    assert_eq!(
        peer.recv().unwrap(),
        (
            sunrise_protocol::ids::play::serverbound::PONG,
            vec![0, 0, 0, 65]
        )
    );
    assert_eq!(peer.recv().unwrap(), (3, vec![]));
    assert_eq!(peer.recv().unwrap(), (3, vec![]));
    peer.reader
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .unwrap();
    assert!(peer.recv().is_err(), "B must not reach the new Play phase");
    assert_eq!(h.take(id), Err(HoldError::Unknown));
}
