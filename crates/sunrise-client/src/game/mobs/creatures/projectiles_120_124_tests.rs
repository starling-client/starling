//! Witness-driven parity tests for projectile gaps #120-#124 against vanilla 26.3.
use crate::game::{Game, commands::Target};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use sunrise_client::mobs::{Who, leash::Holder};
use sunrise_client::projectiles::{Projectile, ProjectileKind};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};

#[derive(Deserialize)]
struct Witness {
    game_version: String,
    proj_seed: i64,
    level_seed: i64,
    gap_120_spectator: Vec<Case120>,
    gap_121_wind_crystal: Vec<Case121>,
    gap_122_trident_enderman: Vec<Case122>,
    gap_123_piercing: Vec<Case123>,
    gap_124_fireworks: Vec<Case124>,
    review_round1: Vec<ReviewCase>,
}

#[derive(Deserialize)]
struct GameEventSnap {
    id: String,
    pos_bits: [String; 3],
}

#[derive(Deserialize)]
struct Case120 {
    mode: String,
    projectile: String,
    ticks: Vec<Tick120>,
}

#[derive(Deserialize)]
struct Tick120 {
    tick: usize,
    proj_alive: bool,
    proj_pos_bits: [String; 3],
    proj_vel_bits: [String; 3],
    in_ground: bool,
    player_health: f32,
}

#[derive(Deserialize)]
struct Case121 {
    case: String,
    ticks: Vec<Tick121>,
}

#[derive(Deserialize)]
struct Tick121 {
    tick: usize,
    proj_alive: bool,
    proj_pos_bits: [String; 3],
    proj_vel_bits: [String; 3],
    crystal_alive: bool,
    pig_health: f32,
}

#[derive(Deserialize)]
struct Case122 {
    case: String,
    ticks: Vec<Tick122>,
}

#[derive(Deserialize)]
struct Tick122 {
    tick: usize,
    proj_alive: bool,
    proj_pos_bits: [String; 3],
    proj_vel_bits: [String; 3],
    proj_yaw_bits: String,
    dealt_damage: bool,
    target_health: f32,
    target_teleported: bool,
    behind_pig_health: f32,
    game_events: Vec<GameEventSnap>,
    sounds: Vec<String>,
}

#[derive(Deserialize)]
struct Case123 {
    case: String,
    ticks: Vec<Tick123>,
}

#[derive(Deserialize)]
struct Target123 {
    alive: bool,
    health: f32,
}

#[derive(Deserialize)]
struct Tick123 {
    tick: usize,
    proj_alive: bool,
    proj_pos_bits: [String; 3],
    proj_vel_bits: [String; 3],
    proj_yaw_bits: String,
    in_ground: bool,
    targets: Vec<Target123>,
    two_birds_done: bool,
    arbalistic_done: bool,
    game_events: Vec<GameEventSnap>,
    sounds: Vec<String>,
}

#[derive(Deserialize)]
struct Case124 {
    case: String,
    spawn_y: f64,
    ticks: Vec<Tick124>,
}

#[derive(Deserialize)]
struct Tick124 {
    tick: usize,
    rocket_alive: bool,
    rocket_pos_bits: [String; 3],
    rocket_vel_bits: [String; 3],
    targets_alive: BTreeMap<String, bool>,
    drops: Vec<String>,
    game_events: Vec<GameEventSnap>,
    sounds: Vec<String>,
}

#[derive(Deserialize)]
struct ReviewCase {
    case: String,
    ticks: Vec<ReviewTick>,
}

#[derive(Deserialize)]
struct ReviewTick {
    tick: usize,
    player_health: f32,
    lightning_count: usize,
    xp_orb_count: usize,
    target_power: i32,
    sculk_phase: Option<String>,
    entities: BTreeMap<String, ReviewEntitySnap>,
    drops: Vec<String>,
    game_events: Vec<GameEventSnap>,
    sounds: Vec<String>,
}

#[derive(Deserialize)]
struct ReviewEntitySnap {
    present: bool,
    alive: bool,
    pos_bits: [String; 3],
    vel_bits: [String; 3],
    yaw_bits: String,
    fire_ticks: i32,
    ticks_frozen: i32,
    health: Option<f32>,
    frame_item: Option<String>,
    leashed: Option<bool>,
}

const STAR_FIREWORK_ITEM: &str = r#"{id:"minecraft:firework_rocket",count:1,components:{"minecraft:fireworks":{flight_duration:1,explosions:[{shape:"small_ball",colors:[I;11743532]}]}}}"#;
const PLAIN_FIREWORK_ITEM: &str = r#"{id:"minecraft:firework_rocket",count:1}"#;

fn load_witness() -> Witness {
    let dir = std::env::var_os("SUNRISE_PROJECTILE_WITNESS_DIR").unwrap_or_else(|| {
        panic!(
            "SUNRISE_PROJECTILE_WITNESS_DIR is unset; run tools/test_projectile_targets_parity.py"
        )
    });
    let path = PathBuf::from(dir).join("projectiles_120_124_witness.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_projectile_targets_parity.py): {e}",
            path.display()
        )
    });
    let witness: Witness = serde_json::from_str(&raw).expect("valid 26.3 witness JSON");
    assert_eq!(witness.game_version, "26.3");
    witness
}

fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

fn fbits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

fn vec_bits(v: Vec3) -> [String; 3] {
    [bits(v.x), bits(v.y), bits(v.z)]
}

fn setup_game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.options.dedicated = None;
    let mut player =
        sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 17.5)).unwrap();
    player.client_loaded = true;
    game.player = Some(player);
    crate::game::native_entity_refs::tests::bind_local_player(&mut game, 1, (413u128 << 96) | 1);
    game.level.health = 20.0;
    game.creative = false;
    game.build_registries();
    game.set_allow_commands(true);
    game.load_ticking_neighbourhood(0, 0);
    // The oracle waits for its forced chunk to be ready, including the neighbours
    // that the sculk vibration ticker needs. The fixture uses empty chunk data instead.
    for x in -2..=2 {
        for z in -2..=2 {
            std::sync::Arc::make_mut(game.world.chunks.get_mut(&(x, z)).unwrap()).lit = true;
        }
    }
    // Match vanilla WorldPresets.FLAT: solid ground across the 5x5 neighbourhood for teleports.
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in -32..48 {
        for z in -32..48 {
            game.world.set(x, 0, z, stone).unwrap();
        }
    }
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("difficulty normal"));
    if game
        .adv
        .tree
        .find_id("minecraft:adventure/two_birds_one_arrow")
        .is_none()
    {
        assert!(game.run_command("reload"));
    }
    assert!(
        game.adv
            .tree
            .find_id("minecraft:adventure/two_birds_one_arrow")
            .is_some()
    );
    assert!(
        game.adv
            .tree
            .find_id("minecraft:adventure/arbalistic")
            .is_some()
    );
    for command in [
        "gamerule spawn_mobs false",
        "gamerule advance_time false",
        "gamerule advance_weather false",
        "gamerule mob_griefing true",
        "gamerule mob_drops false",
        "gamerule entity_drops true",
        "gamerule random_tick_speed 0",
    ] {
        let _ = game.run_command(command);
    }
    game
}

fn clear_arena(game: &mut Game<'_>, backstop_x: i32) {
    if game.riding.is_some() {
        game.stop_riding();
    }
    game.mobs.clear();
    game.dropped.clear();
    game.boats.clear();
    game.carts.clear();
    game.armor_stands.clear();
    game.tnts.clear();
    game.falling.clear();
    game.orbs.clear();
    game.bolts.clear();
    game.creatures.projectiles.clear();
    game.rockets.clear();
    game.tridents.clear();
    game.item_frames.clear();
    game.paintings.clear();
    game.cushions.clear();
    game.crystals.clear();
    game.others.clear();
    game.knot_ids.clear();
    game.broken_knots.clear();
    game.resident_entities.knots.clear();
    game.reset_entity_residency();
    game.bind_local_player_residency();
    let max_x = 14.max(backstop_x + 2);
    let _ = game.run_command(&format!("fill 0 63 0 {max_x} 63 10 minecraft:stone"));
    let _ = game.run_command(&format!("fill 0 64 0 {max_x} 70 10 minecraft:air"));
    let _ = game.run_command("fill 3 63 16 5 63 18 minecraft:stone");
    let _ = game.run_command("fill 3 64 16 5 67 18 minecraft:air");
    if backstop_x > 0 {
        let _ = game.run_command(&format!(
            "fill {backstop_x} 64 2 {backstop_x} 67 6 minecraft:stone"
        ));
    }
    let _ = game.run_command("gamerule pvp true");
    let _ = game.run_command("team remove blue");
    let _ = game.run_command("weather clear 10000");
    game.world.weather.rain_level = (0.0, 0.0);
    game.world.weather.thunder_level = (0.0, 0.0);
    game.set_mode(sunrise_client::command::args::GameMode::Survival);
    game.yaw = -90.0;
    game.pitch = 0.0;
    if let Some(p) = game.player.as_mut() {
        let _ = p.set_health(20.0, false);
        p.health.invulnerable_time = 0;
        p.health.hurt.cooldown = 0;
        p.health.hurt.last_hurt = 0.0;
        p.health.permanently_invulnerable = false;
        p.client_loaded = true;
        p.fire_ticks = 0;
        p.position = Vec3::new(4.5, 64.0, 17.5);
        p.velocity = Vec3::ZERO;
    }
    let _ = game.run_command("advancement revoke @s everything");
    game.tick_server(false).unwrap();
    game.audio.heard.clear();
}

fn player_uuid_nbt(game: &Game<'_>) -> String {
    let arr = game.level.player_uuid.unwrap_or([0, 0, 0, 1]);
    format!("[I;{},{},{},{}]", arr[0], arr[1], arr[2], arr[3])
}

fn adv_done(game: &Game<'_>, id: &str) -> bool {
    game.adv
        .tree
        .find_id(id)
        .is_some_and(|idx| game.adv.progress.done(&game.adv.tree.list[idx]))
}

fn step_recording(
    game: &mut Game<'_>,
) -> (
    Vec<(&'static str, [f64; 3], Option<Who>)>,
    BTreeMap<String, usize>,
) {
    game.audio.heard.clear();
    super::super::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    game.tick_server(false).unwrap();
    let events =
        super::super::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow_mut().take().unwrap_or_default());
    let sounds = game
        .audio
        .heard
        .iter()
        .map(|(k, &v)| (k.clone(), v as usize))
        .collect();
    (events, sounds)
}

fn land_positions_match(
    events: &[(&'static str, [f64; 3], Option<Who>)],
    expected: &[GameEventSnap],
) -> Result<(), String> {
    let exp_lands: Vec<&[String; 3]> = expected
        .iter()
        .filter(|e| e.id == "minecraft:projectile_land")
        .map(|e| &e.pos_bits)
        .collect();
    let got_lands: Vec<[String; 3]> = events
        .iter()
        .filter(|&&(id, _, _)| id == "minecraft:projectile_land")
        .map(|(_, at, _)| [bits(at[0]), bits(at[1]), bits(at[2])])
        .collect();
    if got_lands.len() != exp_lands.len() || got_lands.iter().zip(&exp_lands).any(|(a, e)| a != *e)
    {
        Err(format!(
            "projectile_land pos_bits expected {exp_lands:?} got {got_lands:?}"
        ))
    } else {
        Ok(())
    }
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_spectator_player_120() {
    run_witness_spectator_player_120();
}

pub(in crate::game) fn run_witness_spectator_player_120() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();

    for case in &witness.gap_120_spectator {
        let label = format!("120 {} x {}", case.mode, case.projectile);
        clear_arena(&mut game, 8);
        game.world.random.set_seed(witness.level_seed);
        let mode = if case.mode == "spectator" {
            sunrise_client::command::args::GameMode::Spectator
        } else {
            sunrise_client::command::args::GameMode::Survival
        };
        game.set_mode(mode);
        if let Some(p) = game.player.as_mut() {
            let _ = p.set_health(20.0, false);
            p.health.invulnerable_time = 0;
            p.health.hurt.cooldown = 0;
            p.health.hurt.last_hurt = 0.0;
            p.position = Vec3::new(3.5, 64.0, 4.5);
            p.velocity = Vec3::ZERO;
        }
        game.tick_server(false).unwrap();

        let cmd = match case.projectile.as_str() {
            "arrow" => "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}",
            "snowball" => "summon minecraft:snowball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}",
            "small_fireball" => {
                "summon minecraft:small_fireball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}"
            }
            "wind_charge" => {
                "summon minecraft:wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}"
            }
            "trident" => "summon minecraft:trident 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}",
            other => panic!("unexpected projectile {other}"),
        };
        assert!(game.run_command(cmd), "{cmd}");
        let is_trident = case.projectile == "trident";
        let id = if is_trident {
            let idx = game.tridents.len() - 1;
            let id = game.tridents[idx].id;
            game.tridents.update(idx, |t| {
                t.random = LegacyRandom::new(witness.proj_seed);
            });
            id
        } else {
            let idx = game.creatures.projectiles.len() - 1;
            let id = game.creatures.projectiles[idx].id;
            game.creatures.projectiles.update(idx, |p| {
                p.entity_random().set_seed(witness.proj_seed);
            });
            id
        };

        for exp in &case.ticks {
            game.tick_server(false).unwrap();
            let hp = game.player.as_ref().unwrap().health.health();
            if hp.to_bits() != exp.player_health.to_bits() {
                failures.push(format!(
                    "{label} t={}: player_health expected {} got {hp}",
                    exp.tick, exp.player_health
                ));
                break;
            }
            let (alive, pos, vel, ig) = if is_trident {
                match game.tridents.iter().find(|t| t.id == id) {
                    Some(t) => (true, Some(t.position), Some(t.velocity), t.in_ground),
                    None => (false, None, None, false),
                }
            } else {
                match game.creatures.projectiles.iter().find(|p| p.id == id) {
                    Some(p) => (
                        true,
                        Some(p.position),
                        Some(p.velocity),
                        p.arrow.as_ref().is_some_and(|a| a.in_ground),
                    ),
                    None => (false, None, None, false),
                }
            };
            if alive != exp.proj_alive {
                failures.push(format!(
                    "{label} t={}: proj_alive expected {} got {alive}",
                    exp.tick, exp.proj_alive
                ));
                break;
            }
            if let (Some(pos), Some(vel)) = (pos, vel) {
                if vec_bits(pos) != exp.proj_pos_bits {
                    failures.push(format!(
                        "{label} t={}: pos expected {:?} got {:?} ({pos:?})",
                        exp.tick,
                        exp.proj_pos_bits,
                        vec_bits(pos)
                    ));
                    break;
                }
                if vec_bits(vel) != exp.proj_vel_bits {
                    failures.push(format!(
                        "{label} t={}: vel expected {:?} got {:?} ({vel:?})",
                        exp.tick,
                        exp.proj_vel_bits,
                        vec_bits(vel)
                    ));
                    break;
                }
                if ig != exp.in_ground {
                    failures.push(format!(
                        "{label} t={}: in_ground expected {} got {ig}",
                        exp.tick, exp.in_ground
                    ));
                    break;
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "gap #120 failures:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_wind_charge_end_crystal_121() {
    run_witness_wind_charge_end_crystal_121();
}

pub(in crate::game) fn run_witness_wind_charge_end_crystal_121() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();

    for case in &witness.gap_121_wind_crystal {
        let label = format!("121 {}", case.case);
        clear_arena(&mut game, 9);
        game.world.random.set_seed(witness.level_seed);

        assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
        let crystal_id = game.crystals.last().unwrap().id;
        let pig_id = if case.case == "wind_charge_crystal_then_pig" {
            assert!(game.run_command("summon minecraft:pig 7.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
            Some(game.mobs.last().unwrap().id)
        } else {
            None
        };
        let cmd = match case.case.as_str() {
            "wind_charge_through_crystal" | "wind_charge_crystal_then_pig" => {
                "summon minecraft:wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}"
            }
            "breeze_wind_charge_through_crystal" => {
                "summon minecraft:breeze_wind_charge 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],acceleration_power:0.0d}"
            }
            "arrow_hits_crystal" => {
                "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}"
            }
            other => panic!("unexpected case {other}"),
        };
        assert!(game.run_command(cmd), "{cmd}");
        let idx = game.creatures.projectiles.len() - 1;
        let proj_id = game.creatures.projectiles[idx].id;
        game.creatures.projectiles.update(idx, |p| {
            p.entity_random().set_seed(witness.proj_seed);
        });

        for exp in &case.ticks {
            game.tick_server(false).unwrap();
            let crystal_alive = game.crystals.iter().any(|c| c.id == crystal_id);
            if crystal_alive != exp.crystal_alive {
                failures.push(format!(
                    "{label} t={}: crystal_alive expected {} got {crystal_alive}",
                    exp.tick, exp.crystal_alive
                ));
                break;
            }
            if let Some(pid) = pig_id {
                let hp = game
                    .mobs
                    .iter()
                    .find(|m| m.id == pid)
                    .map_or(0.0, |m| m.health);
                if hp.to_bits() != exp.pig_health.to_bits() {
                    failures.push(format!(
                        "{label} t={}: pig_health expected {} got {hp}",
                        exp.tick, exp.pig_health
                    ));
                    break;
                }
            }
            let live = game.creatures.projectiles.iter().find(|p| p.id == proj_id);
            if live.is_some() != exp.proj_alive {
                failures.push(format!(
                    "{label} t={}: proj_alive expected {} got {}",
                    exp.tick,
                    exp.proj_alive,
                    live.is_some()
                ));
                break;
            }
            if let Some(p) = live {
                if vec_bits(p.position) != exp.proj_pos_bits {
                    failures.push(format!(
                        "{label} t={}: pos expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_pos_bits,
                        vec_bits(p.position),
                        p.position
                    ));
                    break;
                }
                if vec_bits(p.velocity) != exp.proj_vel_bits {
                    failures.push(format!(
                        "{label} t={}: vel expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_vel_bits,
                        vec_bits(p.velocity),
                        p.velocity
                    ));
                    break;
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "gap #121 failures:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_trident_enderman_dodge_122() {
    run_witness_trident_enderman_dodge_122();
}

pub(in crate::game) fn run_witness_trident_enderman_dodge_122() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();

    for case in &witness.gap_122_trident_enderman {
        let label = format!("122 {}", case.case);
        clear_arena(&mut game, 10);
        game.world.random.set_seed(witness.level_seed);

        let target_id = match case.case.as_str() {
            "trident_enderman_in_boat" => {
                assert!(game.run_command(
                    r#"summon minecraft:oak_boat 3.5 64.0 4.5 {Passengers:[{id:"minecraft:enderman",NoAI:1b,Silent:1b}]}"#
                ));
                game.mobs.last().unwrap().id
            }
            "trident_invulnerable_pig" => {
                assert!(game.run_command(
                    "summon minecraft:pig 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Invulnerable:1b}"
                ));
                game.mobs.last().unwrap().id
            }
            _ => {
                assert!(
                    game.run_command("summon minecraft:enderman 3.5 64.0 4.5 {NoAI:1b,Silent:1b}")
                );
                game.mobs.last().unwrap().id
            }
        };
        let target_pos0 = game
            .mobs
            .iter()
            .find(|m| m.id == target_id)
            .unwrap()
            .position;
        assert!(game.run_command("summon minecraft:pig 6.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
        let behind_id = game.mobs.last().unwrap().id;

        let cmd = if case.case == "trident_owned_enderman" {
            format!(
                "summon minecraft:trident 1.5 65.0 4.5 {{Motion:[1.0d,0.0d,0.0d],Owner:{},LeftOwner:1b}}",
                player_uuid_nbt(&game)
            )
        } else {
            "summon minecraft:trident 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}".to_owned()
        };
        assert!(game.run_command(&cmd), "{cmd}");
        let idx = game.tridents.len() - 1;
        let tr_id = game.tridents[idx].id;
        game.tridents.update(idx, |t| {
            t.random = LegacyRandom::new(witness.proj_seed);
        });

        for exp in &case.ticks {
            let (events, sounds) = step_recording(&mut game);
            let t = game.tridents.iter().find(|t| t.id == tr_id);
            if t.is_some() != exp.proj_alive {
                failures.push(format!(
                    "{label} t={}: alive expected {} got {}",
                    exp.tick,
                    exp.proj_alive,
                    t.is_some()
                ));
                break;
            }
            if let Some(t) = t {
                if vec_bits(t.position) != exp.proj_pos_bits {
                    failures.push(format!(
                        "{label} t={}: pos expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_pos_bits,
                        vec_bits(t.position),
                        t.position
                    ));
                    break;
                }
                if vec_bits(t.velocity) != exp.proj_vel_bits {
                    failures.push(format!(
                        "{label} t={}: vel expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_vel_bits,
                        vec_bits(t.velocity),
                        t.velocity
                    ));
                    break;
                }
                if t.dealt_damage != exp.dealt_damage {
                    failures.push(format!(
                        "{label} t={}: dealt_damage expected {} got {}",
                        exp.tick, exp.dealt_damage, t.dealt_damage
                    ));
                    break;
                }
                if fbits(t.yaw) != exp.proj_yaw_bits {
                    failures.push(format!(
                        "{label} t={}: yaw expected {} got {} ({})",
                        exp.tick,
                        exp.proj_yaw_bits,
                        fbits(t.yaw),
                        t.yaw
                    ));
                    break;
                }
            }
            let target = game.mobs.iter().find(|m| m.id == target_id).unwrap();
            if target.health.to_bits() != exp.target_health.to_bits() {
                failures.push(format!(
                    "{label} t={}: target_health expected {} got {}",
                    exp.tick, exp.target_health, target.health
                ));
                break;
            }
            let exp_tp = exp
                .game_events
                .iter()
                .filter(|e| e.id == "minecraft:teleport")
                .count();
            let got_tp = events
                .iter()
                .filter(|&&(e, _, _)| e == "minecraft:teleport")
                .count();
            let teleported = got_tp > 0 || vec_bits(target.position) != vec_bits(target_pos0);
            if teleported != exp.target_teleported || got_tp != exp_tp {
                failures.push(format!(
                    "{label} t={}: target_teleported expected {} (tp={exp_tp}) got {teleported} (tp={got_tp})",
                    exp.tick, exp.target_teleported
                ));
                break;
            }
            let behind_hp = game
                .mobs
                .iter()
                .find(|m| m.id == behind_id)
                .map_or(0.0, |m| m.health);
            if behind_hp.to_bits() != exp.behind_pig_health.to_bits() {
                failures.push(format!(
                    "{label} t={}: behind_pig_health expected {} got {behind_hp}",
                    exp.tick, exp.behind_pig_health
                ));
                break;
            }
            let exp_hit_snd = exp
                .sounds
                .iter()
                .filter(|s| s.as_str() == "minecraft:item.trident.hit")
                .count();
            let got_hit_snd = sounds
                .get("minecraft:item.trident.hit")
                .copied()
                .unwrap_or(0);
            if got_hit_snd != exp_hit_snd {
                failures.push(format!(
                    "{label} t={}: item.trident.hit count expected {exp_hit_snd} got {got_hit_snd}",
                    exp.tick
                ));
                break;
            }
            if let Err(msg) = land_positions_match(&events, &exp.game_events) {
                failures.push(format!("{label} t={}: {msg}", exp.tick));
                break;
            }
        }
    }

    // Also verify mob-thrown ProjectileKind::Trident path against unowned Enderman witness.
    let unowned = witness
        .gap_122_trident_enderman
        .iter()
        .find(|c| c.case == "trident_unowned_enderman")
        .unwrap();
    clear_arena(&mut game, 10);
    game.world.random.set_seed(witness.level_seed);
    assert!(game.run_command("summon minecraft:enderman 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
    let enderman_id = game.mobs.last().unwrap().id;
    let enderman_pos0 = game
        .mobs
        .iter()
        .find(|m| m.id == enderman_id)
        .unwrap()
        .position;
    assert!(game.run_command("summon minecraft:pig 6.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
    let behind_id = game.mobs.last().unwrap().id;
    let mut proj = Projectile::new(ProjectileKind::Trident, Vec3::new(1.5, 65.0, 4.5), None);
    proj.id = game.take_entity_id();
    proj.velocity = Vec3::new(1.0, 0.0, 0.0);
    proj.entity_random().set_seed(witness.proj_seed);
    let mob_tr_id = proj.id;
    game.creatures.projectiles.push(proj);
    for exp in &unowned.ticks {
        let (events, sounds) = step_recording(&mut game);
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == mob_tr_id);
        let Some(p) = p else {
            failures.push(format!("122 mob_trident t={}: disappeared", exp.tick));
            break;
        };
        if vec_bits(p.position) != exp.proj_pos_bits
            || vec_bits(p.velocity) != exp.proj_vel_bits
            || fbits(p.yaw) != exp.proj_yaw_bits
        {
            failures.push(format!(
                "122 mob_trident t={}: pos/vel/yaw expected {:?}/{:?}/{} got {:?}/{:?}/{}",
                exp.tick,
                exp.proj_pos_bits,
                exp.proj_vel_bits,
                exp.proj_yaw_bits,
                vec_bits(p.position),
                vec_bits(p.velocity),
                fbits(p.yaw)
            ));
            break;
        }
        let dd = p.arrow.as_ref().is_some_and(|a| a.dealt_damage);
        if dd != exp.dealt_damage {
            failures.push(format!(
                "122 mob_trident t={}: dealt_damage expected {} got {dd}",
                exp.tick, exp.dealt_damage
            ));
            break;
        }
        let em = game.mobs.iter().find(|m| m.id == enderman_id).unwrap();
        let behind_hp = game
            .mobs
            .iter()
            .find(|m| m.id == behind_id)
            .map_or(0.0, |m| m.health);
        let hit_snd = sounds
            .get("minecraft:item.trident.hit")
            .copied()
            .unwrap_or(0);
        let exp_tp = exp
            .game_events
            .iter()
            .filter(|e| e.id == "minecraft:teleport")
            .count();
        let got_tp = events
            .iter()
            .filter(|&&(e, _, _)| e == "minecraft:teleport")
            .count();
        let teleported = got_tp > 0 || vec_bits(em.position) != vec_bits(enderman_pos0);
        if let Err(msg) = land_positions_match(&events, &exp.game_events) {
            failures.push(format!("122 mob_trident t={}: {msg}", exp.tick));
            break;
        }
        if em.health.to_bits() != exp.target_health.to_bits()
            || teleported != exp.target_teleported
            || behind_hp.to_bits() != exp.behind_pig_health.to_bits()
            || hit_snd != 0
            || got_tp != exp_tp
        {
            failures.push(format!(
                "122 mob_trident t={}: em_hp={} tp={teleported} behind_hp={behind_hp} snd={hit_snd} tp_ev={got_tp}",
                exp.tick, em.health
            ));
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "gap #122 failures:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_piercing_arrow_multihit_123() {
    run_witness_piercing_arrow_multihit_123();
}

pub(in crate::game) fn run_witness_piercing_arrow_multihit_123() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();
    let xbow = r#"{id:"minecraft:crossbow",count:1}"#;

    for case in &witness.gap_123_piercing {
        let label = format!("123 {}", case.case);
        let wall_x = if case.case == "pierce_through_mob_into_wall_same_tick" {
            4
        } else {
            11
        };
        clear_arena(&mut game, wall_x);
        game.world.random.set_seed(witness.level_seed);

        let mut tracked_mobs: Vec<(bool, u32)> = Vec::new();
        match case.case.as_str() {
            "pierce_two_mobs_same_tick" => {
                assert!(game.run_command(
                    "summon minecraft:chicken 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
                assert!(game.run_command(
                    "summon minecraft:pig 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
            }
            "pierce_two_phantoms_two_birds" => {
                assert!(game.run_command(
                    "summon minecraft:phantom 2.5 64.3 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
                assert!(game.run_command(
                    "summon minecraft:phantom 3.8 64.3 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
            }
            "pierce_five_unique_arbalistic" => {
                for (kind, x) in [
                    ("pig", "2.5"),
                    ("cow", "3.5"),
                    ("sheep", "4.5"),
                    ("chicken", "5.8"),
                    ("villager", "7.0"),
                ] {
                    let cmd = format!(
                        "summon minecraft:{kind} {x} 64.0 4.5 {{NoAI:1b,Silent:1b,Health:1.0f}}"
                    );
                    assert!(game.run_command(&cmd), "{cmd}");
                    tracked_mobs.push((true, game.mobs.last().unwrap().id));
                }
            }
            "pierce_limit_stops_at_pierce_plus_one" => {
                for x in ["2.5", "3.5", "4.5"] {
                    let cmd = format!(
                        "summon minecraft:pig {x} 64.0 4.5 {{NoAI:1b,Silent:1b,Health:1.0f}}"
                    );
                    assert!(game.run_command(&cmd), "{cmd}");
                    tracked_mobs.push((true, game.mobs.last().unwrap().id));
                }
            }
            "pierce_through_mob_into_wall_same_tick" => {
                assert!(game.run_command(
                    "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
            }
            "pierce_hits_falling_block_deflects" => {
                assert!(game.run_command(
                    r#"summon minecraft:falling_block 2.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
                ));
                tracked_mobs.push((false, game.falling.last().unwrap().id));
                assert!(game.run_command(
                    "summon minecraft:zombie 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
            }
            "pierce_hits_breeze_deflects" => {
                assert!(
                    game.run_command("summon minecraft:breeze 2.5 64.0 4.5 {NoAI:1b,Silent:1b}")
                );
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
                assert!(game.run_command(
                    "summon minecraft:pig 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:1.0f}"
                ));
                tracked_mobs.push((true, game.mobs.last().unwrap().id));
            }
            other => panic!("unexpected case {other}"),
        }

        let owner = player_uuid_nbt(&game);
        let arrow_cmd = match case.case.as_str() {
            "pierce_two_mobs_same_tick"
            | "pierce_two_phantoms_two_birds"
            | "pierce_through_mob_into_wall_same_tick"
            | "pierce_hits_breeze_deflects" => format!(
                "summon minecraft:arrow 1.5 64.5 4.5 {{Motion:[3.0d,0.0d,0.0d],damage:20.0d,PierceLevel:2b,Owner:{owner},LeftOwner:1b,weapon:{xbow}}}"
            ),
            "pierce_five_unique_arbalistic" => format!(
                "summon minecraft:arrow 1.5 64.5 4.5 {{Motion:[3.5d,0.0d,0.0d],damage:20.0d,PierceLevel:4b,Owner:{owner},LeftOwner:1b,weapon:{xbow}}}"
            ),
            "pierce_limit_stops_at_pierce_plus_one" => format!(
                "summon minecraft:arrow 1.5 64.5 4.5 {{Motion:[4.0d,0.0d,0.0d],damage:20.0d,PierceLevel:1b,Owner:{owner},LeftOwner:1b,weapon:{xbow}}}"
            ),
            "pierce_hits_falling_block_deflects" => format!(
                "summon minecraft:arrow 1.5 65.3 4.5 {{Motion:[3.0d,0.0d,0.0d],damage:20.0d,PierceLevel:2b,Owner:{owner},LeftOwner:1b,weapon:{xbow}}}"
            ),
            other => panic!("unexpected case {other}"),
        };
        assert!(game.run_command(&arrow_cmd), "{arrow_cmd}");
        let idx = game.creatures.projectiles.len() - 1;
        let arrow_id = game.creatures.projectiles[idx].id;
        game.creatures.projectiles.update(idx, |p| {
            p.entity_random().set_seed(witness.proj_seed);
        });

        for exp in &case.ticks {
            let (events, sounds) = step_recording(&mut game);
            let live = game.creatures.projectiles.iter().find(|p| p.id == arrow_id);
            if live.is_some() != exp.proj_alive {
                failures.push(format!(
                    "{label} t={}: proj_alive expected {} got {}",
                    exp.tick,
                    exp.proj_alive,
                    live.is_some()
                ));
                break;
            }
            if let Some(p) = live {
                if vec_bits(p.position) != exp.proj_pos_bits {
                    failures.push(format!(
                        "{label} t={}: pos expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_pos_bits,
                        vec_bits(p.position),
                        p.position
                    ));
                    break;
                }
                if vec_bits(p.velocity) != exp.proj_vel_bits {
                    failures.push(format!(
                        "{label} t={}: vel expected {:?} got {:?} ({:?})",
                        exp.tick,
                        exp.proj_vel_bits,
                        vec_bits(p.velocity),
                        p.velocity
                    ));
                    break;
                }
                let ig = p.arrow.as_ref().is_some_and(|a| a.in_ground);
                if ig != exp.in_ground {
                    failures.push(format!(
                        "{label} t={}: in_ground expected {} got {ig}",
                        exp.tick, exp.in_ground
                    ));
                    break;
                }
                if fbits(p.yaw) != exp.proj_yaw_bits {
                    failures.push(format!(
                        "{label} t={}: yaw expected {} got {} ({})",
                        exp.tick,
                        exp.proj_yaw_bits,
                        fbits(p.yaw),
                        p.yaw
                    ));
                    break;
                }
            }
            let mut target_err = false;
            for (k, ((is_mob, tid), exp_t)) in tracked_mobs.iter().zip(&exp.targets).enumerate() {
                let (alive, hp) = if *is_mob {
                    match game.mobs.iter().find(|m| m.id == *tid) {
                        Some(m) => (m.alive(), m.health),
                        None => (false, 0.0),
                    }
                } else {
                    (game.falling.iter().any(|f| f.id == *tid), -1.0)
                };
                if alive != exp_t.alive || hp.to_bits() != exp_t.health.to_bits() {
                    failures.push(format!(
                        "{label} t={} target[{k}]: expected (alive={}, hp={}) got ({alive}, {hp})",
                        exp.tick, exp_t.alive, exp_t.health
                    ));
                    target_err = true;
                    break;
                }
            }
            if target_err {
                break;
            }
            let two_birds = adv_done(&game, "minecraft:adventure/two_birds_one_arrow");
            if two_birds != exp.two_birds_done {
                failures.push(format!(
                    "{label} t={}: two_birds_one_arrow expected {} got {two_birds}",
                    exp.tick, exp.two_birds_done
                ));
                break;
            }
            let arbalistic = adv_done(&game, "minecraft:adventure/arbalistic");
            if arbalistic != exp.arbalistic_done {
                failures.push(format!(
                    "{label} t={}: arbalistic expected {} got {arbalistic}",
                    exp.tick, exp.arbalistic_done
                ));
                break;
            }
            if let Err(msg) = land_positions_match(&events, &exp.game_events) {
                failures.push(format!("{label} t={}: {msg}", exp.tick));
                break;
            }
            for snd in [
                "minecraft:entity.arrow.hit",
                "minecraft:entity.breeze.deflect",
            ] {
                let exp_n = exp.sounds.iter().filter(|s| s.as_str() == snd).count();
                let got_n = sounds.get(snd).copied().unwrap_or(0);
                if got_n != exp_n {
                    failures.push(format!(
                        "{label} t={}: {snd} count expected {exp_n} got {got_n}",
                        exp.tick
                    ));
                    target_err = true;
                    break;
                }
            }
            if target_err {
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "gap #123 failures:\n{}",
        failures.join("\n")
    );
}

#[derive(Clone, Copy)]
enum Target124 {
    Boat(u32),
    Cart(u32),
    Frame(u32),
    Painting(u32),
    Falling(u32),
    Knot(u32),
    Cushion(u32),
    Crystal(u32),
    Tnt(u32),
    Stand(u32),
}

fn alive_124(game: &Game<'_>, t: Target124) -> bool {
    match t {
        Target124::Boat(id) => game.boats.iter().any(|b| b.id == id),
        Target124::Cart(id) => game.carts.iter().any(|c| c.id == id),
        Target124::Frame(id) => game.item_frames.iter().any(|f| f.id == id),
        Target124::Painting(id) => game.paintings.iter().any(|p| p.id == id),
        Target124::Falling(id) => game.falling.iter().any(|f| f.id == id),
        Target124::Knot(id) => game.others.iter().any(|o| o.id == id),
        Target124::Cushion(id) => game.cushions.iter().any(|c| c.id == id),
        Target124::Crystal(id) => game.crystals.iter().any(|c| c.id == id),
        Target124::Tnt(id) => game.tnts.iter().any(|t| t.id == id),
        Target124::Stand(id) => game.armor_stands.iter().any(|a| a.id == id),
    }
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_firework_rocket_targets_124() {
    run_witness_firework_rocket_targets_124();
}

pub(in crate::game) fn run_witness_firework_rocket_targets_124() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();

    for case in &witness.gap_124_fireworks {
        let label = format!("124 {}", case.case);
        clear_arena(&mut game, 10);
        game.world.random.set_seed(witness.level_seed);

        if case.case == "item_frame" || case.case == "painting" {
            let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
        }
        if case.case == "leash_knot" {
            let _ = game.run_command("setblock 3 64 4 minecraft:oak_fence");
        }
        if case.case == "armor_stand_blast_nearby" {
            let _ = game.run_command("fill 6 64 3 6 66 5 minecraft:stone");
        }

        let mut targets: BTreeMap<String, Target124> = BTreeMap::new();
        match case.case.as_str() {
            "oak_boat" | "oak_boat_no_star" => {
                assert!(game.run_command("summon minecraft:oak_boat 3.5 64.0 4.5 {}"));
                targets.insert(
                    "boat".into(),
                    Target124::Boat(game.boats.last().unwrap().id),
                );
            }
            "minecart" => {
                assert!(game.run_command("summon minecraft:minecart 3.5 64.0 4.5 {}"));
                targets.insert(
                    "minecart".into(),
                    Target124::Cart(game.carts.last().unwrap().id),
                );
            }
            "item_frame" => {
                assert!(game.run_command(
                    r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
                ));
                targets.insert(
                    "frame".into(),
                    Target124::Frame(game.item_frames.last().unwrap().id),
                );
            }
            "painting" => {
                assert!(game.run_command(
                    r#"summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:"minecraft:kebab"}"#
                ));
                targets.insert(
                    "painting".into(),
                    Target124::Painting(game.paintings.last().unwrap().id),
                );
            }
            "falling_block" => {
                assert!(game.run_command(
                    r#"summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
                ));
                targets.insert(
                    "falling".into(),
                    Target124::Falling(game.falling.last().unwrap().id),
                );
            }
            "leash_knot" => {
                assert!(game.run_command("summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
                targets.insert(
                    "knot".into(),
                    Target124::Knot(game.others.last().unwrap().id),
                );
            }
            "cushion" => {
                assert!(game.run_command(
                    r#"summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:"white"}"#
                ));
                targets.insert(
                    "cushion".into(),
                    Target124::Cushion(game.cushions.last().unwrap().id),
                );
            }
            "end_crystal" => {
                assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                targets.insert(
                    "crystal".into(),
                    Target124::Crystal(game.crystals.last().unwrap().id),
                );
            }
            "tnt" => {
                assert!(game.run_command("summon minecraft:tnt 3.5 64.0 4.5 {fuse:80}"));
                targets.insert("tnt".into(), Target124::Tnt(game.tnts.last().unwrap().id));
            }
            "armor_stand" => {
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                targets.insert(
                    "stand".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            "armor_stand_marker" => {
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
                targets.insert(
                    "stand".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            "armor_stand_blast_nearby" => {
                assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                targets.insert(
                    "normal".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(
                    game.run_command("summon minecraft:armor_stand 3.5 64.0 6.5 {Invisible:1b}")
                );
                targets.insert(
                    "invisible".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 2.5 {Marker:1b}"));
                targets.insert(
                    "marker".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 7.5 64.0 4.5 {}"));
                targets.insert(
                    "occluded".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command("summon minecraft:armor_stand 9.5 64.0 4.5 {}"));
                targets.insert(
                    "far".into(),
                    Target124::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            other => panic!("unexpected case {other}"),
        }

        let rocket_cmd = if case.case == "armor_stand_blast_nearby" {
            format!(
                "summon minecraft:firework_rocket 3.5 64.5 4.5 {{LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
            )
        } else {
            let plain = matches!(
                case.case.as_str(),
                "oak_boat_no_star" | "item_frame" | "painting" | "leash_knot"
            );
            let item = if plain {
                PLAIN_FIREWORK_ITEM
            } else {
                STAR_FIREWORK_ITEM
            };
            format!(
                "summon minecraft:firework_rocket 1.5 {} 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:{item}}}",
                case.spawn_y
            )
        };
        assert!(game.run_command(&rocket_cmd), "{rocket_cmd}");
        let idx = game.rockets.len() - 1;
        let rocket_id = game.rockets[idx].id;
        game.rockets.update(idx, |r| {
            r.random = LegacyRandom::new(witness.proj_seed);
        });

        for exp in &case.ticks {
            let (events, sounds) = step_recording(&mut game);
            let live = game.rockets.iter().find(|r| r.id == rocket_id);
            if live.is_some() != exp.rocket_alive {
                failures.push(format!(
                    "{label} t={}: rocket_alive expected {} got {}",
                    exp.tick,
                    exp.rocket_alive,
                    live.is_some()
                ));
                break;
            }
            if let Some(r) = live
                && (vec_bits(r.position) != exp.rocket_pos_bits
                    || vec_bits(r.velocity) != exp.rocket_vel_bits)
            {
                failures.push(format!(
                    "{label} t={}: pos/vel expected {:?}/{:?} got {:?}/{:?}",
                    exp.tick,
                    exp.rocket_pos_bits,
                    exp.rocket_vel_bits,
                    vec_bits(r.position),
                    vec_bits(r.velocity)
                ));
                break;
            }
            let got_alive: BTreeMap<String, bool> = targets
                .iter()
                .map(|(k, &t)| (k.clone(), alive_124(&game, t)))
                .collect();
            if got_alive != exp.targets_alive {
                failures.push(format!(
                    "{label} t={}: targets_alive expected {:?} got {got_alive:?}",
                    exp.tick, exp.targets_alive
                ));
                break;
            }
            let mut got_drops: Vec<String> = game
                .dropped
                .iter()
                .map(|d| d.stack.item.to_string())
                .collect();
            got_drops.sort();
            if got_drops != exp.drops {
                failures.push(format!(
                    "{label} t={}: drops expected {:?} got {got_drops:?}",
                    exp.tick, exp.drops
                ));
                break;
            }
            if let Err(msg) = land_positions_match(&events, &exp.game_events) {
                failures.push(format!("{label} t={}: {msg}", exp.tick));
                break;
            }
            let exp_expl = exp
                .game_events
                .iter()
                .filter(|e| e.id == "minecraft:explode")
                .count();
            let got_expl = events
                .iter()
                .filter(|&&(e, _, _)| e == "minecraft:explode")
                .count();
            if got_expl != exp_expl {
                failures.push(format!(
                    "{label} t={}: explode count expected {exp_expl} got {got_expl}",
                    exp.tick
                ));
                break;
            }
            let exp_break = exp
                .sounds
                .iter()
                .filter(|s| s.as_str() == "minecraft:entity.armor_stand.break")
                .count();
            let got_break = sounds
                .get("minecraft:entity.armor_stand.break")
                .copied()
                .unwrap_or(0);
            if got_break != exp_break {
                failures.push(format!(
                    "{label} t={}: armor_stand.break count expected {exp_break} got {got_break}",
                    exp.tick
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "gap #124 failures:\n{}",
        failures.join("\n")
    );
}

#[derive(Clone, Copy)]
enum ReviewTracked {
    Mob(u32),
    Stand(u32),
    Frame(u32),
    Crystal(u32),
    Boat(u32),
    Falling(u32),
    Proj(u32),
    Trident(u32),
    Rocket(u32),
}

fn summon_mob_seeded(game: &mut Game<'_>, cmd: &str, seed: i64) -> u32 {
    assert!(game.run_command(cmd), "{cmd}");
    let idx = game.mobs.len() - 1;
    let id = game.mobs[idx].id;
    game.mobs[idx].random.set_seed(seed);
    id
}

fn summon_proj_seeded(game: &mut Game<'_>, cmd: &str, seed: i64) -> u32 {
    assert!(game.run_command(cmd), "{cmd}");
    let idx = game.creatures.projectiles.len() - 1;
    let id = game.creatures.projectiles[idx].id;
    game.creatures.projectiles.update(idx, |p| {
        p.entity_random().set_seed(seed);
    });
    id
}

fn summon_trident_seeded(game: &mut Game<'_>, cmd: &str, seed: i64) -> u32 {
    assert!(game.run_command(cmd), "{cmd}");
    let idx = game.tridents.len() - 1;
    let id = game.tridents[idx].id;
    game.tridents.update(idx, |t| {
        t.random = LegacyRandom::new(seed);
    });
    id
}

fn summon_rocket_seeded(game: &mut Game<'_>, cmd: &str, seed: i64) -> u32 {
    assert!(game.run_command(cmd), "{cmd}");
    let idx = game.rockets.len() - 1;
    let id = game.rockets[idx].id;
    game.rockets.update(idx, |r| {
        r.random = LegacyRandom::new(seed);
    });
    id
}

fn check_review_entity(
    game: &Game<'_>,
    label: &str,
    tick: usize,
    key: &str,
    tracked: ReviewTracked,
    exp: &ReviewEntitySnap,
    failures: &mut Vec<String>,
) -> bool {
    let check_kinematics = |pos: Vec3, vel: Vec3, yaw: Option<f32>, failures: &mut Vec<String>| {
        if vec_bits(pos) != exp.pos_bits {
            failures.push(format!(
                "{label} t={tick} {key}: pos expected {:?} got {:?} ({pos:?})",
                exp.pos_bits,
                vec_bits(pos)
            ));
            return false;
        }
        if vec_bits(vel) != exp.vel_bits {
            failures.push(format!(
                "{label} t={tick} {key}: vel expected {:?} got {:?} ({vel:?})",
                exp.vel_bits,
                vec_bits(vel)
            ));
            return false;
        }
        if let Some(y) = yaw
            && fbits(y) != exp.yaw_bits
        {
            failures.push(format!(
                "{label} t={tick} {key}: yaw expected {} got {} ({y})",
                exp.yaw_bits,
                fbits(y)
            ));
            return false;
        }
        true
    };
    match tracked {
        ReviewTracked::Mob(id) => {
            let live = game.mobs.iter().find(|m| m.id == id && !m.removed);
            let present = live.is_some();
            let alive = live.is_some_and(|m| m.alive());
            if present != exp.present || alive != exp.alive {
                failures.push(format!(
                    "{label} t={tick} {key}: mob present/alive expected {}/{} got {present}/{alive}",
                    exp.present, exp.alive
                ));
                return false;
            }
            if let Some(m) = live {
                if let Some(exp_hp) = exp.health
                    && m.health.to_bits() != exp_hp.to_bits()
                {
                    failures.push(format!(
                        "{label} t={tick} {key}: health expected {exp_hp} got {}",
                        m.health
                    ));
                    return false;
                }
                if m.fire_ticks != exp.fire_ticks {
                    failures.push(format!(
                        "{label} t={tick} {key}: fire_ticks expected {} got {}",
                        exp.fire_ticks, m.fire_ticks
                    ));
                    return false;
                }
                if let Some(exp_leashed) = exp.leashed {
                    let leashed = matches!(m.extra.leash, Some(Holder::Knot(_)));
                    if leashed != exp_leashed {
                        failures.push(format!(
                            "{label} t={tick} {key}: leashed expected {exp_leashed} got {leashed}"
                        ));
                        return false;
                    }
                }
                if exp.alive {
                    if label.contains("r18_arrow_knockback_") {
                        // LivingEntity.knockback falls back to unseeded Math.random() when xd^2+zd^2 < 1e-4;
                        // check exact pos, exact vel.y (0.0 vs 0.1 from Punch), and Punch +0.6 X impulse.
                        if vec_bits(m.position) != exp.pos_bits {
                            failures.push(format!(
                                "{label} t={tick} {key}: pos expected {:?} got {:?}",
                                exp.pos_bits,
                                vec_bits(m.position)
                            ));
                            return false;
                        }
                        if bits(m.velocity.y) != exp.vel_bits[1] {
                            failures.push(format!(
                                "{label} t={tick} {key}: vel.y expected {} got {} ({})",
                                exp.vel_bits[1],
                                bits(m.velocity.y),
                                m.velocity.y
                            ));
                            return false;
                        }
                        let punch_x = if label.contains("above_threshold") {
                            0.6
                        } else {
                            0.0
                        };
                        let base_norm = (m.velocity.x - punch_x).hypot(m.velocity.z);
                        if (base_norm - 0.4).abs() > 1e-6 {
                            failures.push(format!(
                                "{label} t={tick} {key}: base knockback norm expected 0.4 after punch_x={punch_x} got {base_norm} ({:?})",
                                m.velocity
                            ));
                            return false;
                        }
                    } else if !check_kinematics(m.position, m.velocity, None, failures) {
                        return false;
                    }
                }
            }
        }
        ReviewTracked::Stand(id) => {
            let live = game.armor_stands.iter().find(|a| a.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: stand present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(a) = live {
                if let Some(exp_hp) = exp.health
                    && a.health.to_bits() != exp_hp.to_bits()
                {
                    failures.push(format!(
                        "{label} t={tick} {key}: stand health expected {exp_hp} got {}",
                        a.health
                    ));
                    return false;
                }
                if a.fire_ticks != exp.fire_ticks {
                    failures.push(format!(
                        "{label} t={tick} {key}: stand fire_ticks expected {} got {}",
                        exp.fire_ticks, a.fire_ticks
                    ));
                    return false;
                }
                let frozen = sunrise_client::save::number(
                    game.persist.raw.get(&id).and_then(|t| t.get("TicksFrozen")),
                )
                .unwrap_or(0) as i32;
                if exp.ticks_frozen == 0 && frozen != 0 {
                    failures.push(format!(
                        "{label} t={tick} {key}: stand ticks_frozen expected 0 got {frozen}"
                    ));
                    return false;
                }
            }
        }
        ReviewTracked::Frame(id) => {
            let live = game.item_frames.iter().find(|f| f.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: frame present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(f) = live {
                let item = f.item.as_ref().map(|s| s.item.to_string());
                if item != exp.frame_item {
                    failures.push(format!(
                        "{label} t={tick} {key}: frame item expected {:?} got {item:?}",
                        exp.frame_item
                    ));
                    return false;
                }
            }
        }
        ReviewTracked::Crystal(id) => {
            let present = game.crystals.iter().any(|c| c.id == id);
            if present != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: crystal present expected {} got {present}",
                    exp.present
                ));
                return false;
            }
        }
        ReviewTracked::Boat(id) => {
            let present = game.boats.iter().any(|b| b.id == id);
            if present != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: boat present expected {} got {present}",
                    exp.present
                ));
                return false;
            }
        }
        ReviewTracked::Falling(id) => {
            let live = game.falling.iter().find(|f| f.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: falling present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(f) = live
                && !check_kinematics(f.position, f.velocity, None, failures)
            {
                return false;
            }
        }
        ReviewTracked::Proj(id) => {
            let live = game.creatures.projectiles.iter().find(|p| p.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: proj present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(p) = live {
                let check_yaw = !matches!(
                    p.kind,
                    ProjectileKind::SmallFireball
                        | ProjectileKind::LargeFireball { .. }
                        | ProjectileKind::WitherSkull { .. }
                        | ProjectileKind::WindCharge
                        | ProjectileKind::BreezeWindCharge
                        | ProjectileKind::ShulkerBullet { .. }
                );
                if !check_kinematics(p.position, p.velocity, check_yaw.then_some(p.yaw), failures) {
                    return false;
                }
            }
        }
        ReviewTracked::Trident(id) => {
            let live = game.tridents.iter().find(|t| t.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: trident present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(t) = live
                && !check_kinematics(t.position, t.velocity, Some(t.yaw), failures)
            {
                return false;
            }
        }
        ReviewTracked::Rocket(id) => {
            let live = game.rockets.iter().find(|r| r.id == id);
            if live.is_some() != exp.present {
                failures.push(format!(
                    "{label} t={tick} {key}: rocket present expected {} got {}",
                    exp.present,
                    live.is_some()
                ));
                return false;
            }
            if let Some(r) = live
                && !check_kinematics(r.position, r.velocity, Some(r.rotation[0]), failures)
            {
                return false;
            }
        }
    }
    true
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_review_round1_regressions() {
    run_witness_review_round1_regressions();
}

pub(in crate::game) fn run_witness_review_round1_regressions() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();
    let owner = player_uuid_nbt(&game);

    for case in &witness.review_round1 {
        let label = format!("review {}", case.case);
        let backstop_x = match case.case.as_str() {
            "r03_pierce_two_crystals" => 16,
            "r05_wind_charge_redirects_fireball"
            | "r06_arrow_skips_boat_copassenger"
            | "r07_firework_margin_tick8" => 12,
            "r12_pierce1_crit_three_zombies" => 11,
            _ => 10,
        };
        clear_arena(&mut game, backstop_x);
        game.world.random.set_seed(witness.level_seed);

        let mut tracked: BTreeMap<String, ReviewTracked> = BTreeMap::new();
        let seed = witness.proj_seed;
        match case.case.as_str() {
            "r01_potion_slab_blocks_pig" => {
                assert!(game.run_command("setblock 3 64 4 minecraft:stone_slab[type=bottom]"));
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 8.5 64.0 4.5 {NoAI:1b,Silent:1b}",
                        seed,
                    )),
                );
                tracked.insert(
                    "potion".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        r#"summon minecraft:splash_potion 1.5 64.3 4.5 {Motion:[1.0d,0.0d,0.0d],Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:harming"}}}}"#,
                        seed,
                    )),
                );
            }
            "r01_exp_bottle_slab_clip" => {
                assert!(game.run_command("setblock 3 64 4 minecraft:stone_slab[type=bottom]"));
                tracked.insert(
                    "bottle".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:experience_bottle 1.5 64.3 4.5 {Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r01_potion_thin_frame" => {
                let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
                assert!(game.run_command(
                    r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
                ));
                tracked.insert(
                    "frame".into(),
                    ReviewTracked::Frame(game.item_frames.last().unwrap().id),
                );
                tracked.insert(
                    "potion".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        r#"summon minecraft:splash_potion 1.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:water"}}}}"#,
                        seed,
                    )),
                );
            }
            "r02_two_arrows_empty_frame" => {
                let _ = game.run_command("fill 2 64 5 5 66 5 minecraft:stone");
                assert!(game.run_command(
                    "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:2b}"
                ));
                tracked.insert(
                    "frame".into(),
                    ReviewTracked::Frame(game.item_frames.last().unwrap().id),
                );
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 5.5 64.0 4.54 {NoAI:1b,Silent:1b,Health:10.0f}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow1".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 64.5 4.96 {Motion:[2.5d,0.0d,0.0d],damage:2.0d}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow2".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 64.5 4.96 {Motion:[2.5d,0.0d,0.0d],damage:2.0d}",
                        seed,
                    )),
                );
            }
            "r02_rocket_breaks_stand_before_arrow" => {
                assert!(game.run_command("summon minecraft:armor_stand 4.5 64.0 4.5 {}"));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 7.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}",
                        seed,
                    )),
                );
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 3.5 64.5 4.5 {{LifeTime:0,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 2.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d}",
                        seed,
                    )),
                );
            }
            "r03_pierce_two_crystals" => {
                assert!(game.run_command("summon minecraft:end_crystal 3.5 64.0 4.5 {}"));
                tracked.insert(
                    "crystal1".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                assert!(
                    game.run_command(
                        "summon minecraft:end_crystal 10.5 64.0 4.5 {Invulnerable:1b}"
                    )
                );
                tracked.insert(
                    "crystal2".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[9.5d,0.0d,0.0d],PierceLevel:2b,damage:2.0d}",
                        seed,
                    )),
                );
            }
            "r04_tie_stand_before_parrot" | "r04_tie_parrot_before_stand" => {
                if case.case == "r04_tie_stand_before_parrot" {
                    assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                    tracked.insert(
                        "stand".into(),
                        ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                    );
                    tracked.insert(
                        "parrot".into(),
                        ReviewTracked::Mob(summon_mob_seeded(
                            &mut game,
                            "summon minecraft:parrot 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:6.0f}",
                            seed,
                        )),
                    );
                } else {
                    tracked.insert(
                        "parrot".into(),
                        ReviewTracked::Mob(summon_mob_seeded(
                            &mut game,
                            "summon minecraft:parrot 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:6.0f}",
                            seed,
                        )),
                    );
                    assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                    tracked.insert(
                        "stand".into(),
                        ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                    );
                }
                tracked.insert(
                    "snowball".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:snowball 1.5 64.3 4.5 {Motion:[2.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r04_arrow_sort_frame_center" => {
                let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
                assert!(game.run_command("summon minecraft:armor_stand 3.74 64.0 4.5 {}"));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                assert!(game.run_command(
                    r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
                ));
                tracked.insert(
                    "frame".into(),
                    ReviewTracked::Frame(game.item_frames.last().unwrap().id),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 3.0 64.85 4.5 {Motion:[1.5d,0.0d,0.0d],damage:2.0d}",
                        seed,
                    )),
                );
            }
            "r05_wind_charge_redirects_fireball" => {
                tracked.insert(
                    "fireball".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:fireball 3.5 65.0 4.5 {ExplosionPower:1b,acceleration_power:0.0d,Motion:[0.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
                tracked.insert(
                    "wind".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:wind_charge 1.5 65.0 4.5 {{acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Owner:{owner},LeftOwner:1b}}"
                        ),
                        seed,
                    )),
                );
            }
            "r05_arrow_pops_shulker_bullet" => {
                tracked.insert(
                    "bullet".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:shulker_bullet 3.5 65.0 4.5 {Motion:[0.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],damage:2.0d}",
                        seed,
                    )),
                );
            }
            "r05_wind_charge_skips_wind_charge" => {
                tracked.insert(
                    "wind1".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:wind_charge 1.5 65.0 4.5 {acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
                tracked.insert(
                    "wind2".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:wind_charge 4.5 65.0 4.5 {acceleration_power:0.0d,Motion:[-1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r06_arrow_skips_boat_copassenger" => {
                assert!(game.run_command("summon minecraft:oak_boat 2.5 64.0 4.5 {}"));
                let boat_id = game.boats.last().unwrap().id;
                let copass_id = summon_mob_seeded(
                    &mut game,
                    "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}",
                    seed,
                );
                assert!(game.start_riding_target(Target::Player, Target::Boat(boat_id), true));
                assert!(game.start_riding_target(
                    Target::Mob(copass_id),
                    Target::Boat(boat_id),
                    true
                ));
                let outside_id = summon_mob_seeded(
                    &mut game,
                    "summon minecraft:pig 6.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}",
                    seed,
                );
                game.tick_server(false).unwrap();
                tracked.insert("boat".into(), ReviewTracked::Boat(boat_id));
                tracked.insert("copassenger".into(), ReviewTracked::Mob(copass_id));
                tracked.insert("outside_pig".into(), ReviewTracked::Mob(outside_id));
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:arrow 2.5 64.8 4.5 {{Motion:[2.0d,0.0d,0.0d],damage:2.0d,Owner:{owner},LeftOwner:0b}}"
                        ),
                        seed,
                    )),
                );
            }
            "r07_firework_margin_tick8" => {
                assert!(game.run_command(
                    "summon minecraft:armor_stand 5.7 64.0 5.050000005 {NoGravity:1b}"
                ));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 1.5 64.5 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[0.5d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
            }
            "r07_trident_wall_endpoint" => {
                assert!(game.run_command("setblock 4 64 4 minecraft:stone"));
                assert!(
                    game.run_command("summon minecraft:armor_stand 4.25 64.0 4.75 {NoGravity:1b}")
                );
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "trident".into(),
                    ReviewTracked::Trident(summon_trident_seeded(
                        &mut game,
                        "summon minecraft:trident 3.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r09_pierce0_falling_block_then_zombie" => {
                assert!(game.run_command(
                    r#"summon minecraft:falling_block 2.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
                ));
                tracked.insert(
                    "falling".into(),
                    ReviewTracked::Falling(game.falling.last().unwrap().id),
                );
                tracked.insert(
                    "zombie".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:zombie 3.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 65.3 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d,PierceLevel:0b}",
                        seed,
                    )),
                );
            }
            "r10_pierce_pig_then_player" => {
                if let Some(p) = game.player.as_mut() {
                    p.position = Vec3::new(3.8, 64.0, 4.5);
                    p.velocity = Vec3::ZERO;
                }
                game.tick_server(false).unwrap();
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[3.0d,0.0d,0.0d],damage:2.0d,PierceLevel:2b}",
                        seed,
                    )),
                );
            }
            "r11_pvp_false_arrow" | "r11_pvp_false_trident" | "r11_team_ff_false_trident" => {
                if case.case.starts_with("r11_pvp_false") {
                    assert!(game.run_command("gamerule pvp false"));
                } else {
                    assert!(game.run_command("team add blue"));
                    assert!(game.run_command("team modify blue friendlyFire false"));
                    assert!(game.run_command("team join blue @s"));
                }
                if let Some(p) = game.player.as_mut() {
                    p.position = Vec3::new(3.5, 64.0, 4.5);
                    p.velocity = Vec3::ZERO;
                }
                game.tick_server(false).unwrap();
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 6.8 64.0 4.5 {NoAI:1b,Silent:1b,Health:10.0f}",
                        seed,
                    )),
                );
                if case.case.ends_with("_arrow") {
                    tracked.insert(
                        "proj".into(),
                        ReviewTracked::Proj(summon_proj_seeded(
                            &mut game,
                            &format!(
                                "summon minecraft:arrow 1.5 64.5 4.5 {{Motion:[1.5d,0.0d,0.0d],damage:2.0d,Owner:{owner},LeftOwner:1b}}"
                            ),
                            seed,
                        )),
                    );
                } else {
                    tracked.insert(
                        "proj".into(),
                        ReviewTracked::Trident(summon_trident_seeded(
                            &mut game,
                            &format!(
                                "summon minecraft:trident 1.5 64.5 4.5 {{Motion:[1.5d,0.0d,0.0d],Owner:{owner},LeftOwner:1b}}"
                            ),
                            seed,
                        )),
                    );
                }
            }
            "r12_pierce1_crit_three_zombies" => {
                for (k, x) in [("mob0", "2.5"), ("mob1", "3.5"), ("mob2", "4.5")] {
                    tracked.insert(
                        k.into(),
                        ReviewTracked::Mob(summon_mob_seeded(
                            &mut game,
                            &format!(
                                "summon minecraft:zombie {x} 64.0 4.5 {{NoAI:1b,Silent:1b,Health:20.0f}}"
                            ),
                            seed,
                        )),
                    );
                }
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[4.0d,0.0d,0.0d],damage:2.0d,crit:1b,PierceLevel:1b}",
                        seed,
                    )),
                );
            }
            "r14_breeze_deflects_firework" => {
                tracked.insert(
                    "breeze".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}",
                        seed,
                    )),
                );
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 1.5 64.5 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
            }
            "r14_breeze_deflects_snowball" => {
                tracked.insert(
                    "breeze".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}",
                        seed,
                    )),
                );
                tracked.insert(
                    "snowball".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:snowball 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r14_breeze_arrow_no_repeat" => {
                tracked.insert(
                    "breeze".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}",
                        seed,
                    )),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 3.1 65.0 4.5 {Motion:[0.2d,0.0d,0.0d],NoGravity:1b}",
                        seed,
                    )),
                );
            }
            "r15_trident_singleton_box_entry" => {
                tracked.insert(
                    "chicken".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:chicken 3.6 64.5 4.5 {NoAI:1b,Silent:1b,NoGravity:1b,Health:4.0f}",
                        seed,
                    )),
                );
                assert!(game.run_command("summon minecraft:end_crystal 4.2 64.0 4.5 {}"));
                tracked.insert(
                    "crystal".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                tracked.insert(
                    "trident".into(),
                    ReviewTracked::Trident(summon_trident_seeded(
                        &mut game,
                        "summon minecraft:trident 1.5 65.0 4.5 {Motion:[3.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r16_trident_channeling_stand" => {
                let _ = game.run_command("weather thunder 1000");
                game.world.weather.rain_level = (1.0, 1.0);
                game.world.weather.thunder_level = (1.0, 1.0);
                game.tick_server(false).unwrap();
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "trident".into(),
                    ReviewTracked::Trident(summon_trident_seeded(
                        &mut game,
                        &format!(
                            r#"summon minecraft:trident 1.5 64.5 4.5 {{Motion:[1.0d,0.0d,0.0d],Owner:{owner},LeftOwner:1b,item:{{id:"minecraft:trident",count:1,components:{{"minecraft:enchantments":{{"minecraft:channeling":1}}}}}}}}"#
                        ),
                        seed,
                    )),
                );
            }
            "r17_invuln_enderman_in_boat_flame_arrow" => {
                assert!(game.run_command(
                    r#"summon minecraft:oak_boat 3.5 64.0 4.5 {Passengers:[{id:"minecraft:enderman",NoAI:1b,Silent:1b,Invulnerable:1b}]}"#
                ));
                let em_idx = game.mobs.len() - 1;
                game.mobs[em_idx].random.set_seed(seed);
                tracked.insert("enderman".into(), ReviewTracked::Mob(game.mobs[em_idx].id));
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d],Fire:100s}",
                        seed,
                    )),
                );
            }
            "r18_arrow_knockback_subthreshold" | "r18_arrow_knockback_above_threshold" => {
                tracked.insert(
                    "pig".into(),
                    ReviewTracked::Mob(summon_mob_seeded(
                        &mut game,
                        "summon minecraft:pig 3.5 64.0 4.5 {NoAI:1b,Silent:1b,Health:20.0f}",
                        seed,
                    )),
                );
                let vx = if case.case == "r18_arrow_knockback_subthreshold" {
                    "5.0e-6d"
                } else {
                    "2.0e-5d"
                };
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            r#"summon minecraft:arrow 3.5 65.5 4.5 {{Motion:[{vx},-1.0d,0.0d],damage:1.0d,weapon:{{id:"minecraft:bow",count:1,components:{{"minecraft:enchantments":{{"minecraft:punch":1}}}}}}}}"#
                        ),
                        seed,
                    )),
                );
            }
            "r19_unowned_llama_spit_crystal" => {
                assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tracked.insert(
                    "crystal".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                tracked.insert(
                    "spit".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:llama_spit 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r19_owned_llama_spit_crystal" => {
                let llama_id = summon_mob_seeded(
                    &mut game,
                    "summon minecraft:llama 0.5 64.0 20.5 {NoAI:1b,NoGravity:1b,Silent:1b}",
                    seed,
                );
                let lu = game.mob_uuid(llama_id);
                let llama_uuid = format!("[I;{},{},{},{}]", lu[0], lu[1], lu[2], lu[3]);
                assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tracked.insert(
                    "crystal".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                tracked.insert(
                    "spit".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:llama_spit 1.5 65.0 4.5 {{Motion:[1.0d,0.0d,0.0d],Owner:{llama_uuid},LeftOwner:1b}}"
                        ),
                        seed,
                    )),
                );
            }
            "r19_shulker_bullet_crystal" => {
                assert!(game.run_command("summon minecraft:end_crystal 4.25 64.0 4.5 {}"));
                tracked.insert(
                    "crystal".into(),
                    ReviewTracked::Crystal(game.crystals.last().unwrap().id),
                );
                tracked.insert(
                    "bullet".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:shulker_bullet 1.5 65.0 4.5 {Motion:[1.0d,0.0d,0.0d]}",
                        seed,
                    )),
                );
            }
            "r20_thorns_armor_stand_arrow" => {
                assert!(game.run_command(
                    r#"summon minecraft:armor_stand 3.5 64.0 4.5 {equipment:{chest:{id:"minecraft:iron_chestplate",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}}}}"#
                ));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:arrow 1.5 64.5 4.5 {{Motion:[1.0d,0.0d,0.0d],damage:2.0d,Owner:{owner},LeftOwner:1b}}"
                        ),
                        seed,
                    )),
                );
            }
            "r20_large_fireball_thorns_stand" => {
                assert!(game.run_command(
                    r#"summon minecraft:armor_stand 3.5 64.0 4.5 {Invulnerable:1b,equipment:{head:{id:"minecraft:iron_helmet",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}},chest:{id:"minecraft:iron_chestplate",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}},legs:{id:"minecraft:iron_leggings",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}},feet:{id:"minecraft:iron_boots",count:1,components:{"minecraft:enchantments":{"minecraft:thorns":3}}}}}"#
                ));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "fireball".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:fireball 1.5 64.5 4.5 {{ExplosionPower:1b,acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d],Owner:{owner},LeftOwner:1b}}"
                        ),
                        seed,
                    )),
                );
            }
            "r21_frozen_stand_flame_arrow" => {
                assert!(game.run_command(
                    "summon minecraft:armor_stand 3.5 64.0 4.5 {Invulnerable:1b,TicksFrozen:140}"
                ));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
                tracked.insert(
                    "arrow".into(),
                    ReviewTracked::Proj(summon_proj_seeded(
                        &mut game,
                        "summon minecraft:arrow 1.5 64.5 4.5 {Motion:[1.0d,0.0d,0.0d],Fire:100s}",
                        seed,
                    )),
                );
            }
            "r21_stand_in_fire_neg2" => {
                assert!(game.run_command("setblock 3 64 4 minecraft:fire"));
                assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Fire:-2s}"));
                tracked.insert(
                    "stand".into(),
                    ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                );
            }
            "r23_replacement_knot_drops_old_lead" => {
                assert!(game.run_command("setblock 3 64 4 minecraft:oak_fence"));
                let pig_a = summon_mob_seeded(
                    &mut game,
                    "summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}",
                    seed,
                );
                game.tick_server(false).unwrap();
                let arrow = summon_proj_seeded(
                    &mut game,
                    "summon minecraft:arrow 1.5 64.6 4.5 {Motion:[2.0d,0.0d,0.0d]}",
                    seed,
                );
                game.tick_server(false).unwrap();
                let pig_b = summon_mob_seeded(
                    &mut game,
                    "summon minecraft:pig 3.5 64.0 2.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}",
                    seed,
                );
                tracked.insert("pig_a".into(), ReviewTracked::Mob(pig_a));
                tracked.insert("pig_b".into(), ReviewTracked::Mob(pig_b));
                tracked.insert("arrow".into(), ReviewTracked::Proj(arrow));
            }
            "r25_rocket_first_tick_boat_sound" => {
                assert!(game.run_command("summon minecraft:oak_boat 3.5 64.0 4.5 {}"));
                tracked.insert(
                    "boat".into(),
                    ReviewTracked::Boat(game.boats.last().unwrap().id),
                );
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 2.5 64.3 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[1.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
            }
            "r25_rocket_hits_target_block" => {
                assert!(game.run_command("setblock 4 64 4 minecraft:target"));
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 2.5 64.5 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[2.0d,0.0d,0.0d],FireworksItem:{PLAIN_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
            }
            "r26_rocket_blast_stand_and_mob" | "r26_rocket_blast_mob_then_stand" => {
                if case.case == "r26_rocket_blast_stand_and_mob" {
                    assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                    tracked.insert(
                        "stand".into(),
                        ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                    );
                    tracked.insert(
                        "pig".into(),
                        ReviewTracked::Mob(summon_mob_seeded(
                            &mut game,
                            "summon minecraft:pig 5.5 64.0 5.5 {NoAI:1b,Silent:1b,Health:20.0f}",
                            seed,
                        )),
                    );
                } else {
                    tracked.insert(
                        "pig".into(),
                        ReviewTracked::Mob(summon_mob_seeded(
                            &mut game,
                            "summon minecraft:pig 5.5 64.0 5.5 {NoAI:1b,Silent:1b,Health:20.0f}",
                            seed,
                        )),
                    );
                    assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
                    tracked.insert(
                        "stand".into(),
                        ReviewTracked::Stand(game.armor_stands.last().unwrap().id),
                    );
                }
                tracked.insert(
                    "rocket".into(),
                    ReviewTracked::Rocket(summon_rocket_seeded(
                        &mut game,
                        &format!(
                            "summon minecraft:firework_rocket 3.5 64.5 4.5 {{LifeTime:1,ShotAtAngle:1b,Motion:[0.0d,0.0d,0.0d],FireworksItem:{STAR_FIREWORK_ITEM}}}"
                        ),
                        seed,
                    )),
                );
            }
            name if name.starts_with("r27_") => {
                let rest = name.strip_prefix("r27_").unwrap();
                let (block_kind, proj_kind) = rest.split_once('_').unwrap();
                let block_id = if block_kind == "wool" {
                    "minecraft:white_wool"
                } else {
                    "minecraft:stone"
                };
                assert!(game.run_command(&format!("setblock 4 64 4 {block_id}")));
                assert!(game.run_command("setblock 4 64 6 minecraft:sculk_sensor"));
                game.tick_server(false).unwrap();
                match proj_kind {
                    "arrow" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                "summon minecraft:arrow 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "trident" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Trident(summon_trident_seeded(
                                &mut game,
                                "summon minecraft:trident 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "snowball" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                "summon minecraft:snowball 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "wind_charge" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                "summon minecraft:wind_charge 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "small_fireball" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                "summon minecraft:small_fireball 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "wither_skull" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                "summon minecraft:wither_skull 2.5 64.75 4.5 {acceleration_power:0.0d,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b}",
                                seed,
                            )),
                        );
                    }
                    "splash_potion" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Proj(summon_proj_seeded(
                                &mut game,
                                r#"summon minecraft:splash_potion 2.5 64.75 4.5 {Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b,Item:{id:"minecraft:splash_potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:water"}}}}"#,
                                seed,
                            )),
                        );
                    }
                    "firework_rocket" => {
                        tracked.insert(
                            "proj".into(),
                            ReviewTracked::Rocket(summon_rocket_seeded(
                                &mut game,
                                &format!(
                                    "summon minecraft:firework_rocket 2.5 64.75 4.5 {{LifeTime:20,ShotAtAngle:1b,Motion:[2.0d,0.0d,0.0d],HasBeenShot:1b,FireworksItem:{PLAIN_FIREWORK_ITEM}}}"
                                ),
                                seed,
                            )),
                        );
                    }
                    other => panic!("unexpected r27 projectile {other}"),
                }
            }
            other => panic!("unexpected review_round1 case {other}"),
        }

        for exp in &case.ticks {
            let (events, sounds) = step_recording(&mut game);
            let pl_hp = game.player.as_ref().unwrap().health.health();
            if case.case == "r20_large_fireball_thorns_stand" {
                if (exp.player_health < 20.0) != (pl_hp < 20.0) {
                    failures.push(format!(
                        "{label} t={}: thorns player_health expected <20={} got {pl_hp}",
                        exp.tick,
                        exp.player_health < 20.0
                    ));
                    break;
                }
            } else if pl_hp.to_bits() != exp.player_health.to_bits() {
                failures.push(format!(
                    "{label} t={}: player_health expected {} got {pl_hp}",
                    exp.tick, exp.player_health
                ));
                break;
            }
            if game.bolts.len() != exp.lightning_count {
                failures.push(format!(
                    "{label} t={}: lightning_count expected {} got {}",
                    exp.tick,
                    exp.lightning_count,
                    game.bolts.len()
                ));
                break;
            }
            if (exp.xp_orb_count > 0) != (!game.orbs.is_empty()) {
                failures.push(format!(
                    "{label} t={}: xp_orb presence expected {} got {}",
                    exp.tick,
                    exp.xp_orb_count,
                    game.orbs.len()
                ));
                break;
            }
            let target_power = game
                .pack
                .assets
                .registry
                .state(game.world.block(4, 64, 4))
                .and_then(|s| s.properties.get("power"))
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(-1);
            if target_power != exp.target_power {
                failures.push(format!(
                    "{label} t={}: target_power expected {} got {target_power}",
                    exp.tick, exp.target_power
                ));
                break;
            }
            let sculk_phase = game
                .pack
                .assets
                .registry
                .state(game.world.block(4, 64, 6))
                .and_then(|s| s.properties.get("sculk_sensor_phase"))
                .cloned();
            if sculk_phase != exp.sculk_phase {
                failures.push(format!(
                    "{label} t={}: sculk_phase expected {:?} got {sculk_phase:?}",
                    exp.tick, exp.sculk_phase
                ));
                break;
            }
            let mut entity_ok = true;
            for (k, exp_e) in &exp.entities {
                let tr = *tracked
                    .get(k)
                    .unwrap_or_else(|| panic!("{label}: missing {k}"));
                if !check_review_entity(&game, &label, exp.tick, k, tr, exp_e, &mut failures) {
                    entity_ok = false;
                    break;
                }
            }
            if !entity_ok {
                break;
            }
            if let Err(msg) = land_positions_match(&events, &exp.game_events) {
                failures.push(format!("{label} t={}: {msg}", exp.tick));
                break;
            }
            if case.case == "r19_shulker_bullet_crystal" {
                let exp_has_destroy = exp
                    .game_events
                    .iter()
                    .rposition(|e| e.id == "minecraft:entity_damage")
                    .zip(
                        exp.game_events
                            .iter()
                            .position(|e| e.id == "minecraft:projectile_land"),
                    )
                    .is_some_and(|(d, l)| d > l);
                let got_has_destroy = events
                    .iter()
                    .rposition(|&(id, _, _)| id == "minecraft:entity_damage")
                    .zip(
                        events
                            .iter()
                            .position(|&(id, _, _)| id == "minecraft:projectile_land"),
                    )
                    .is_some_and(|(d, l)| d > l);
                if got_has_destroy != exp_has_destroy {
                    failures.push(format!(
                        "{label} t={}: shulker_bullet destroy entity_damage expected {exp_has_destroy} got {got_has_destroy}",
                        exp.tick
                    ));
                    break;
                }
            }
            if case.case.starts_with("r26_") {
                let exp_order: Vec<&str> = exp
                    .game_events
                    .iter()
                    .map(|e| e.id.as_str())
                    .filter(|&id| id == "minecraft:entity_die" || id == "minecraft:entity_damage")
                    .collect();
                let got_order: Vec<&str> = events
                    .iter()
                    .map(|&(id, _, _)| id)
                    .filter(|&id| id == "minecraft:entity_die" || id == "minecraft:entity_damage")
                    .collect();
                if got_order != exp_order {
                    failures.push(format!(
                        "{label} t={}: blast damage order expected {exp_order:?} got {got_order:?}",
                        exp.tick
                    ));
                    break;
                }
            }
            let mut got_drops: Vec<String> = game
                .dropped
                .iter()
                .map(|d| d.stack.item.to_string())
                .filter(|item| item != "minecraft:cobblestone")
                .collect();
            got_drops.sort();
            let mut exp_drops: Vec<String> = exp
                .drops
                .iter()
                .filter(|item| item.as_str() != "minecraft:cobblestone")
                .cloned()
                .collect();
            exp_drops.sort();
            if got_drops != exp_drops {
                failures.push(format!(
                    "{label} t={}: drops expected {exp_drops:?} got {got_drops:?}",
                    exp.tick
                ));
                break;
            }
            let mut snd_err = false;
            for snd in [
                "minecraft:entity.arrow.hit",
                "minecraft:item.trident.hit",
                "minecraft:entity.breeze.deflect",
                "minecraft:entity.firework_rocket.launch",
                "minecraft:entity.armor_stand.break",
                "minecraft:enchant.thorns.hit",
            ] {
                let exp_n = exp.sounds.iter().filter(|s| s.as_str() == snd).count();
                let got_n = sounds.get(snd).copied().unwrap_or(0);
                if got_n != exp_n {
                    failures.push(format!(
                        "{label} t={}: {snd} count expected {exp_n} got {got_n}",
                        exp.tick
                    ));
                    snd_err = true;
                    break;
                }
            }
            if snd_err {
                break;
            }
        }
    }

    // Finding #15: also verify mob-thrown ProjectileKind::Trident selects crystal box entry over chicken center.
    let r15 = witness
        .review_round1
        .iter()
        .find(|c| c.case == "r15_trident_singleton_box_entry")
        .unwrap();
    clear_arena(&mut game, 10);
    game.world.random.set_seed(witness.level_seed);
    let _chicken = summon_mob_seeded(
        &mut game,
        "summon minecraft:chicken 3.6 64.5 4.5 {NoAI:1b,Silent:1b,NoGravity:1b,Health:4.0f}",
        witness.proj_seed,
    );
    assert!(game.run_command("summon minecraft:end_crystal 4.2 64.0 4.5 {}"));
    let crystal_id = game.crystals.last().unwrap().id;
    let mut proj = Projectile::new(ProjectileKind::Trident, Vec3::new(1.5, 65.0, 4.5), None);
    proj.id = game.take_entity_id();
    proj.velocity = Vec3::new(3.0, 0.0, 0.0);
    proj.entity_random().set_seed(witness.proj_seed);
    let mob_tr_id = proj.id;
    game.creatures.projectiles.push(proj);
    for exp in &r15.ticks {
        let (events, _) = step_recording(&mut game);
        if let Err(msg) = land_positions_match(&events, &exp.game_events) {
            failures.push(format!("review r15_mob_trident t={}: {msg}", exp.tick));
            break;
        }
        if !check_review_entity(
            &game,
            "review r15_mob_trident",
            exp.tick,
            "crystal",
            ReviewTracked::Crystal(crystal_id),
            &exp.entities["crystal"],
            &mut failures,
        ) || !check_review_entity(
            &game,
            "review r15_mob_trident",
            exp.tick,
            "trident",
            ReviewTracked::Proj(mob_tr_id),
            &exp.entities["trident"],
            &mut failures,
        ) {
            break;
        }
    }

    assert!(
        failures.is_empty(),
        "review round-1 witness failures:\n{}",
        failures.join("\n")
    );
}
