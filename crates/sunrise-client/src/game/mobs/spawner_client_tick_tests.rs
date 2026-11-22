//! `ClientLevel.tickBlockEntities` for spawners, trial spawners and vaults on a server's client,
//! case by case against vanilla 26.3 (tools/test_block_entity_client_tick_parity.py): each case's
//! columns arrive in chunk packets (`remote_chunk`), are forgotten and sent again as the server's
//! packets do, take level events 3011 to 3021 (`level_event_at`) and tick
//! (`tick_client_block_entities`). Each tick logs one ordered stream of the particles and local
//! sounds (with the seed each sound's instance takes), each block entity's spin and the level
//! random's state.
use super::*;
use std::sync::Arc;
use sunrise_client::chunk::block_entity::{Next, SpawnData, Spawner, TrialSpawner, Vault};
use sunrise_core::nbt::Tag;

type Pos = (i32, i32, i32);
use sunrise_protocol::chunk::{BlockEntityInfo, ChunkData, LightData};
use sunrise_world::registry::BlockStateId;

/// The committed cases and the SHA-256 of the log vanilla made of each (`--write` of the harness
/// makes them).
const COMMITTED: &str = include_str!("../../../../../docs/parity/block-entity-client-ticks.jsonl");
/// The packets' block entity type ids: their index here.
const TYPES: [&str; 3] = [
    "minecraft:mob_spawner",
    "minecraft:trial_spawner",
    "minecraft:vault",
];
const PLAYER: [i32; 4] = [0x5355_4e52, 0x4953_4500, 0, 1];

/// The harness gives its cases (`SUNRISE_CLIENT_TICK_CASES`) and takes the logs
/// (`SUNRISE_CLIENT_TICK_OUT`) to compare them with vanilla's. Without them (the daily run) the
/// committed cases run, and each log must hash to vanilla's.
#[test]
#[ignore = "daily"]
fn client_ticks_match_the_vanilla_cases() {
    let harness = std::env::var("SUNRISE_CLIENT_TICK_CASES")
        .ok()
        .zip(std::env::var("SUNRISE_CLIENT_TICK_OUT").ok());
    let committed: Vec<serde_json::Value> = COMMITTED
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let cases: Vec<serde_json::Value> = match &harness {
        Some((cases, _)) => std::fs::read_to_string(cases)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
        None => committed.iter().map(|row| row["case"].clone()).collect(),
    };
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.settings.render_distance = 32;
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "ClientTick").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        Arc::make_mut(&mut remote.builtin).registries.insert(
            "minecraft:block_entity_type".into(),
            sunrise_protocol::registry::Ids::from_names(TYPES.map(str::to_owned).to_vec()),
        );
        let mut reordered = false;
        let logs: Vec<String> = cases
            .iter()
            .map(|case| client_tick_case(game, case, &mut reordered).to_string())
            .collect();
        match harness {
            Some((_, out)) => std::fs::write(
                out,
                logs.iter()
                    .map(|log| format!("{log}\n"))
                    .collect::<String>(),
            )
            .unwrap(),
            None => {
                use sha2::Digest;
                let differ: Vec<&str> = committed
                    .iter()
                    .zip(&logs)
                    .filter(|(row, log)| {
                        format!("{:x}", sha2::Sha256::digest(log.as_bytes())) != row["vanilla"]
                    })
                    .map(|(row, _)| row["case"]["name"].as_str().unwrap())
                    .collect();
                assert!(
                    differ.is_empty(),
                    "these cases tick unlike vanilla 26.3 (tools/test_block_entity_client_tick_parity.py \
                     shows where): {differ:?}"
                );
                // A column whose packet lists its block entities in another order than its block
                // entity map walks them: the tickers follow the packet.
                assert!(reordered, "no case sends a column out of its map's order");
            }
        }
    });
}

fn hex_f64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

fn hex_f32(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

fn int(v: &serde_json::Value) -> i32 {
    v.as_i64().unwrap() as i32
}

fn key_of(e: &serde_json::Value) -> Pos {
    (int(&e["pos"][0]), int(&e["pos"][1]), int(&e["pos"][2]))
}

fn column(k: Pos) -> (i32, i32) {
    (k.0 >> 4, k.2 >> 4)
}

/// One case's block entity as the server holds it, with its block state and packet type id.
fn client_tick_entity(game: &Game<'_>, e: &serde_json::Value) -> (BlockStateId, i32, BlockEntity) {
    let text = |k: &str| e[k].as_str().unwrap().to_owned();
    let flag = |k: &str| e[k].as_bool().unwrap().to_string();
    let (name, kind, properties, entity) = match e["kind"].as_str().unwrap() {
        "spawner" => {
            let mut mob = sunrise_core::nbt::Compound::new();
            mob.insert("id".into(), Tag::String("minecraft:pig".into()));
            let spawner = Spawner {
                required_player_range: int(&e["range"]),
                spawn_delay: int(&e["delay"]),
                next: Some(Next::Own(SpawnData {
                    entity: mob,
                    ..Default::default()
                })),
                ..Spawner::default()
            };
            (
                "minecraft:spawner",
                0,
                vec![],
                BlockEntity::Spawner(Box::new(spawner)),
            )
        }
        "trial_spawner" => (
            "minecraft:trial_spawner",
            1,
            vec![
                ("trial_spawner_state", text("state")),
                ("ominous", flag("ominous")),
            ],
            BlockEntity::TrialSpawner(Box::new(TrialSpawner {
                next_mob_spawns_at: e["nextMobSpawnsAt"].as_i64().unwrap(),
                ..Default::default()
            })),
        ),
        _ => (
            "minecraft:vault",
            2,
            vec![
                ("facing", text("facing")),
                ("ominous", flag("ominous")),
                ("vault_state", text("state")),
            ],
            BlockEntity::Vault(Box::new(Vault {
                display_item: e["display"]
                    .as_bool()
                    .unwrap()
                    .then(|| sunrise_client::items::ItemStack::new("minecraft:diamond", 1)),
                connected_players: if e["connected"].as_bool().unwrap() {
                    vec![PLAYER]
                } else {
                    Vec::new()
                },
                connected_particles_range: e["particlesRange"].as_f64().unwrap(),
                ..Default::default()
            })),
        ),
    };
    let registry = game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state(name).unwrap())
        .unwrap()
        .clone();
    for (k, v) in properties {
        state.properties.insert(k.into(), v);
    }
    (registry.id(&state).unwrap(), kind, entity)
}

/// `ChunkMap.playerLoadedChunk`'s packet for column `at`: its blocks, and the block entities
/// `listed` (indices into the case's) in that order, each with its update tag
/// (`BlockEntity.getUpdateTag`, as a server sends it). Whether that order differs from the order
/// the client's block entity map walks them.
fn send_column(
    game: &mut Game<'_>,
    entities: &[(Pos, BlockStateId, i32, BlockEntity)],
    at: (i32, i32),
    listed: &[usize],
) -> bool {
    let mut chunk = sunrise_client::chunk::Chunk::empty(at.0, at.1);
    let mut infos = Vec::new();
    for &i in listed {
        let (k, state, kind, entity) = &entities[i];
        assert_eq!(column(*k), at, "entity {i} is not in column {at:?}");
        chunk.set(k.0 & 15, k.1, k.2 & 15, state.0);
        // Only a trial spawner's state has this property.
        let active = game
            .pack
            .assets
            .registry
            .state(*state)
            .unwrap()
            .properties
            .get("trial_spawner_state")
            .is_some_and(|v| v.as_str() == "active");
        infos.push(BlockEntityInfo {
            packed_xz: (((k.0 & 15) << 4) | (k.2 & 15)) as u8 as i8,
            y: k.1 as i16,
            kind: *kind,
            tag: sunrise_client::lan::chunks::entity_tag(entity, active, game.res.items())
                .unwrap_or(Tag::End),
        });
    }
    let packet = sunrise_protocol::packets::play::LevelChunkWithLight {
        x: at.0,
        z: at.1,
        chunk: ChunkData {
            heightmaps: Vec::new(),
            buffer: Vec::new(),
            block_entities: infos,
        },
        light: LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
        },
    };
    let format = game.remote.as_ref().unwrap().chunk_format.clone();
    game.remote_chunk(&packet, Some((&format, Ok(chunk))));
    let order: Vec<Pos> = listed.iter().map(|&i| entities[i].0).collect();
    sunrise_client::block_entities::chunk_map_order(order.iter().copied()) != order
}

/// The particle and sound requests captured since the tick began, as one stream in the order
/// they were made: a sound sits after the particles requested before it
/// (`CapturedSound::particles_before`).
fn event_stream(game: &mut Game<'_>, particles_at_start: u64) -> Vec<serde_json::Value> {
    let particles = game.particles.requested.replace(Vec::new()).unwrap();
    let sounds = game.audio.capture.replace(Vec::new()).unwrap();
    let mut out = Vec::new();
    let mut particles = particles.into_iter();
    let mut taken = 0;
    let particle_row = |(o, p, v): (
        sunrise_client::particles::ParticleOptions,
        [f64; 3],
        [f64; 3],
    )| {
        let mut row = vec![
            serde_json::json!("particle"),
            serde_json::json!(format!("minecraft:{}", o.ty.name())),
        ];
        row.extend(p.iter().chain(&v).map(|d| serde_json::json!(hex_f64(*d))));
        serde_json::Value::Array(row)
    };
    for s in sounds {
        while taken < s.particles_before - particles_at_start {
            out.push(particle_row(particles.next().unwrap()));
            taken += 1;
        }
        let mut row = vec![serde_json::json!("sound"), serde_json::json!(s.event)];
        row.extend(s.position.iter().map(|d| serde_json::json!(hex_f64(*d))));
        row.push(serde_json::json!(hex_f32(s.volume)));
        row.push(serde_json::json!(hex_f32(s.pitch)));
        row.push(serde_json::json!(format!("{:x}", s.seed)));
        out.push(serde_json::Value::Array(row));
    }
    out.extend(particles.map(particle_row));
    out
}

fn client_tick_case(
    game: &mut Game<'_>,
    case: &serde_json::Value,
    reordered: &mut bool,
) -> serde_json::Value {
    game.forget_all_remote_columns();
    let entities: Vec<_> = case["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let (state, kind, entity) = client_tick_entity(game, e);
            (key_of(e), state, kind, entity)
        })
        .collect();
    let p = &case["player"];
    let at = |i: usize| p["pos"][i].as_f64().unwrap();
    let mut player =
        sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(at(0), at(1), at(2)))
            .unwrap();
    player.health.dead = p["dead"].as_bool().unwrap();
    game.player = Some(player);
    game.mode = if p["spectator"].as_bool().unwrap() {
        sunrise_client::command::args::GameMode::Spectator
    } else {
        sunrise_client::command::args::GameMode::Survival
    };
    game.level.player_uuid = Some(PLAYER);
    game.particles
        .seed_randoms(0, case["seed"].as_i64().unwrap(), 0);
    game.world.game_time = case["gameTime"].as_i64().unwrap();
    // The columns arrive in the order their first block entity is listed, each listing its
    // block entities in the case's order.
    let mut columns: Vec<((i32, i32), Vec<usize>)> = Vec::new();
    for (i, (k, ..)) in entities.iter().enumerate() {
        match columns.iter_mut().find(|(c, _)| *c == column(*k)) {
            Some((_, listed)) => listed.push(i),
            None => columns.push((column(*k), vec![i])),
        }
    }
    for (at, listed) in &columns {
        *reordered |= send_column(game, &entities, *at, listed);
    }
    game.particles.requested = Some(Vec::new());
    game.audio.capture = Some(Vec::new());
    let mut ticks = Vec::new();
    for t in 0..case["ticks"].as_i64().unwrap() {
        let particles_at_start = sunrise_client::particles::requested_count();
        // The packets handled before the tick.
        for op in case["events"][t.to_string()]
            .as_array()
            .into_iter()
            .flatten()
        {
            match op[0].as_str().unwrap() {
                "forget" => {
                    let at = (int(&op[1]), int(&op[2]));
                    game.forget_remote_columns(&mut [at].into_iter().collect(), true);
                }
                "send" => {
                    let listed: Vec<usize> = op[3]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| i.as_u64().unwrap() as usize)
                        .collect();
                    *reordered |= send_column(game, &entities, (int(&op[1]), int(&op[2])), &listed);
                }
                "event" => {
                    let pos = [int(&op[2][0]), int(&op[2][1]), int(&op[2][2])];
                    game.level_event_at(int(&op[1]), pos, int(&op[3]));
                }
                other => panic!("unknown case event {other}"),
            }
        }
        game.tick_client_block_entities();
        let events = event_stream(game, particles_at_start);
        // Vec3i.compareTo: y, then z, then x.
        let mut spins: Vec<_> = (&game.world.block_entities)
            .into_iter()
            .filter_map(|(k, e)| {
                let (spin, o_spin) = match e {
                    BlockEntity::Spawner(s) => (hex_f64(s.spin), hex_f64(s.o_spin)),
                    BlockEntity::TrialSpawner(s) => (hex_f64(s.spin), hex_f64(s.o_spin)),
                    BlockEntity::Vault(v) => (hex_f32(v.spin), hex_f32(v.o_spin)),
                    _ => return None,
                };
                Some((
                    (k.1, k.2, k.0),
                    serde_json::json!([k.0, k.1, k.2, spin, o_spin]),
                ))
            })
            .collect();
        spins.sort_by_key(|(order, _)| *order);
        // Keys in their sorted order, as the harness hashes vanilla's log.
        ticks.push(serde_json::json!({
            "events": events,
            "random": format!("{:x}", game.particles.level_random().state()),
            "spins": spins.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
        }));
        game.world.game_time += 1;
    }
    game.particles.requested = None;
    game.audio.capture = None;
    serde_json::json!({ "ticks": ticks })
}
