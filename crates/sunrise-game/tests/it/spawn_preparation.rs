//! Exercise `PrepareSpawnTask` across a real configuration acknowledgement and placement.
use super::dedicated_walk::{boot, configure, resources};
use std::{
    fs,
    io::Write,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Connection,
    crypto::{offline_uuid, uuid_string},
    packets::{
        Empty, configuration as cfg,
        play::{self, Clientbound as C, Serverbound as S},
    },
};

fn expect_log(lines: &Receiver<String>, log: &mut Vec<String>, expected: &str) {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let line = lines
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| panic!("Missing {expected:?}\n{}", log.join("\n")));
        let matched = line.contains(expected);
        log.push(line);
        if matched {
            break;
        }
    }
}

fn play_until<T>(bot: &mut Connection, mut select: impl FnMut(C) -> Option<T>) -> T {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let left = until.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "missing expected play packet");
        bot.reader.set_read_timeout(Some(left)).unwrap();
        let (id, data) = bot.recv().unwrap();
        let Ok(packet) = C::decode(id, &data) else {
            continue;
        };
        match packet {
            C::KeepAlive(k) => bot.send(&S::KeepAlive(k).encode()).unwrap(),
            C::ChunkBatchFinished(_) => bot
                .send(
                    &S::ChunkBatchReceived(play::ChunkBatchReceived {
                        chunks_per_tick: 20.0,
                    })
                    .encode(),
                )
                .unwrap(),
            C::Disconnect(d) => panic!("disconnected before the assertion: {d:?}"),
            p => {
                if let Some(value) = select(p) {
                    return value;
                }
            }
        }
    }
}

#[test]
#[ignore = "daily"]
fn configuration_keeps_the_cold_spawn_corner_loaded_until_an_immediate_fill() {
    let resources = resources().expect("daily needs privately prepared full resources");
    let scratch = tempfile::tempdir().unwrap();
    let game = scratch.path().join("server");
    let world = game.join("world");
    let name = "SpawnWalker";
    let uuid = offline_uuid(name);
    const SPAWN: [f64; 3] = [4096.5, 300.0, 4096.5];
    // The diagonally opposite outer chunk is outside ordinary boot generation.
    const CORNER: &str = "4159 300 4159";
    sunrise_client::lan::playerdata::directory(&world);
    sunrise_client::save::write_dat(
        &sunrise_client::lan::playerdata::path(&world, uuid),
        Tag::compound([
            ("DataVersion", Tag::Int(sunrise_core::version::DATA_VERSION)),
            ("Dimension", Tag::String("minecraft:overworld".into())),
            (
                "Pos",
                Tag::List(
                    SPAWN
                        .into_iter()
                        .map(|v| Tag::Double(v.to_bits()))
                        .collect(),
                ),
            ),
        ]),
    )
    .unwrap();
    fs::write(
        game.join("ops.json"),
        serde_json::json!([{
            "uuid": uuid_string(uuid), "name": name, "level": 4, "bypassesPlayerLimit": false,
        }])
        .to_string(),
    )
    .unwrap();
    let pack = world.join("datapacks/spawn-check");
    let functions = pack.join("data/test/function");
    fs::create_dir_all(&functions).unwrap();
    let format = sunrise_core::version::DATA_PACK_FORMAT;
    fs::write(pack.join("pack.mcmeta"), serde_json::json!({
        "pack": {"description": "Delayed spawn check", "min_format": format, "max_format": format},
    }).to_string()).unwrap();
    fs::write(
        functions.join("check_corner.mcfunction"),
        format!(
            "execute if block {CORNER} minecraft:air unless entity @a[name={name}] run say spawn-corner-loaded\n"
        ),
    )
    .unwrap();
    let (_server, port, lines, mut log, mut console) = boot(scratch.path(), &resources);
    writeln!(
        console,
        "execute unless loaded {CORNER} run say spawn-corner-cold"
    )
    .unwrap();
    expect_log(&lines, &mut log, "spawn-corner-cold");
    let mut bot = configure(port, name).unwrap_or_else(|e| {
        log.extend(lines.try_iter());
        panic!("configuration failed: {e}\n{}", log.join("\n"));
    });

    // Let more than PLAYER_SPAWN's 20 ticks elapse before acknowledging. No
    // player is placed yet to keep the cold corner resident with its view.
    writeln!(console, "schedule function test:check_corner 25t").unwrap();
    expect_log(&lines, &mut log, "spawn-corner-loaded");

    bot.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    let p = play_until(&mut bot, |p| match p {
        C::PlayerPosition(p) => Some(p),
        _ => None,
    });
    assert_eq!(p.position, SPAWN);
    bot.send(
        &S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: SPAWN[0],
            y: SPAWN[1],
            z: SPAWN[2],
            yaw: p.yaw,
            pitch: p.pitch,
        })
        .encode(),
    )
    .unwrap();
    bot.send(&S::PlayerLoaded(Empty).encode()).unwrap();
    bot.send(
        &S::ChatCommand(play::ChatCommand {
            command: format!("fill {CORNER} {CORNER} minecraft:stone"),
        })
        .encode(),
    )
    .unwrap();
    let feedback = play_until(&mut bot, |p| match p {
        C::SystemChat(p)
            if p.content.get("translate") == Some(&Tag::String("commands.fill.success".into())) =>
        {
            Some(p.content)
        }
        _ => None,
    });
    assert_eq!(feedback.get("with"), Some(&Tag::List(vec![Tag::Int(1)])));
}
