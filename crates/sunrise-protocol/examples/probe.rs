//! Log into a server in offline mode and decode everything it sends for a few
//! seconds, reporting packet counts and any decode failures. A differential
//! check of the codecs against a real vanilla server:
//!
//!     cargo run -p sunrise-protocol --example protocol_probe -- 127.0.0.1:25565 [seconds] [registries.json]
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    PROTOCOL_VERSION, State,
    buf::Reader,
    connection::Connection,
    ids,
    packets::{Empty, configuration as cfg, handshake, login, play},
    registry::{Builtin, Dynamic, SyncedNames, ceil_log2},
    servers::parse_address,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().collect();
    let suggestions = args
        .iter()
        .position(|s| s == "--suggestions")
        .is_some_and(|at| {
            args.remove(at);
            true
        });
    let (host, port) =
        parse_address(args.get(1).map_or("127.0.0.1", String::as_str)).ok_or("bad address")?;
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let builtin = args
        .get(3)
        .map(|p| Builtin::load(std::path::Path::new(p)))
        .transpose()?
        .unwrap_or_default();
    let mut c = Connection::connect(&host, port, Duration::from_secs(5))?;
    c.reader.set_read_timeout(Some(Duration::from_secs(10)))?;
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )?;
    let name = "SunriseProbe";
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: sunrise_protocol::crypto::offline_uuid(name),
        })
        .encode(),
    )?;
    loop {
        let (id, payload) = c.recv()?;
        match login::Clientbound::decode(id, &payload)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(p) => {
                println!(
                    "logged in as {} {}",
                    p.profile.name,
                    sunrise_protocol::crypto::uuid_string(p.profile.uuid)
                );
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())?;
                break;
            }
            login::Clientbound::Disconnect(p) => return Err(p.reason_json.into()),
            other => println!("login: {other:?}"),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(cfg::ClientInformation::new(8)).encode())?;
    let mut registries = Dynamic::default();
    let mut decode_errors = 0;
    loop {
        let (id, payload) = c.recv()?;
        let names = SyncedNames {
            builtin: &builtin,
            dynamic: &registries,
        };
        match cfg::Clientbound::decode_reader(id, Reader::new(&payload).with_names(&names)) {
            Ok(cfg::Clientbound::SelectKnownPacks(p)) => {
                println!("known packs offered: {:?}", p.packs);
                c.send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )?;
            }
            Ok(cfg::Clientbound::RegistryData(p)) => {
                println!("registry {} ({} entries)", p.registry, p.entries.len());
                registries.add(p);
            }
            Ok(cfg::Clientbound::KeepAlive(p)) => {
                c.send(&cfg::Serverbound::KeepAlive(p).encode())?
            }
            Ok(cfg::Clientbound::Ping(p)) => c.send(&cfg::Serverbound::Pong(p).encode())?,
            Ok(cfg::Clientbound::CodeOfConduct(_)) => {
                c.send(&cfg::Serverbound::AcceptCodeOfConduct(Empty).encode())?
            }
            Ok(cfg::Clientbound::FinishConfiguration(_)) => {
                c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())?;
                break;
            }
            Ok(cfg::Clientbound::Disconnect(p)) => return Err(format!("{:?}", p.reason).into()),
            Ok(cfg::Clientbound::UpdateTags(p)) => {
                println!("tags for {} registries", p.registries.len())
            }
            Ok(other) => println!(
                "configuration: {}",
                ids::name(
                    State::Configuration,
                    sunrise_protocol::Direction::Clientbound,
                    other.id()
                )
                .unwrap_or("?")
            ),
            Err(e) => {
                decode_errors += 1;
                println!("configuration decode error in {id}: {e}");
            }
        }
    }
    let names = SyncedNames {
        builtin: &builtin,
        dynamic: &registries,
    };
    let biome_bits = ceil_log2(registries.len("minecraft:worldgen/biome"));
    let mut counts: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut entity_types: BTreeMap<String, u32> = BTreeMap::new();
    let mut dimension = registries.dimension(0);
    let start = Instant::now();
    let mut suggestion_sent = false;
    let mut suggestion_received = false;
    c.reader
        .set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut ticks = 0;
    while start.elapsed() < Duration::from_secs(seconds) {
        let (id, payload) = match c.recv() {
            Ok(p) => p,
            Err(e) if e.is_timed_out() => {
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if suggestions && !suggestion_sent && start.elapsed() >= Duration::from_secs(6) {
            let mut w = Writer::new();
            w.varint(ids::play::serverbound::COMMAND_SUGGESTION);
            w.varint(764);
            w.string("/execute as @e[");
            c.send(&w.data)?;
            suggestion_sent = true;
        }
        let name =
            ids::name(State::Play, sunrise_protocol::Direction::Clientbound, id).unwrap_or("?");
        let entry = counts.entry(name).or_default();
        entry.0 += 1;
        match play::Clientbound::decode_reader(id, Reader::new(&payload).with_names(&names)) {
            Ok(packet) => match packet {
                play::Clientbound::CommandSuggestions(p) if suggestions && p.id == 764 => {
                    let tooltips = p.suggestions.iter().filter(|s| s.tooltip.is_some()).count();
                    if tooltips > 0 {
                        suggestion_received = true;
                        println!(
                            "command_suggestions_validated {} {tooltips}",
                            p.suggestions.len()
                        );
                    }
                }
                play::Clientbound::Login(p) => {
                    dimension = registries.dimension(p.spawn.dimension_type);
                    println!(
                        "login: entity {} in {} ({:?})",
                        p.player_id, p.spawn.dimension, dimension
                    );
                    c.send(&play::Serverbound::PlayerLoaded(Empty).encode())?;
                }
                play::Clientbound::Respawn(p) => {
                    dimension = registries.dimension(p.spawn.dimension_type);
                    c.send(&play::Serverbound::PlayerLoaded(Empty).encode())?;
                }
                play::Clientbound::KeepAlive(p) => {
                    c.send(&play::Serverbound::KeepAlive(p).encode())?
                }
                play::Clientbound::Ping(p) => c.send(&play::Serverbound::Pong(p).encode())?,
                play::Clientbound::PlayerPosition(p) => {
                    println!("position {:?} teleport {}", p.position, p.teleport);
                    let [x, y, z] = p.position;
                    c.send(
                        &play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                            id: p.teleport,
                            x,
                            y,
                            z,
                            yaw: p.yaw,
                            pitch: p.pitch,
                        })
                        .encode(),
                    )?;
                    // One position per client tick: end the tick like the client.
                    c.send(&play::Serverbound::ClientTickEnd(Empty).encode())?;
                }
                play::Clientbound::ChunkBatchFinished(_) => {
                    c.send(
                        &play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                            chunks_per_tick: 50.0,
                        })
                        .encode(),
                    )?;
                    ticks += 1;
                }
                play::Clientbound::LevelChunkWithLight(p) => {
                    if let Err(e) = p.chunk.sections(dimension.sections(), 16, biome_bits) {
                        println!("chunk {} {} sections: {e}", p.x, p.z);
                        entry.1 += 1;
                        decode_errors += 1;
                    }
                }
                play::Clientbound::AddEntity(p) => {
                    let kind = builtin
                        .get("minecraft:entity_type")
                        .and_then(|ids| ids.name(p.kind))
                        .unwrap_or("?");
                    *entity_types.entry(kind.to_owned()).or_default() += 1;
                    if counts.get("add_entity").is_some_and(|c| c.0 < 4) {
                        println!("entity {} {kind} at {:?}", p.id, p.position);
                    }
                }
                play::Clientbound::SystemChat(p) => println!(
                    "chat: {}",
                    sunrise_protocol::text::plain(&p.content, &Default::default())
                ),
                play::Clientbound::Disconnect(p) => {
                    return Err(format!("disconnected: {:?}", p.reason).into());
                }
                _ => {}
            },
            Err(e) => {
                entry.1 += 1;
                decode_errors += 1;
                if entry.1 <= 2 {
                    println!("decode error in {name} ({} bytes): {e}", payload.len());
                }
            }
        }
    }
    println!("{ticks} chunk batches");
    for (kind, count) in entity_types {
        println!("entity_type {kind} {count}");
    }
    for (name, (n, errors)) in counts {
        println!(
            "{name:32} {n:6} {}",
            if errors > 0 {
                format!("{errors} ERRORS")
            } else {
                String::new()
            }
        );
    }
    if decode_errors > 0 {
        return Err(format!("{decode_errors} packet/chunk decode failures").into());
    }
    if suggestions && !suggestion_received {
        return Err("command suggestion request had no validated tooltip response".into());
    }
    Ok(())
}
