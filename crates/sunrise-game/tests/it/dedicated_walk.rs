//! The dedicated engine against a headless protocol client, with full or server-only resources.
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc::Receiver,
    thread,
    time::{Duration, Instant},
};
use sunrise_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login,
        play::{self, Clientbound as C, Serverbound as S},
    },
};

/// Reap the server on assertion failures as well as successful test completion.
pub(crate) struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn resources() -> Option<PathBuf> {
    let root = std::env::var_os("SUNRISE_TEST_RESOURCES").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        },
        PathBuf::from,
    );
    root.join("items.json").exists().then_some(root)
}

/// Log in offline and finish configuration as a vanilla client that knows the core pack.
pub(crate) fn join(port: u16, name: &str) -> sunrise_protocol::Result<Connection> {
    let mut c = configure(port, name)?;
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())?;
    Ok(c)
}

/// Stop at the server's FinishConfiguration, before acknowledging it.
pub(crate) fn configure(port: u16, name: &str) -> sunrise_protocol::Result<Connection> {
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5))?;
    c.reader.set_read_timeout(Some(Duration::from_secs(60)))?;
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: offline_uuid(name),
        })
        .encode(),
    )?;
    loop {
        let (id, payload) = c.recv()?;
        match login::Clientbound::decode(id, &payload)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            other => return Err(sunrise_protocol::Error::new(format!("{other:?}"))),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())?;
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(3)).encode())?;
    loop {
        let (id, payload) = c.recv()?;
        match cfg::Clientbound::decode(id, &payload)? {
            cfg::Clientbound::SelectKnownPacks(p) => c.send(
                &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: p.packs })
                    .encode(),
            )?,
            cfg::Clientbound::KeepAlive(p) => c.send(&cfg::Serverbound::KeepAlive(p).encode())?,
            cfg::Clientbound::FinishConfiguration(_) => return Ok(c),
            _ => {}
        }
    }
}

/// Start a dedicated process without touching real player state. Keep both log streams drained.
fn start_server(resources: &Path, game: &Path, scratch: &Path) -> (Server, Receiver<String>) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_sunrise-client"));
    command
        .args(["--resources".as_ref(), resources.as_os_str()])
        .args(["--dedicated", "--nogui"])
        .current_dir(game)
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, folder) in [
        ("SUNRISE_HOME", "home"),
        ("SUNRISE_LAUNCHER_HOME", "launcher"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
    ] {
        command.env(key, scratch.join(folder));
    }
    let mut server = Server(command.spawn().unwrap());
    let (lines, read) = std::sync::mpsc::channel::<String>();
    for stream in [
        Box::new(server.0.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(server.0.stderr.take().unwrap()),
    ] {
        let lines = lines.clone();
        thread::spawn(move || {
            for line in std::io::BufReader::new(stream)
                .lines()
                .map_while(Result::ok)
            {
                let _ = lines.send(line);
            }
        });
    }
    drop(lines);
    (server, read)
}

fn wait_for_log(
    lines: &Receiver<String>,
    log: &mut Vec<String>,
    timeout: Duration,
    expected: &str,
) {
    let start = Instant::now();
    loop {
        let line = lines
            .recv_timeout(timeout.saturating_sub(start.elapsed()))
            .unwrap_or_else(|_| panic!("Server did not report {expected:?}\n{}", log.join("\n")));
        let matched = line.contains(expected);
        log.push(line);
        if matched {
            return;
        }
    }
}

/// Start the dedicated server in `scratch` and return it once it logs `Done (`: the process, its
/// port, the log lines still to read, those read so far and its console.
pub(crate) fn boot(
    scratch: &Path,
    resources: &Path,
) -> (
    Server,
    u16,
    std::sync::mpsc::Receiver<String>,
    Vec<String>,
    std::process::ChildStdin,
) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let game = scratch.join("server");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(
        game.join("server.properties"),
        format!(
            "server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\nwhite-list=false\nlevel-seed=12345\n\
             view-distance=3\nsimulation-distance=3\ngamemode=creative\nforce-gamemode=true\n\
             spawn-protection=0\npause-when-empty-seconds=-1\n"
        ),
    )
    .unwrap();
    std::fs::write(game.join("eula.txt"), "eula=true\n").unwrap();
    let (mut server, read) = start_server(resources, &game, scratch);
    let mut log = Vec::new();
    let started = Instant::now();
    while !log.iter().any(|line: &String| line.contains("Done (")) {
        let line = read
            .recv_timeout(Duration::from_secs(300).saturating_sub(started.elapsed()))
            .unwrap_or_else(|_| panic!("the server never finished starting:\n{}", log.join("\n")));
        log.push(line);
    }
    let console = server.0.stdin.take().unwrap();
    (server, port, read, log, console)
}

/// The dedicated server keeps running while a player walks through freshly generated terrain and
/// sand falls and lands near it:
/// the whole level tick (entities, falling blocks, the entity residency pass) runs every tick
/// against real worldgen, not a flat test world. #2845: a pass left unbalanced stopped the
/// server about 30 level ticks after a walking bot joined.
#[test]
#[ignore = "daily"]
fn a_dedicated_server_keeps_ticking_while_a_player_walks() {
    let Some(resources) = resources() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let (mut server, port, read, mut log, mut console) = boot(scratch.path(), &resources);
    let walked = (|| {
        let mut bot = join(port, "Walker").map_err(|e| e.to_string())?;
        bot.reader
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|e| e.to_string())?;
        // A failed send shows up as the next read's lost connection.
        let send = |bot: &mut Connection, p: S| {
            let _ = bot.send(&p.encode());
        };
        let mut at: Option<[f64; 3]> = None;
        let mut moves = 0;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let tick = Instant::now();
            loop {
                match bot.recv() {
                    Ok((id, payload)) => match C::decode(id, &payload) {
                        Ok(C::PlayerPosition(p)) => {
                            send(
                                &mut bot,
                                S::AcceptTeleportation(play::AcceptTeleportation {
                                    id: p.teleport,
                                    x: p.position[0],
                                    y: p.position[1],
                                    z: p.position[2],
                                    yaw: 0.0,
                                    pitch: 0.0,
                                }),
                            );
                            if at.is_none() {
                                send(&mut bot, S::PlayerLoaded(Empty));
                            }
                            at = Some(p.position);
                        }
                        Ok(C::KeepAlive(k)) => send(&mut bot, S::KeepAlive(k)),
                        Ok(C::ChunkBatchFinished(_)) => send(
                            &mut bot,
                            S::ChunkBatchReceived(play::ChunkBatchReceived {
                                chunks_per_tick: 20.0,
                            }),
                        ),
                        Ok(C::Disconnect(d)) => return Err(format!("disconnected: {d:?}")),
                        _ => {}
                    },
                    Err(e) if e.is_timed_out() => {}
                    Err(e) => return Err(format!("connection lost after {moves} moves: {e}")),
                }
                if tick.elapsed() >= Duration::from_millis(50) {
                    break;
                }
            }
            // Once the player is in, the console summons six falling sand blocks just behind it; each
            // falls and lands, as the gravity blocks the player disturbed did in #2845.
            if moves == 20
                && let Some([x, y, z]) = at
            {
                let (x, y, z) = (x.floor() as i32 - 4, y.floor() as i32 + 6, z.floor() as i32);
                for (dx, dz) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)] {
                    writeln!(
                        console,
                        "summon falling_block {} {y} {} {{BlockState:{{Name:\"minecraft:sand\"}},Time:1}}",
                        x + dx,
                        z + dz
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            // Fly east at sprint speed, two blocks above the spawn: new chunks keep entering the view.
            if let Some([x, y, z]) = at.as_mut().map(|p| {
                p[0] += 0.28;
                *p
            }) {
                send(
                    &mut bot,
                    S::MovePlayerPos(play::MovePlayerPos {
                        x,
                        y: y + 2.0,
                        z,
                        flags: 0,
                    }),
                );
                send(&mut bot, S::ClientTickEnd(Empty));
                moves += 1;
            }
        }
        if moves < 200 {
            return Err(format!("only {moves} moves"));
        }
        Ok(())
    })();
    let exited = server.0.try_wait().unwrap();
    let _ = server.0.kill();
    let _ = server.0.wait();
    log.extend(read.try_iter());
    let unfinished = log
        .iter()
        .any(|line| line.contains("residency") && line.contains("did not finish"));
    assert!(
        walked.is_ok() && exited.is_none() && !unfinished,
        "walk: {walked:?}, server exit: {exited:?}\n{}",
        log.join("\n")
    );
}

/// Receive a play packet while giving vanilla's ordinary acknowledgements. A client waiting for
/// a teleport or use receipt must still keep chunks and keep-alives moving.
fn play_until<T>(
    bot: &mut Connection,
    description: &str,
    mut selected: impl FnMut(C) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "Profile client did not receive {description}"
        );
        bot.reader.set_read_timeout(Some(left)).unwrap();
        let (id, payload) = bot
            .recv()
            .unwrap_or_else(|e| panic!("Waiting for {description}: {e}"));
        match C::decode(id, &payload) {
            Ok(C::KeepAlive(keep)) => bot.send(&S::KeepAlive(keep).encode()).unwrap(),
            Ok(C::ChunkBatchFinished(_)) => bot
                .send(
                    &S::ChunkBatchReceived(play::ChunkBatchReceived {
                        chunks_per_tick: 20.0,
                    })
                    .encode(),
                )
                .unwrap(),
            Ok(C::Disconnect(reason)) => panic!("Profile client disconnected: {reason:?}"),
            Ok(packet) => {
                if let Some(matched) = selected(packet) {
                    return matched;
                }
            }
            // The bot doesn't need presentation packets it can't decode.
            Err(_) => {}
        }
    }
}

fn wait_position(bot: &mut Connection) -> [f64; 3] {
    let p = play_until(bot, "an absolute player position", |packet| match packet {
        C::PlayerPosition(p) => Some(p),
        _ => None,
    });
    assert_eq!(p.relatives & 7, 0, "expected an absolute teleport");
    let [x, y, z] = p.position;
    bot.send(
        &S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x,
            y,
            z,
            yaw: p.yaw,
            pitch: p.pitch,
        })
        .encode(),
    )
    .unwrap();
    p.position
}

fn stop_profile_server(mut server: Server, lines: Receiver<String>, log: &mut Vec<String>) {
    writeln!(server.0.stdin.as_mut().unwrap(), "stop").unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => log.push(line),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("Profile server did not stop cleanly")
            }
        }
    }
    assert!(
        server.0.wait().unwrap().success(),
        "Server failed during stop\n{}",
        log.join("\n")
    );
}

#[test]
#[ignore = "daily"]
fn a_server_profile_can_reload_and_reopen_a_world_with_a_placed_boat() {
    let resources = PathBuf::from(
        std::env::var_os("SUNRISE_TEST_SERVER_PROFILE")
            .expect("daily must prepare the server profile with sunrise-prepare --server"),
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(resources.join("sunrise-resources.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "server");
    assert!(!resources.join("assets/minecraft/textures").exists());
    let scratch = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let game = scratch.path().join("server");
    let pack = game.join("world/datapacks/profile-smoke");
    let function = pack.join("data/profile/function/probe.mcfunction");
    std::fs::create_dir_all(function.parent().unwrap()).unwrap();
    let (major, minor) = sunrise_core::version::DATA_PACK_FORMAT;
    std::fs::write(pack.join("pack.mcmeta"), format!(r#"{{"pack":{{"description":"Server profile smoke test","min_format":[{major},{minor}],"max_format":[{major},{minor}]}}}}"#)).unwrap();
    std::fs::write(&function, "say profile_before_reload\n").unwrap();
    std::fs::write(
        game.join("world/datapacks.json"),
        r#"{"enabled":["vanilla","file/profile-smoke"],"disabled":[]}"#,
    )
    .unwrap();
    std::fs::write(game.join("server.properties"), format!(
        "server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\nwhite-list=false\nlevel-seed=12345\n\
         view-distance=3\nsimulation-distance=3\ngamemode=creative\nforce-gamemode=true\n\
         spawn-protection=0\npause-when-empty-seconds=-1\nlevel-type=minecraft:flat\n"
    )).unwrap();
    std::fs::write(game.join("eula.txt"), "eula=true\n").unwrap();
    let (mut server, lines) = start_server(&resources, &game, scratch.path());
    let mut log = Vec::new();
    let ready = Duration::from_secs(300);
    let command = Duration::from_secs(45);
    wait_for_log(&lines, &mut log, ready, "Done (");
    let mut bot = join(port, "ProfileWalker").unwrap();
    let [spawn_x, spawn_y, spawn_z] = wait_position(&mut bot);
    bot.send(&S::PlayerLoaded(Empty).encode()).unwrap();
    wait_for_log(&lines, &mut log, command, "ProfileWalker joined the game");
    let (x, y, z) = (
        spawn_x.floor() as i32,
        spawn_y.floor() as i32 + 8,
        spawn_z.floor() as i32,
    );
    let water_floor = y - 1;
    let [start_x, feet, start_z] = [f64::from(x) + 0.5, f64::from(y) + 2.0, f64::from(z) + 0.5];
    let console = server.0.stdin.as_mut().unwrap();
    writeln!(console, "function profile:probe").unwrap();
    wait_for_log(&lines, &mut log, command, "profile_before_reload");
    writeln!(
        console,
        "fill {} {water_floor} {} {} {water_floor} {} minecraft:stone",
        x - 2,
        z - 2,
        x + 3,
        z + 2
    )
    .unwrap();
    writeln!(
        console,
        "fill {} {y} {} {} {y} {} minecraft:water",
        x - 2,
        z - 2,
        x + 3,
        z + 2
    )
    .unwrap();
    writeln!(console, "forceload add {x} {z}").unwrap();
    writeln!(
        console,
        "item replace entity ProfileWalker weapon.mainhand with minecraft:oak_boat"
    )
    .unwrap();
    writeln!(console, "tp ProfileWalker {start_x} {feet} {start_z} 0 90").unwrap();
    assert_eq!(wait_position(&mut bot), [start_x, feet, start_z]);
    let moved_x = start_x + 1.0;
    bot.send(
        &S::MovePlayerPos(play::MovePlayerPos {
            x: moved_x,
            y: feet,
            z: start_z,
            flags: 0,
        })
        .encode(),
    )
    .unwrap();
    bot.send(&S::ClientTickEnd(Empty).encode()).unwrap();
    // Check server-observed movement, not just the packet the client sent.
    let moved = format!("@a[name=ProfileWalker,x={moved_x},y={feet},z={start_z},distance=..0.25]");
    // BoatItem.use uses this look ray to find the water. Do not summon the boat by command.
    bot.send(
        &S::UseItem(play::UseItem {
            hand: 0,
            sequence: 1,
            yaw: 0.0,
            pitch: 90.0,
        })
        .encode(),
    )
    .unwrap();
    play_until(&mut bot, "the boat-use receipt", |packet| {
        matches!(packet, C::BlockChangedAck(ack) if ack.sequence == 1).then_some(())
    });
    // An acknowledgement establishes that the preceding movement packet was handled too. A
    // console request issued before the networking tick can race it, so query after the ack.
    writeln!(
        console,
        "execute if entity {moved} run say profile_player_moved"
    )
    .unwrap();
    wait_for_log(&lines, &mut log, command, "profile_player_moved");
    writeln!(
        console,
        "tag @e[type=minecraft:oak_boat,limit=1] add profile_smoke_boat"
    )
    .unwrap();
    let boat = "@e[type=minecraft:oak_boat,tag=profile_smoke_boat,limit=1]";
    writeln!(
        console,
        "execute if entity {boat} run say profile_boat_present"
    )
    .unwrap();
    wait_for_log(&lines, &mut log, command, "profile_boat_present");
    std::fs::write(function, "say profile_after_reload\n").unwrap();
    writeln!(console, "reload\nfunction profile:probe").unwrap();
    wait_for_log(&lines, &mut log, command, "profile_after_reload");
    writeln!(console, "save-all flush").unwrap();
    wait_for_log(&lines, &mut log, command, "Saved the game");
    drop(bot);
    stop_profile_server(server, lines, &mut log);

    let (mut reopened, lines) = start_server(&resources, &game, scratch.path());
    let mut log = Vec::new();
    wait_for_log(&lines, &mut log, ready, "Done (");
    let mut bot = join(port, "ProfileWalker").unwrap();
    wait_position(&mut bot);
    bot.send(&S::PlayerLoaded(Empty).encode()).unwrap();
    wait_for_log(&lines, &mut log, command, "ProfileWalker joined the game");
    writeln!(
        reopened.0.stdin.as_mut().unwrap(),
        "execute if entity {boat} run say profile_boat_reopened"
    )
    .unwrap();
    wait_for_log(&lines, &mut log, command, "profile_boat_reopened");
    drop(bot);
    stop_profile_server(reopened, lines, &mut log);
}
