//! `DedicatedServer.endMetricsRecordingTick`: tick-time logging starts the tick after the
//! snapshot first contains an operator who asked for `dedicated_server_tick_time`.
use super::dedicated_walk::{boot, join, resources};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    buf::Reader,
    connection::Connection,
    ids::play::clientbound::DEBUG_SAMPLE,
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
        status::Ping,
    },
};

const TICK_MARKER: &str = "sample-test-tick-boundary";

/// A literal marker may be a component itself or an argument of a chat component.
fn has_marker(component: &Tag, marker: &Tag) -> bool {
    match component {
        Tag::String(_) => component == marker,
        Tag::List(entries) => entries.iter().any(|entry| has_marker(entry, marker)),
        Tag::Compound(entries) => entries.iter().any(|(_, entry)| has_marker(entry, marker)),
        _ => false,
    }
}

/// A vanilla tick function marks every running server tick before its wait and sample.
fn install_tick_marker(game: &Path) {
    let pack = game.join("world/datapacks/tick-marker");
    let format = sunrise_core::version::DATA_PACK_FORMAT;
    fs::create_dir_all(pack.join("data/minecraft/tags/function")).unwrap();
    fs::create_dir_all(pack.join("data/test/function")).unwrap();
    fs::write(
        pack.join("pack.mcmeta"),
        serde_json::json!({"pack": {"description": "Tick marker", "min_format": format, "max_format": format}})
            .to_string(),
    )
    .unwrap();
    fs::write(
        pack.join("data/minecraft/tags/function/tick.json"),
        r#"{"values":["test:mark_tick"]}"#,
    )
    .unwrap();
    fs::write(
        pack.join("data/test/function/mark_tick.mcfunction"),
        format!("say {TICK_MARKER}\n"),
    )
    .unwrap();
}

/// A joined player that answers what keeps it in the world: teleports, chunk batches, keep-alives.
struct Bot {
    c: Connection,
    player_id: Option<i32>,
    loaded: bool,
}
impl Bot {
    fn send(&mut self, p: S) {
        self.c.send(&p.encode()).unwrap();
    }
    /// The next packet within 50 ms, after the replies it needs.
    fn step(&mut self) -> Option<C> {
        let (id, payload) = match self.c.recv() {
            Ok(packet) => packet,
            Err(e) if e.is_timed_out() => return None,
            Err(e) => panic!("connection lost: {e}"),
        };
        let packet = if id == DEBUG_SAMPLE {
            // ClientboundDebugSamplePacket.write: VarInt length, four big-endian longs
            // (TpsDebugDimensions), then the TICK_TIME enum ordinal as a VarInt.
            let mut frame = Reader::new(&payload);
            assert_eq!(frame.varint().expect("sample length"), 4);
            for _ in 0..4 {
                frame.i64().expect("sample dimension");
            }
            assert_eq!(frame.varint().expect("sample kind"), 0);
            assert_eq!(frame.remaining(), 0, "trailing bytes in DEBUG_SAMPLE");
            C::decode(id, &payload).expect("valid DEBUG_SAMPLE")
        } else {
            C::decode(id, &payload).ok()?
        };
        match &packet {
            C::Login(login) => self.player_id = Some(login.player_id),
            C::PlayerPosition(p) => {
                self.send(S::AcceptTeleportation(play::AcceptTeleportation {
                    id: p.teleport,
                    x: p.position[0],
                    y: p.position[1],
                    z: p.position[2],
                    yaw: 0.0,
                    pitch: 0.0,
                }));
                if !self.loaded {
                    self.loaded = true;
                    self.send(S::PlayerLoaded(Empty));
                }
            }
            C::KeepAlive(k) => self.send(S::KeepAlive(k.clone())),
            C::ChunkBatchFinished(_) => {
                self.send(S::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 20.0,
                }))
            }
            C::Disconnect(d) => panic!("disconnected: {d:?}"),
            _ => {}
        }
        Some(packet)
    }
    fn request(&mut self, subscriptions: Vec<i32>) {
        self.send(S::DebugSubscriptionRequest(
            play::DebugSubscriptionRequest { subscriptions },
        ));
    }
    /// Require the boundary/acknowledgement; count samples before it in wire order.
    #[track_caller]
    fn until(
        &mut self,
        expected: &str,
        mut done: impl FnMut(&C, &[Vec<i64>]) -> bool,
    ) -> Vec<Vec<i64>> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut samples = Vec::new();
        while Instant::now() < deadline {
            let Some(packet) = self.step() else { continue };
            if let C::DebugSample(s) = &packet {
                samples.push(s.sample.clone());
            }
            if done(&packet, &samples) {
                return samples;
            }
        }
        panic!("did not receive {expected}; saw {} samples", samples.len());
    }
    #[track_caller]
    fn until_chat(&mut self, marker: &str) -> Vec<Vec<i64>> {
        let literal = Tag::String(marker.into());
        self.until(marker, |p, _| match p {
            C::DisguisedChat(c) => has_marker(&c.message, &literal),
            C::SystemChat(c) => has_marker(&c.content, &literal),
            _ => false,
        })
    }
    fn silence(&mut self) {
        // The first marker aligns to a tick; the next two close whole ticks.
        for _ in 0..3 {
            let samples = self.until_chat(TICK_MARKER);
            assert!(samples.is_empty(), "unexpected samples: {samples:?}");
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_dedicated_server_sends_tick_samples_only_to_an_operator_who_asked() {
    let resources = resources().expect("SUNRISE_TEST_RESOURCES must name prepared resources");
    let scratch = tempfile::tempdir().unwrap();
    install_tick_marker(&scratch.path().join("server"));
    let (_server, port, _read, _log, mut console) = boot(scratch.path(), &resources);
    let mut say = |line: &str| writeln!(console, "{line}").unwrap();
    let mut bot = Bot {
        c: join(port, "Sampler").unwrap(),
        player_id: None,
        loaded: false,
    };
    bot.c
        .reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    bot.until_chat(TICK_MARKER);
    let player_id = bot.player_id.expect("play login before the tick marker");

    // The pong proves the preceding request was processed without operator permission.
    bot.request(vec![0]);
    bot.send(S::PingRequest(Ping { time: 42 }));
    assert!(
        bot.until(
            "pong",
            |p, _| matches!(p, C::PongResponse(p) if p.time == 42)
        )
        .is_empty()
    );
    bot.silence();

    // The permission event is sent by /op before this tick's tick function. The next
    // marker ends the first eligible tick, and the one after it ends the sampled tick.
    say("op Sampler");
    assert!(
        bot.until(
            "operator permission",
            |p, _| matches!(p, C::EntityEvent(e) if e.id == player_id && e.event == 28)
        )
        .is_empty()
    );
    assert!(bot.until_chat(TICK_MARKER).is_empty());
    assert!(
        bot.until_chat(TICK_MARKER).is_empty(),
        "sample on first eligible tick"
    );
    let samples = bot.until_chat(TICK_MARKER);
    assert_eq!(samples.len(), 1, "the next tick sends exactly one sample");
    for v in &samples {
        assert!(v.iter().all(|n| *n >= 0), "{v:?}");
        assert!(
            v[0] >= v[1] + v[2] + v[3],
            "the tick holds method, tasks and idle: {v:?}"
        );
    }

    // Deop stops the stream; re-op resumes it without another request.
    say("deop Sampler");
    bot.until(
        "permission level zero from /deop",
        |p, _| matches!(p, C::EntityEvent(e) if e.id == player_id && e.event == 24),
    );
    bot.silence();
    say("op Sampler");
    bot.until("sample after re-op", |_, s| !s.is_empty());

    // Require the command after the empty request as an acknowledgement before checking silence.
    bot.request(vec![]);
    bot.send(S::ChatCommand(play::ChatCommand {
        command: "say sample-test-cleared".into(),
    }));
    bot.until_chat("sample-test-cleared");
    bot.silence();
    bot.request(vec![0]);
    bot.until("sample after resubscribing", |_, s| !s.is_empty());
}
