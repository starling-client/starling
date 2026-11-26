//! `block_entity_client_tick` (#3611), through a client's real frames (`client_tick_steps` after
//! the frame's `update_world` drain): a client's copies of a type that declared client ticks are
//! told once per client tick each, between `client_tick_pre`, the level's pre and its post, and
//! `client_tick_post`, in the ticker list's order, with the data the frame's drain merged; not
//! while paused or frozen; a type that didn't declare them never; a copy removed or whose column
//! the server forgets not again; and a server, the world's own ticks, never. A spawner between
//! them in the ticker list ticks between them, from the same list.
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use crate::game::world_loading::WorldRequest;
use mod_api::{
    BlockEntityClientTickEvent, BlockEntityDataReceivedEvent, BlockEntityPart, BlockPart,
    BlocksEvent, ClientHost, ClientTickPostEvent, ClientTickPreEvent, Hooks, HostSide,
    LevelTickPostEvent, LevelTickPreEvent, Registrar, RegistrationContext,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sunrise_client::containers::BlockEntity;
use sunrise_core::nbt::{Compound, Tag};

const HUM: &str = "client_tick_probe:hum";
const QUIET: &str = "client_tick_probe:quiet";
/// A mod block named as vanilla's shulker boxes are: its ticker is its own type's.
const CRATE: &str = "client_tick_probe:crate_shulker_box";
/// Chunk (100, 100): far from the spawn.
const CHUNK: (i32, i32) = (100, 100);
const HUMS: [[i32; 3]; 2] = [[1610, 70, 1613], [1601, 64, 1602]];
const QUIET_AT: [i32; 3] = [1605, 64, 1605];
/// A hum in the next column, which stays.
const OTHER: [i32; 3] = [1620, 64, 1600];
const CRATE_AT: [i32; 3] = [1621, 64, 1600];
/// A spawner that arrives after the second hum and before the first.
const SPAWNER_AT: [i32; 3] = [1603, 64, 1608];

/// What the probe heard, in order: the client tick's and level's brackets, each client tick
/// (`tick`, its position and `rpm`) and each synced data notice (`data`).
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// How many particles the game had requested at each client tick told
/// (`sunrise_client::particles::requested_count`), in order.
static TICK_PARTICLES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

fn note(event: String) {
    EVENTS.lock().unwrap().push(event);
}

fn take_events() -> Vec<String> {
    std::mem::take(&mut *EVENTS.lock().unwrap())
}

fn rpm_of(nbt: &Compound) -> i32 {
    match nbt.get(&"rpm".into()) {
        Some(Tag::Int(rpm)) => *rpm,
        _ => -1,
    }
}

/// A client tick's event, once its scoped view is checked to read its position and its state to
/// be its block's.
fn record(event: &mut BlockEntityClientTickEvent<'_>) {
    let (handle, pos, block, id, rpm) = event
        .subject
        .with_view(|view| {
            (
                view.handle(),
                view.position(),
                view.state().block(),
                view.id().to_owned(),
                rpm_of(view.nbt()),
            )
        })
        .expect("live resident client block entity");
    let host = event.subject.host();
    assert_eq!(handle.position(host), Some(pos));
    // The block and its block entity type share the name here.
    assert_eq!(block.id(host), Some(id.as_str()));
    TICK_PARTICLES
        .lock()
        .unwrap()
        .push(sunrise_client::particles::requested_count());
    note(format!("tick {pos:?} {rpm}"));
}

/// Declares client ticks.
struct Hum;
#[mod_api::sunrise_part]
impl BlockPart for Hum {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Hum {
    fn block_entity_client_tick(&self, event: &mut BlockEntityClientTickEvent<'_>) {
        record(event);
    }
    fn block_entity_data_received(&self, event: &mut BlockEntityDataReceivedEvent<'_>) {
        let (pos, rpm) = event
            .subject
            .with_view(|view| (view.position(), rpm_of(view.nbt())))
            .expect("live resident client block entity");
        note(format!("data {pos:?} {rpm}"));
    }
}

/// Has a client tick, but the selector rejects its client states.
struct Quiet;
#[mod_api::sunrise_part]
impl BlockPart for Quiet {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Quiet {
    fn block_entity_client_tick(&self, event: &mut BlockEntityClientTickEvent<'_>) {
        record(event);
    }
}

struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let hum = r.block_entity_type("hum").blocks(&[HUM]).build();
        r.block_entity_part(&hum, Hum).unwrap();
        let quiet = r
            .block_entity_type("quiet")
            .blocks(&[QUIET])
            .ticker_when(|_, _, side| side == mod_api::LogicalSide::Server)
            .build();
        r.block_entity_part(&quiet, Quiet).unwrap();
        let boxed = r
            .block_entity_type("crate_shulker_box")
            .blocks(&[CRATE])
            .build();
        r.block_entity_part(&boxed, Hum).unwrap();
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for block in [HUM, QUIET, CRATE] {
            event
                .register(mod_api::Block::new(block, "minecraft:stone"))
                .unwrap();
        }
    }
    fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        note("client_pre".into());
    }
    fn client_tick_post(&self, _: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        note("client_post".into());
    }
    fn level_tick_pre(&self, host: HostSide<'_>, _: &mut LevelTickPreEvent) {
        if let HostSide::Client(_) = host {
            note("level_pre".into());
        }
    }
    fn level_tick_post(&self, host: HostSide<'_>, _: &mut LevelTickPostEvent) {
        if let HostSide::Client(_) = host {
            note("level_post".into());
        }
    }
}

fn install() {
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("client_tick_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
}

/// The prepared resources with the installed mods' content.
fn prepared_root() -> PathBuf {
    let base = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf()
}

fn rpm(rpm: i32) -> Compound {
    let mut nbt = Compound::new();
    nbt.insert("rpm".into(), Tag::Int(rpm));
    nbt
}

/// One frame: what the server sent is handled (`update_world`, which a frame runs before its
/// ticks), then `steps` client ticks. Returns what the probe heard.
fn frame(game: &mut crate::game::Game<'_>, steps: u32) -> Vec<String> {
    game.update_world();
    game.client_tick_steps(steps, None).unwrap();
    take_events()
}

/// The events of one client tick that ticks the level, its client ticks being `ticks`.
fn ticked(ticks: &[([i32; 3], i32)]) -> Vec<String> {
    let mut events = vec!["client_pre".to_owned(), "level_pre".to_owned()];
    events.extend(ticks.iter().map(|(pos, rpm)| format!("tick {pos:?} {rpm}")));
    events.extend(["level_post".to_owned(), "client_post".to_owned()]);
    events
}

#[test]
#[ignore = "daily"]
fn a_clients_frames_tick_its_declared_copies_until_they_go() {
    use crate::game::execution_role::ExecutionRole;
    use sunrise_client::{
        chunk::Chunk,
        containers::{ModBlockEntity, ModBlockEntityFields},
    };
    use sunrise_protocol::packets::play;
    if run_fresh_cases(
        "game::native_hooks::block_entity_client_tick_tests::a_clients_frames_tick_its_declared_copies_until_they_go",
        "SUNRISE_BLOCK_ENTITY_CLIENT_TICK_CHILD",
        &["game"],
    ) {
        return;
    }
    install();
    let types = [
        "client_tick_probe:hum",
        "client_tick_probe:quiet",
        "client_tick_probe:crate_shulker_box",
    ];
    with_game_at(prepared_root(), |game| {
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;

        // Not connected yet: no client level, nothing told.
        game.tick_client_block_entities();
        assert!(take_events().is_empty(), "told without a client level");

        let (_sent, mut server) = crate::game::remote::tests::connect_menu_test(game);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        Arc::make_mut(&mut remote.builtin).registries.insert(
            "minecraft:block_entity_type".into(),
            sunrise_protocol::registry::Ids::from_names(types.map(str::to_owned).to_vec()),
        );
        for (x, z) in [CHUNK, (101, 100)] {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
        let registry = game.pack.assets.registry;
        let hum = registry.default_state(HUM).unwrap();
        let quiet = registry.default_state(QUIET).unwrap();
        let boxed = registry.default_state(CRATE).unwrap();
        // Arrival order: the second hum, the quiet one, the first hum, the other column's.
        let placed = [
            (HUMS[1], hum, HUM, 10),
            (QUIET_AT, quiet, QUIET, 20),
            (HUMS[0], hum, HUM, 30),
            (OTHER, hum, HUM, 40),
            (CRATE_AT, boxed, CRATE, 50),
        ];
        for (n, ([x, y, z], state, id, speed)) in placed.into_iter().enumerate() {
            game.world.set(x, y, z, state).unwrap();
            let entity = ModBlockEntity::new(
                id.into(),
                ModBlockEntityFields {
                    nbt: rpm(speed),
                    ..Default::default()
                },
            );
            game.world
                .block_entities
                .insert((x, y, z), BlockEntity::Mod(Box::new(entity)));
            if n == 0 {
                // A spawner that sees a player at any range: two particles each client tick.
                let mut pig = Compound::new();
                pig.insert("id".into(), Tag::String("minecraft:pig".into()));
                let spawner = sunrise_client::chunk::block_entity::Spawner {
                    required_player_range: -1,
                    next: Some(sunrise_client::chunk::block_entity::Next::Own(
                        sunrise_client::chunk::block_entity::SpawnData {
                            entity: pig,
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                };
                let [x, y, z] = SPAWNER_AT;
                let state = registry.default_state("minecraft:spawner").unwrap();
                game.world.set(x, y, z, state).unwrap();
                game.world
                    .block_entities
                    .insert((x, y, z), BlockEntity::Spawner(Box::new(spawner)));
            }
        }
        // The player the spawner looks for (`hasNearbyAlivePlayer` needs one, whatever the range).
        let [x, y, z] = SPAWNER_AT.map(f64::from);
        game.player = Some(
            sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(x, y + 2.0, z),
            )
            .unwrap(),
        );
        take_events();

        // Each declared copy once per tick, in arrival order, inside the level's brackets; the
        // quiet type never. The spawner ticks between the second hum and the first, as it arrived:
        // its two particles come between their ticks.
        let all = |first| ticked(&[(HUMS[1], 10), (HUMS[0], first), (OTHER, 40), (CRATE_AT, 50)]);
        game.particles.requested = Some(Vec::new());
        TICK_PARTICLES.lock().unwrap().clear();
        assert_eq!(frame(game, 1), all(30));
        let seen = std::mem::take(&mut *TICK_PARTICLES.lock().unwrap());
        assert_eq!([seen[1] - seen[0], seen[2] - seen[1]], [2, 0], "{seen:?}");
        game.particles.requested = None;
        assert_eq!(frame(game, 2), [all(30), all(30)].concat());

        // The server's update reaches the next frame's drain: the notice comes first, and both
        // of that frame's ticks see the merged data directly from the resident block entity.
        server
            .send(
                &play::Clientbound::BlockEntityData(play::BlockEntityData {
                    pos: HUMS[0],
                    kind: 0,
                    tag: Tag::Compound(rpm(31)),
                })
                .encode(),
            )
            .unwrap();
        // The frame whose drain (`update_world`) handles it, once the connection has it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while EVENTS.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "the update did not arrive");
            std::thread::sleep(Duration::from_millis(1));
            game.update_world();
        }
        game.client_tick_steps(2, None).unwrap();
        let data = format!("data {:?} 31", HUMS[0]);
        assert_eq!(take_events(), [vec![data], all(31), all(31)].concat());

        // Paused, the client tick runs but not the level; frozen, the level runs but not its
        // block entities.
        game.pause = true;
        assert_eq!(
            frame(game, 1),
            ["client_pre", "client_post"].map(str::to_owned)
        );
        game.pause = false;
        game.clock.set_frozen(true);
        assert_eq!(frame(game, 1), ticked(&[]));
        game.clock.set_frozen(false);

        // A copy removed is not told again; its column's other hum still is.
        let [x, y, z] = HUMS[1];
        game.world.block_entities.remove(&(x, y, z));
        assert_eq!(
            frame(game, 1),
            ticked(&[(HUMS[0], 31), (OTHER, 40), (CRATE_AT, 50)])
        );

        // The server forgets the column: its hums are not told again, the other column's is.
        game.forget_remote_columns(&mut [CHUNK].into_iter().collect(), true);
        for _ in 0..2 {
            assert_eq!(frame(game, 1), ticked(&[(OTHER, 40), (CRATE_AT, 50)]));
        }

        // The level going leaves nothing to tell.
        game.forget_all_remote_columns();
        assert_eq!(frame(game, 1), ticked(&[]));
    });
}

fn request(world: &Path) -> WorldRequest {
    let rules = std::sync::Arc::new(sunrise_client::gamerules::RuleRegistry::vanilla());
    let options = sunrise_client::title::WorldOptions {
        world_type: sunrise_client::save::WorldPreset::Flat,
        rules: sunrise_client::gamerules::GameRules::with_registry(rules.clone(), &[]),
        ..Default::default()
    };
    let mut request = WorldRequest::new(
        "client_tick",
        Some("client_tick"),
        Some(1),
        true,
        Some(&options),
        rules,
    )
    .unwrap();
    request.save_root = world.to_owned();
    request
}

/// A world's own ticks (a server's) never tell a client tick, a declared type's included.
#[test]
#[ignore = "daily"]
fn a_server_never_ticks_a_client_ticker() {
    if run_fresh_cases(
        "game::native_hooks::block_entity_client_tick_tests::a_server_never_ticks_a_client_ticker",
        "SUNRISE_BLOCK_ENTITY_CLIENT_TICK_SERVER_CHILD",
        &["game"],
    ) {
        return;
    }
    install();
    let saves = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("block-entity-client-tick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    with_game_at(prepared_root(), |game| {
        game.load_world(request(&saves)).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 1600 1600"));
        let deadline = Instant::now() + Duration::from_secs(120);
        while !game.world.chunks.contains_key(&CHUNK) || game.streaming.busy() {
            assert!(Instant::now() < deadline, "chunk {CHUNK:?} did not load");
            game.streaming.wait(Duration::from_millis(5));
            game.update_world();
        }
        game.tick_world(true).unwrap();
        for ([x, y, z], block) in [(HUMS[0], HUM), (QUIET_AT, QUIET)] {
            assert!(game.run_command(&format!("setblock {x} {y} {z} {block}")));
        }
        for _ in 0..5 {
            game.tick_world(true).unwrap();
        }
        let [x, y, z] = HUMS[0];
        assert!(
            matches!(
                game.world.block_entities.get(&(x, y, z)),
                Some(BlockEntity::Mod(_))
            ),
            "the hum was placed"
        );
        assert!(
            !take_events().iter().any(|e| e.starts_with("tick")),
            "a server told a client tick"
        );
    });
    let _ = std::fs::remove_dir_all(&saves);
}
