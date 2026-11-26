//! Witness-driven parity tests for projectiles hitting non-mob pickable entities (#119).
use crate::game::{Game, commands::Target};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use sunrise_client::mobs::{Who, leash::Holder};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_core::random::{LegacyRandom, RandomSource};

#[derive(Deserialize)]
struct Witness {
    game_version: String,
    projectile_seed: i64,
    level_seed: i64,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    projectile: String,
    target: String,
    spawn_y: f64,
    target_initial_box: [f64; 6],
    hit_tick: Option<usize>,
    ticks: Vec<TickSnap>,
}

#[derive(Deserialize)]
struct TickSnap {
    tick: usize,
    projectile: ProjSnap,
    target: TargetSnap,
    game_events: Vec<GameEventSnap>,
    drops: Vec<DropSnap>,
}

#[derive(Deserialize)]
struct ProjSnap {
    alive: bool,
    pos_bits: [String; 3],
    vel_bits: [String; 3],
    in_ground: Option<bool>,
    dealt_damage: Option<bool>,
}

#[derive(Deserialize)]
struct TargetSnap {
    alive: bool,
    fire_ticks: i32,
    item: Option<String>,
    block_at_floor: Option<String>,
    pig_leashed: Option<bool>,
}

#[derive(Deserialize)]
struct GameEventSnap {
    id: String,
    pos_bits: [String; 3],
    source: Option<String>,
}

#[derive(Deserialize)]
struct DropSnap {
    item: String,
    count: u32,
}

fn load_witness() -> Witness {
    let dir = std::env::var_os("SUNRISE_PROJECTILE_WITNESS_DIR").unwrap_or_else(|| {
        panic!(
            "SUNRISE_PROJECTILE_WITNESS_DIR is unset; run tools/test_projectile_targets_parity.py"
        )
    });
    let path = PathBuf::from(dir).join("witness.json");
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

fn vec_bits(v: Vec3) -> [String; 3] {
    [bits(v.x), bits(v.y), bits(v.z)]
}

fn setup_game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("difficulty normal"));
    // The Java oracle has a live player as the owner of its creative arrows and fireballs.
    let mut player =
        sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 12.5)).unwrap();
    player.client_loaded = true;
    game.player = Some(player);
    crate::game::native_entity_refs::tests::bind_local_player(&mut game, 1, 1);
    for command in [
        "gamerule spawn_mobs false",
        "gamerule advance_time false",
        "gamerule advance_weather false",
        "gamerule mob_griefing true",
        "gamerule entity_drops true",
        "gamerule random_tick_speed 0",
    ] {
        let _ = game.run_command(command);
    }
    game
}

fn clear_entities(game: &mut Game<'_>) {
    game.mobs.clear();
    game.dropped.clear();
    game.boats.clear();
    game.carts.clear();
    game.armor_stands.clear();
    game.tnts.clear();
    game.falling.clear();
    game.orbs.clear();
    game.creatures.projectiles.clear();
    game.rockets.clear();
    game.tridents.clear();
    game.item_frames.clear();
    game.paintings.clear();
    game.cushions.clear();
    game.others.clear();
    game.knot_ids.clear();
    game.broken_knots.clear();
    game.resident_entities.knots.clear();
    game.reset_entity_residency();
    game.bind_local_player_residency();
}

fn reset_arena(game: &mut Game<'_>, target: &str) {
    clear_entities(game);
    let _ = game.run_command("fill 0 63 0 9 63 9 minecraft:stone");
    let _ = game.run_command("fill 0 64 0 9 70 9 minecraft:air");
    let _ = game.run_command("fill 7 64 2 7 67 6 minecraft:stone");
    let wall = target == "painting"
        || target.starts_with("item_frame")
        || target.starts_with("glow_item_frame");
    if wall {
        let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
    }
    if target.starts_with("leash_knot") {
        let _ = game.run_command("setblock 3 64 4 minecraft:oak_fence");
    }
    game.set_mode(sunrise_client::command::args::GameMode::Spectator);
    if let Some(p) = game.player.as_mut() {
        p.position = Vec3::new(0.5, 64.0, 12.5);
        p.velocity = Vec3::ZERO;
    }
    game.tick_server(false).unwrap();
}

#[derive(Clone, Copy)]
enum TrackedTarget {
    Stand(u32),
    Painting(u32),
    Frame(u32),
    Cushion(u32),
    Falling(u32),
    ExplicitKnot(u32),
    NaturalKnot { id: u32, pig_id: u32 },
}

#[derive(Clone, Copy)]
enum TrackedProj {
    Projectile(u32),
    Trident(u32),
}

fn summon_target(game: &mut Game<'_>, target: &str) -> TrackedTarget {
    match target {
        "armor_stand" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_equipped" => {
            assert!(game.run_command(
                r#"summon minecraft:armor_stand 3.5 64.0 4.5 {equipment:{head:{id:"minecraft:iron_helmet",count:1}}}"#
            ));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_marker" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_small" | "armor_stand_small_high" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Small:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_nogravity" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {NoGravity:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_invisible" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Invisible:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "painting" => {
            assert!(game.run_command(
                r#"summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:"minecraft:kebab"}"#
            ));
            TrackedTarget::Painting(game.paintings.last().unwrap().id)
        }
        "item_frame" => {
            assert!(game.run_command(
                r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "item_frame_empty" => {
            assert!(game.run_command(
                "summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b}"
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "item_frame_fixed" => {
            assert!(game.run_command(
                r#"summon minecraft:item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Fixed:1b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "glow_item_frame" => {
            assert!(game.run_command(
                r#"summon minecraft:glow_item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b,Item:{id:"minecraft:diamond",count:1}}"#
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "glow_item_frame_empty" => {
            assert!(game.run_command(
                "summon minecraft:glow_item_frame 3.5 64.5 4.5 {block_pos:[I;3,64,4],Facing:4b}"
            ));
            TrackedTarget::Frame(game.item_frames.last().unwrap().id)
        }
        "leash_knot" => {
            assert!(game.run_command("summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
            TrackedTarget::ExplicitKnot(game.others.last().unwrap().id)
        }
        "leash_knot_tied" => {
            assert!(game.run_command(
                "summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}"
            ));
            let pig_id = game.mobs.last().unwrap().id;
            game.tick_server(false).unwrap();
            let id = *game
                .knot_ids
                .get(&[3, 64, 4])
                .expect("natural leash knot created on fence");
            TrackedTarget::NaturalKnot { id, pig_id }
        }
        "falling_block" => {
            assert!(game.run_command(
                r#"summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
            ));
            TrackedTarget::Falling(game.falling.last().unwrap().id)
        }
        "cushion" => {
            assert!(game.run_command(
                r#"summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:"white"}"#
            ));
            TrackedTarget::Cushion(game.cushions.last().unwrap().id)
        }
        other => panic!("unexpected witness target {other}"),
    }
}

fn target_bounds(game: &Game<'_>, target: TrackedTarget) -> Aabb {
    match target {
        TrackedTarget::Stand(id) => {
            let a = game.armor_stands.iter().find(|a| a.id == id).unwrap();
            if a.marker {
                Aabb::new(a.position, a.position)
            } else {
                a.bounds()
            }
        }
        TrackedTarget::Painting(id) => game
            .selector_target_bounds(Target::Painting(id))
            .expect("painting bounds"),
        TrackedTarget::Frame(id) => game
            .selector_target_bounds(Target::Frame(id))
            .expect("frame bounds"),
        TrackedTarget::Cushion(id) => game
            .selector_target_bounds(Target::Cushion(id))
            .expect("cushion bounds"),
        TrackedTarget::Falling(id) => game
            .selector_target_bounds(Target::Falling(id))
            .expect("falling block bounds"),
        TrackedTarget::ExplicitKnot(id) => {
            let o = game.others.iter().find(|o| o.id == id).unwrap();
            let b = sunrise_core::position::BlockPos::containing(o.position);
            let k = sunrise_client::mobs::leash::knot_state([b.x, b.y, b.z]);
            let half = k.width / 2.0;
            Aabb::new(
                Vec3::new(k.position.x - half, k.position.y, k.position.z - half),
                Vec3::new(
                    k.position.x + half,
                    k.position.y + k.height,
                    k.position.z + half,
                ),
            )
        }
        TrackedTarget::NaturalKnot { id, .. } => game
            .selector_target_bounds(Target::Knot(id))
            .expect("natural knot bounds"),
    }
}

fn summon_projectile(
    game: &mut Game<'_>,
    proj: &str,
    y0: f64,
    proj_seed: i64,
) -> (TrackedProj, Option<u32>) {
    if proj == "arrow_skeleton" {
        assert!(game.run_command("summon minecraft:skeleton 0.5 64.0 2.5 {NoAI:1b,Silent:1b}"));
        let skel_id = game.mobs.last().unwrap().id;
        let skel_uuid = Some(game.mob_uuid(skel_id));
        let cmd =
            format!("summon minecraft:arrow 1.5 {y0} 4.5 {{pickup:1b,Motion:[1.0d,0.0d,0.0d]}}");
        assert!(game.run_command(&cmd), "{cmd}");
        let idx = game.creatures.projectiles.len() - 1;
        let id = game.creatures.projectiles[idx].id;
        game.creatures.projectiles.update(idx, |p| {
            p.entity_random().set_seed(proj_seed);
            p.owner = Some(Who::Mob(skel_id));
            p.owner_uuid = skel_uuid;
            p.left_owner = true;
        });
        return (TrackedProj::Projectile(id), Some(skel_id));
    }
    let owned = proj.ends_with("_owned") || proj.ends_with("_creative");
    if proj.ends_with("_creative") {
        game.set_mode(sunrise_client::command::args::GameMode::Creative);
    } else if proj.ends_with("_owned") {
        game.set_mode(sunrise_client::command::args::GameMode::Survival);
    }
    let base = proj
        .trim_end_matches("_creative")
        .trim_end_matches("_owned");
    let cmd = match base {
        "arrow" => {
            format!("summon minecraft:arrow 1.5 {y0} 4.5 {{pickup:1b,Motion:[1.0d,0.0d,0.0d]}}")
        }
        "spectral_arrow" => format!(
            "summon minecraft:spectral_arrow 1.5 {y0} 4.5 {{pickup:1b,Motion:[1.0d,0.0d,0.0d]}}"
        ),
        "trident" => format!(
            r#"summon minecraft:trident 1.5 {y0} 4.5 {{pickup:1b,Motion:[1.0d,0.0d,0.0d],item:{{id:"minecraft:trident",count:1}}}}"#
        ),
        "snowball" => format!("summon minecraft:snowball 1.5 {y0} 4.5 {{Motion:[1.0d,0.0d,0.0d]}}"),
        "egg" => format!("summon minecraft:egg 1.5 {y0} 4.5 {{Motion:[1.0d,0.0d,0.0d]}}"),
        "ender_pearl" => {
            format!("summon minecraft:ender_pearl 1.5 {y0} 4.5 {{Motion:[1.0d,0.0d,0.0d]}}")
        }
        "wind_charge" => format!(
            "summon minecraft:wind_charge 1.5 {y0} 4.5 {{acceleration_power:0.0d,Motion:[1.0d,0.0d,0.0d]}}"
        ),
        "small_fireball" => format!(
            "summon minecraft:small_fireball 1.5 {y0} 4.5 {{acceleration_power:0.1d,Motion:[1.0d,0.0d,0.0d]}}"
        ),
        "fireball" => format!(
            "summon minecraft:fireball 1.5 {y0} 4.5 {{ExplosionPower:1b,acceleration_power:0.1d,Motion:[1.0d,0.0d,0.0d]}}"
        ),
        other => panic!("unexpected witness projectile {other}"),
    };
    assert!(game.run_command(&cmd), "{cmd}");
    if base == "trident" {
        let idx = game.tridents.len() - 1;
        let id = game.tridents[idx].id;
        let owner_uuid = game.level.player_uuid;
        game.tridents.update(idx, |t| {
            t.random = LegacyRandom::new(proj_seed);
            if owned {
                t.owner_uuid = owner_uuid;
            }
        });
        (TrackedProj::Trident(id), None)
    } else {
        let idx = game.creatures.projectiles.len() - 1;
        let id = game.creatures.projectiles[idx].id;
        let owner_uuid = game.level.player_uuid;
        game.creatures.projectiles.update(idx, |p| {
            p.entity_random().set_seed(proj_seed);
            if owned {
                p.owner = Some(Who::Player);
                p.owner_uuid = owner_uuid;
            }
        });
        (TrackedProj::Projectile(id), None)
    }
}

fn target_alive(game: &Game<'_>, target: TrackedTarget) -> bool {
    match target {
        TrackedTarget::Stand(id) => game.armor_stands.iter().any(|a| a.id == id),
        TrackedTarget::Painting(id) => game.paintings.iter().any(|p| p.id == id),
        TrackedTarget::Frame(id) => game.item_frames.iter().any(|f| f.id == id),
        TrackedTarget::Cushion(id) => game.cushions.iter().any(|c| c.id == id),
        TrackedTarget::Falling(id) => game.falling.iter().any(|f| f.id == id),
        TrackedTarget::ExplicitKnot(id) => game.others.iter().any(|o| o.id == id),
        TrackedTarget::NaturalKnot { id, .. } => {
            game.resident_entities.knots.iter().any(|k| k.id == id)
        }
    }
}

fn check_case(game: &mut Game<'_>, case: &Case, proj_seed: i64, level_seed: i64) -> Vec<String> {
    let label = format!("{} x {}", case.target, case.projectile);
    let mut errors = Vec::new();
    reset_arena(game, &case.target);
    game.world.random.set_seed(level_seed);

    let tracked_target = summon_target(game, &case.target);
    let box_now = target_bounds(game, tracked_target);
    let actual_box = [
        box_now.min_position().x,
        box_now.min_position().y,
        box_now.min_position().z,
        box_now.max_position().x,
        box_now.max_position().y,
        box_now.max_position().z,
    ];
    for k in 0..6 {
        if actual_box[k].to_bits() != case.target_initial_box[k].to_bits() {
            errors.push(format!(
                "{label}: initial box[{k}] expected {} ({:016x}) got {} ({:016x})",
                case.target_initial_box[k],
                case.target_initial_box[k].to_bits(),
                actual_box[k],
                actual_box[k].to_bits(),
            ));
        }
    }

    let (tracked_proj, skel_id) =
        summon_projectile(game, &case.projectile, case.spawn_y, proj_seed);
    game.world.random.set_seed(level_seed);
    let mut seen_drops: std::collections::HashSet<u32> =
        game.dropped.iter().map(|d| d.id).collect();
    let mut expected_drops: BTreeMap<String, u32> = BTreeMap::new();
    let mut actual_drops: BTreeMap<String, u32> = BTreeMap::new();

    for expected in &case.ticks[1..] {
        super::super::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
        game.tick_server(false).unwrap();
        let posted = super::super::sculk::TEST_POSTED_EVENTS
            .with(|e| e.borrow_mut().take().unwrap_or_default());

        // Record new non-terrain item drops surviving the tick.
        for d in &game.dropped {
            if seen_drops.insert(d.id) {
                let item = d.stack.item.to_string();
                if item != "minecraft:oak_fence" && item != "minecraft:cobblestone" {
                    *actual_drops.entry(item).or_default() += d.stack.count;
                }
            }
        }
        for d in &expected.drops {
            if d.item != "minecraft:oak_fence" && d.item != "minecraft:cobblestone" {
                *expected_drops.entry(d.item.clone()).or_default() += d.count;
            }
        }

        // Check projectile alive and bit-exact kinematics while alive.
        match tracked_proj {
            TrackedProj::Projectile(id) => {
                let live = game.creatures.projectiles.iter().find(|p| p.id == id);
                let alive = live.is_some();
                if alive != expected.projectile.alive {
                    errors.push(format!(
                        "{label} t={}: projectile alive expected {} got {alive}",
                        expected.tick, expected.projectile.alive
                    ));
                }
                if let Some(p) = live {
                    let pos_b = vec_bits(p.position);
                    let vel_b = vec_bits(p.velocity);
                    if pos_b != expected.projectile.pos_bits {
                        errors.push(format!(
                            "{label} t={}: proj pos expected {:?} got {:?} ({:?})",
                            expected.tick, expected.projectile.pos_bits, pos_b, p.position
                        ));
                    }
                    if vel_b != expected.projectile.vel_bits {
                        errors.push(format!(
                            "{label} t={}: proj vel expected {:?} got {:?} ({:?})",
                            expected.tick, expected.projectile.vel_bits, vel_b, p.velocity
                        ));
                    }
                    if let Some(exp_ig) = expected.projectile.in_ground {
                        let ig = p.arrow.as_ref().is_some_and(|a| a.in_ground);
                        if ig != exp_ig {
                            errors.push(format!(
                                "{label} t={}: arrow in_ground expected {exp_ig} got {ig}",
                                expected.tick
                            ));
                        }
                    }
                }
            }
            TrackedProj::Trident(id) => {
                let live = game.tridents.iter().find(|t| t.id == id);
                let alive = live.is_some();
                if alive != expected.projectile.alive {
                    errors.push(format!(
                        "{label} t={}: trident alive expected {} got {alive}",
                        expected.tick, expected.projectile.alive
                    ));
                }
                if let Some(t) = live {
                    let pos_b = vec_bits(t.position);
                    let vel_b = vec_bits(t.velocity);
                    if pos_b != expected.projectile.pos_bits {
                        errors.push(format!(
                            "{label} t={}: trident pos expected {:?} got {:?} ({:?})",
                            expected.tick, expected.projectile.pos_bits, pos_b, t.position
                        ));
                    }
                    if vel_b != expected.projectile.vel_bits {
                        errors.push(format!(
                            "{label} t={}: trident vel expected {:?} got {:?} ({:?})",
                            expected.tick, expected.projectile.vel_bits, vel_b, t.velocity
                        ));
                    }
                    if let Some(exp_ig) = expected.projectile.in_ground
                        && t.in_ground != exp_ig
                    {
                        errors.push(format!(
                            "{label} t={}: trident in_ground expected {exp_ig} got {}",
                            expected.tick, t.in_ground
                        ));
                    }
                    if let Some(exp_dd) = expected.projectile.dealt_damage
                        && t.dealt_damage != exp_dd
                    {
                        errors.push(format!(
                            "{label} t={}: trident dealt_damage expected {exp_dd} got {}",
                            expected.tick, t.dealt_damage
                        ));
                    }
                }
            }
        }

        // Check target liveness and target-specific state.
        let t_alive = target_alive(game, tracked_target);
        if t_alive != expected.target.alive {
            errors.push(format!(
                "{label} t={}: target alive expected {} got {t_alive}",
                expected.tick, expected.target.alive
            ));
        }
        match tracked_target {
            TrackedTarget::Stand(id) => {
                if let Some(a) = game.armor_stands.iter().find(|a| a.id == id)
                    && a.fire_ticks != expected.target.fire_ticks
                {
                    errors.push(format!(
                        "{label} t={}: stand fire_ticks expected {} got {}",
                        expected.tick, expected.target.fire_ticks, a.fire_ticks
                    ));
                }
            }
            TrackedTarget::Frame(id) => {
                if let Some(f) = game.item_frames.iter().find(|f| f.id == id) {
                    let item = f.item.as_ref().map(|s| s.item.to_string());
                    if item != expected.target.item {
                        errors.push(format!(
                            "{label} t={}: frame item expected {:?} got {:?}",
                            expected.tick, expected.target.item, item
                        ));
                    }
                }
            }
            TrackedTarget::Falling(_) => {
                if expected.target.block_at_floor.as_deref() == Some("minecraft:sand") {
                    let block = &*game.res.facts().get(game.world.block(3, 64, 4)).name;
                    if block != "minecraft:sand" {
                        errors.push(format!(
                            "{label} t={}: floor block expected minecraft:sand got {block}",
                            expected.tick
                        ));
                    }
                }
            }
            TrackedTarget::NaturalKnot { pig_id, .. } => {
                if expected.tick == case.ticks.len() - 1
                    && let Some(exp_leashed) = expected.target.pig_leashed
                {
                    let leashed = game
                        .mobs
                        .iter()
                        .find(|m| m.id == pig_id)
                        .is_some_and(|m| matches!(m.extra.leash, Some(Holder::Knot(_))));
                    if leashed != exp_leashed {
                        errors.push(format!(
                            "{label} t={}: pig leashed expected {exp_leashed} got {leashed}",
                            expected.tick
                        ));
                    }
                }
            }
            _ => {}
        }

        // Check exact projectile_land positions on every tick (both directions).
        let exp_lands: Vec<&[String; 3]> = expected
            .game_events
            .iter()
            .filter(|g| g.id == "minecraft:projectile_land")
            .map(|g| &g.pos_bits)
            .collect();
        let act_lands: Vec<[String; 3]> = posted
            .iter()
            .filter(|(id, _, _)| *id == "minecraft:projectile_land")
            .map(|(_, at, _)| [bits(at[0]), bits(at[1]), bits(at[2])])
            .collect();
        if act_lands.len() != exp_lands.len()
            || act_lands.iter().zip(&exp_lands).any(|(a, e)| a != *e)
        {
            errors.push(format!(
                "{label} t={}: projectile_land pos_bits expected {exp_lands:?} got {act_lands:?}",
                expected.tick
            ));
        }

        // Check key game events on hit tick (projectile_land, entity_die, block_change + source).
        if case.hit_tick == Some(expected.tick) {
            let important = |id: &str| {
                matches!(
                    id,
                    "minecraft:entity_die" | "minecraft:block_change" | "minecraft:projectile_land"
                )
            };
            let exp_ev: Vec<&GameEventSnap> = expected
                .game_events
                .iter()
                .filter(|g| important(g.id.as_str()))
                .collect();
            let act_ev: Vec<(&str, Option<Who>)> = posted
                .iter()
                .filter(|(id, _, _)| important(id))
                .map(|(id, _, who)| (*id, *who))
                .collect();
            let exp_ids: Vec<&str> = exp_ev.iter().map(|g| g.id.as_str()).collect();
            let act_ids: Vec<&str> = act_ev.iter().map(|(id, _)| *id).collect();
            if act_ids != exp_ids {
                errors.push(format!(
                    "{label} t={}: game_events expected {exp_ids:?} got {act_ids:?}",
                    expected.tick
                ));
            } else {
                for (exp_g, (_, act_who)) in exp_ev.iter().zip(&act_ev) {
                    if exp_g.id == "minecraft:block_change" {
                        let expected_who = match exp_g.source.as_deref() {
                            Some("minecraft:skeleton") => skel_id.map(Who::Mob),
                            Some("player") => Some(Who::Player),
                            _ => None,
                        };
                        if let Some(want) = expected_who
                            && *act_who != Some(want)
                        {
                            errors.push(format!(
                                "{label} t={}: block_change source expected {want:?} got {act_who:?}",
                                expected.tick
                            ));
                        }
                    }
                }
            }
        }
        if !errors.is_empty() {
            break;
        }
    }

    if errors.is_empty() && actual_drops != expected_drops {
        errors.push(format!(
            "{label}: drops expected {expected_drops:?} got {actual_drops:?}"
        ));
    }
    errors
}

fn run_family(filter: impl Fn(&str) -> bool) {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();
    let mut ran = 0usize;
    for case in witness.cases.iter().filter(|c| filter(&c.target)) {
        ran += 1;
        failures.extend(check_case(
            &mut game,
            case,
            witness.projectile_seed,
            witness.level_seed,
        ));
    }
    assert!(ran > 0, "no witness cases matched filter");
    assert!(
        failures.is_empty(),
        "{} / {ran} witness cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_armor_stand_targets() {
    run_witness_armor_stand_targets();
}

pub(in crate::game) fn run_witness_armor_stand_targets() {
    run_family(|t| t.starts_with("armor_stand"));
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_painting_and_cushion_targets() {
    run_witness_painting_and_cushion_targets();
}

pub(in crate::game) fn run_witness_painting_and_cushion_targets() {
    run_family(|t| t == "painting" || t == "cushion");
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_item_frame_targets() {
    run_witness_item_frame_targets();
}

pub(in crate::game) fn run_witness_item_frame_targets() {
    run_family(|t| t.starts_with("item_frame") || t.starts_with("glow_item_frame"));
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_leash_knot_targets() {
    run_witness_leash_knot_targets();
}

pub(in crate::game) fn run_witness_leash_knot_targets() {
    run_family(|t| t.starts_with("leash_knot"));
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_falling_block_targets() {
    run_witness_falling_block_targets();
}

pub(in crate::game) fn run_witness_falling_block_targets() {
    run_family(|t| t == "falling_block");
}
