//! Witness-driven parity tests for FishingHook hitting pickable targets and ItemEntities.
use super::{Hook, State};
use crate::game::{Game, commands::Target};
use serde::Deserialize;
use std::path::PathBuf;
use sunrise_client::items::ItemStack;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};

#[derive(Deserialize)]
struct FishingWitness {
    game_version: String,
    hook_seed: i64,
    level_seed: i64,
    cases: Vec<FishingCase>,
    extra_cases: Vec<FishingExtraCase>,
}

#[derive(Deserialize)]
struct FishingCase {
    target: String,
    spawn_y: f64,
    target_id_mod4: u32,
    ticks: Vec<FishingTick>,
    retrieve_damage: i32,
    target_vel_after_retrieve_bits: [String; 3],
    target_pos_after_retrieve_tick_bits: [String; 3],
}

#[derive(Deserialize)]
struct FishingTick {
    tick: usize,
    hook_state: String,
    hook_pos_bits: [String; 3],
    hook_vel_bits: [String; 3],
    hook_yaw_bits: String,
    hooked_type: Option<String>,
    #[allow(dead_code)]
    target_vel_bits: [String; 3],
    game_events: Vec<GameEventSnap>,
    #[serde(default)]
    sounds: Vec<String>,
}

#[derive(Deserialize)]
struct GameEventSnap {
    id: String,
    pos_bits: [String; 3],
}

#[derive(Deserialize)]
struct FishingExtraCase {
    case: String,
    target_id_mod4: Option<u32>,
    retrieve_on_tick: Option<usize>,
    state_at_retrieve: Option<String>,
    retrieve_damage: Option<i32>,
    target_vel_after_retrieve_bits: Option<[String; 3]>,
    target_pos_after_retrieve_tick_bits: Option<[String; 3]>,
    target_vel_after_retrieve_tick_bits: Option<[String; 3]>,
    ticks: Vec<FishingTick>,
}

fn load_witness() -> FishingWitness {
    let dir = std::env::var_os("SUNRISE_PROJECTILE_WITNESS_DIR").unwrap_or_else(|| {
        panic!(
            "SUNRISE_PROJECTILE_WITNESS_DIR is unset; run tools/test_projectile_targets_parity.py"
        )
    });
    let path = PathBuf::from(dir).join("fishing_witness.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read fishing witness {} (record via tools/test_projectile_targets_parity.py): {e}",
            path.display()
        )
    });
    let witness: FishingWitness = serde_json::from_str(&raw).expect("valid 26.3 fishing witness");
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

fn align_next_entity_mod4(game: &mut Game<'_>, want_mod4: u32) {
    while ((game.next_entity + 1) & 3) != (want_mod4 & 3) {
        game.next_entity += 1;
    }
}

fn setup_game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let mut game = crate::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("difficulty normal"));
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
    game.player = Some(
        sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 4.5)).unwrap(),
    );
    crate::game::native_entity_refs::tests::bind_local_player(&mut game, 7, 7);
    game.next_entity = game.next_entity.max(100);
    game
}

fn clear_entities(game: &mut Game<'_>) {
    game.hook.discard();
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
    game.crystals.clear();
    game.discard_resident_dragon();
    game.others.clear();
    game.knot_ids.clear();
    game.broken_knots.clear();
    game.resident_entities.knots.clear();
    game.reset_entity_residency();
    game.bind_local_player_residency();
}

fn reset_arena(game: &mut Game<'_>, target: &str) {
    clear_entities(game);
    let _ = game.run_command("fill 0 63 0 11 63 9 minecraft:stone");
    let _ = game.run_command("fill 0 64 0 11 70 9 minecraft:air");
    let _ = game.run_command("fill 10 64 2 10 67 6 minecraft:stone");
    let wall = target == "painting"
        || target.starts_with("item_frame")
        || target.starts_with("glow_item_frame");
    if wall {
        let _ = game.run_command("fill 4 64 2 4 66 6 minecraft:stone");
    }
    if target.starts_with("leash_knot") {
        let _ = game.run_command("setblock 3 64 4 minecraft:oak_fence");
    }
    if game.riding.is_some() {
        game.stop_riding();
    }
    game.set_mode(sunrise_client::command::args::GameMode::Survival);
    game.yaw = -90.0;
    game.pitch = 0.0;
    if let Some(p) = game.player.as_mut() {
        p.position = Vec3::new(0.5, 64.0, 4.5);
        p.velocity = Vec3::ZERO;
    }
    game.selected = 0;
    game.inventory[0] = Some(ItemStack::new("minecraft:fishing_rod", 1));
    game.tick_server(false).unwrap();
    game.audio.heard.clear();
}

#[derive(Clone, Copy)]
enum TrackedTarget {
    Frame(u32),
    Painting(u32),
    Falling(u32),
    Crystal(u32),
    Dragon,
    Stand(u32),
    ExplicitKnot(u32),
    NaturalKnot(u32),
    Cushion(u32),
    Item(u32),
}

impl TrackedTarget {
    fn id(self) -> Option<u32> {
        match self {
            Self::Frame(id)
            | Self::Painting(id)
            | Self::Falling(id)
            | Self::Crystal(id)
            | Self::Stand(id)
            | Self::ExplicitKnot(id)
            | Self::NaturalKnot(id)
            | Self::Cushion(id)
            | Self::Item(id) => Some(id),
            Self::Dragon => None,
        }
    }
}

fn summon_target(game: &mut Game<'_>, target: &str) -> TrackedTarget {
    match target {
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
        "painting" => {
            assert!(game.run_command(
                r#"summon minecraft:painting 3.5 64.5 4.5 {block_pos:[I;3,64,4],facing:1b,variant:"minecraft:kebab"}"#
            ));
            TrackedTarget::Painting(game.paintings.last().unwrap().id)
        }
        "falling_block" => {
            assert!(game.run_command(
                r#"summon minecraft:falling_block 3.5 65.0 4.5 {BlockState:"minecraft:sand",Time:1,Motion:[0.0d,0.0d,0.0d]}"#
            ));
            TrackedTarget::Falling(game.falling.last().unwrap().id)
        }
        "end_crystal" => {
            assert!(game.run_command("summon minecraft:end_crystal 4.5 64.0 4.5 {}"));
            TrackedTarget::Crystal(game.crystals.last().unwrap().id)
        }
        "ender_dragon" => {
            assert!(game.run_command(
                "summon minecraft:ender_dragon 7.5 64.0 4.5 {DragonPhase:10,Silent:1b}"
            ));
            game.tick_server(false).unwrap();
            TrackedTarget::Dragon
        }
        "armor_stand" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_marker" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Marker:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_small" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {Small:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "armor_stand_nogravity" => {
            assert!(game.run_command("summon minecraft:armor_stand 3.5 64.0 4.5 {NoGravity:1b}"));
            TrackedTarget::Stand(game.armor_stands.last().unwrap().id)
        }
        "leash_knot" => {
            assert!(game.run_command("summon minecraft:leash_knot 3.5 64.375 4.5 {}"));
            TrackedTarget::ExplicitKnot(game.others.last().unwrap().id)
        }
        "leash_knot_tied" => {
            assert!(game.run_command(
                "summon minecraft:pig 3.5 64.0 6.5 {NoAI:1b,Silent:1b,leash:[I;3,64,4]}"
            ));
            game.tick_server(false).unwrap();
            let id = *game
                .knot_ids
                .get(&[3, 64, 4])
                .expect("natural leash knot created on fence");
            TrackedTarget::NaturalKnot(id)
        }
        "cushion" => {
            assert!(game.run_command(
                r#"summon minecraft:cushion 3.5 64.0 4.5 {block_pos:[I;3,64,4],color:"white"}"#
            ));
            TrackedTarget::Cushion(game.cushions.last().unwrap().id)
        }
        "dropped_item" => {
            assert!(game.run_command(
                r#"summon minecraft:item 3.5 64.0 4.5 {Item:{id:"minecraft:diamond",count:1},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767}"#
            ));
            TrackedTarget::Item(game.dropped.last().unwrap().id)
        }
        other => panic!("unexpected fishing witness target {other}"),
    }
}

fn target_kinematics(game: &Game<'_>, target: TrackedTarget) -> Option<(Vec3, Vec3)> {
    match target {
        TrackedTarget::Falling(id) => game
            .falling
            .iter()
            .find(|f| f.id == id)
            .map(|f| (f.position, f.velocity)),
        TrackedTarget::Stand(id) => game
            .armor_stands
            .iter()
            .find(|a| a.id == id)
            .map(|a| (a.position, a.velocity)),
        TrackedTarget::Item(id) => game
            .dropped
            .iter()
            .find(|i| i.id == id)
            .map(|i| (i.position, i.velocity)),
        _ => None,
    }
}

fn check_hook_tick(
    game: &Game<'_>,
    label: &str,
    expected: &FishingTick,
    expected_hooked_id: Option<u32>,
    is_dragon: bool,
    posted: &[(&'static str, [f64; 3], Option<sunrise_client::mobs::Who>)],
    errors: &mut Vec<String>,
) {
    let exp_lands: Vec<&[String; 3]> = expected
        .game_events
        .iter()
        .filter(|g| g.id == "minecraft:projectile_land")
        .map(|g| &g.pos_bits)
        .collect();
    let got_lands: Vec<[String; 3]> = posted
        .iter()
        .filter(|(id, _, _)| *id == "minecraft:projectile_land")
        .map(|(_, at, _)| [bits(at[0]), bits(at[1]), bits(at[2])])
        .collect();
    if got_lands.len() != exp_lands.len() || got_lands.iter().zip(&exp_lands).any(|(a, e)| a != *e)
    {
        errors.push(format!(
            "{label} t={}: projectile_land expected {exp_lands:?} got {got_lands:?}",
            expected.tick
        ));
    }

    let Some(h) = game.hook.as_ref() else {
        errors.push(format!("{label} t={}: hook vanished", expected.tick));
        return;
    };
    let got_state = match h.state {
        State::Flying => "FLYING",
        State::Hooked(_) => "HOOKED_IN_ENTITY",
        State::Bobbing => "BOBBING",
    };
    if got_state != expected.hook_state {
        errors.push(format!(
            "{label} t={}: hook_state expected {} got {got_state}",
            expected.tick, expected.hook_state
        ));
    }
    if !is_dragon {
        let want_hooked = expected.hooked_type.is_some();
        if h.hooked_in.is_some() != want_hooked {
            errors.push(format!(
                "{label} t={}: hooked_in present expected {want_hooked} ({:?}) got {:?}",
                expected.tick, expected.hooked_type, h.hooked_in
            ));
        } else if want_hooked
            && let Some(tid) = expected_hooked_id
            && h.hooked_in != Some(tid)
        {
            errors.push(format!(
                "{label} t={}: hooked_in id expected {tid} got {:?}",
                expected.tick, h.hooked_in
            ));
        }
    }
    let got_pos = vec_bits(h.position);
    if got_pos != expected.hook_pos_bits {
        errors.push(format!(
            "{label} t={}: hook_pos expected {:?} got {got_pos:?} ({:?})",
            expected.tick, expected.hook_pos_bits, h.position
        ));
    }
    let got_vel = vec_bits(h.velocity);
    if got_vel != expected.hook_vel_bits {
        errors.push(format!(
            "{label} t={}: hook_vel expected {:?} got {got_vel:?} ({:?})",
            expected.tick, expected.hook_vel_bits, h.velocity
        ));
    }
    let got_yaw = fbits(h.yaw);
    if got_yaw != expected.hook_yaw_bits {
        errors.push(format!(
            "{label} t={}: hook_yaw expected {} got {got_yaw} ({})",
            expected.tick, expected.hook_yaw_bits, h.yaw
        ));
    }
}

fn spawn_test_hook(game: &mut Game<'_>, pos: Vec3, vel: Vec3, hook_seed: i64) {
    game.next_entity += 1;
    let id = game.next_entity;
    let mut hook = Hook::remote(id, pos, [0, 0, 0, id as i32]);
    hook.yaw = -90.0;
    hook.velocity = vel;
    hook.random = LegacyRandom::new(hook_seed);
    game.install_fishing_hook(hook);
}

fn run_case(
    game: &mut Game<'_>,
    case: &FishingCase,
    hook_seed: i64,
    level_seed: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let label = &case.target;
    reset_arena(game, &case.target);
    game.world.random.set_seed(level_seed);

    if case.target == "dropped_item" {
        align_next_entity_mod4(game, case.target_id_mod4);
    }
    let tracked = summon_target(game, &case.target);
    if let TrackedTarget::Item(id) = tracked {
        assert_eq!(id & 3, case.target_id_mod4, "ItemEntity tick phase");
    }
    spawn_test_hook(
        game,
        Vec3::new(1.5, case.spawn_y, 4.5),
        Vec3::new(1.0, 0.0, 0.0),
        hook_seed,
    );
    game.world.random.set_seed(level_seed);

    let is_dragon = matches!(tracked, TrackedTarget::Dragon);
    for expected in &case.ticks {
        game.audio.heard.clear();
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| {
            *e.borrow_mut() = Some(Vec::new());
        });
        game.tick_server(false).unwrap();
        let posted = crate::game::sculk::TEST_POSTED_EVENTS
            .with(|e| e.borrow_mut().take().unwrap_or_default());
        check_hook_tick(
            game,
            label,
            expected,
            tracked.id(),
            is_dragon,
            &posted,
            &mut errors,
        );
        if !errors.is_empty() {
            break;
        }
    }

    // Retrieve rod and verify exact durability damage and bit-exact pull velocity.
    let rod = ItemStack::new("minecraft:fishing_rod", 1);
    let damage = game.retrieve_hook(&rod) as i32;
    if damage != case.retrieve_damage {
        errors.push(format!(
            "{label}: retrieve damage expected {} got {damage}",
            case.retrieve_damage
        ));
    }
    if let Some((_, vel)) = target_kinematics(game, tracked) {
        let got_vel = vec_bits(vel);
        if got_vel != case.target_vel_after_retrieve_bits {
            errors.push(format!(
                "{label}: vel after retrieve expected {:?} got {got_vel:?} ({vel:?})",
                case.target_vel_after_retrieve_bits
            ));
        }
    }
    game.tick_server(false).unwrap();
    if let Some((pos, _)) = target_kinematics(game, tracked) {
        let got_pos = vec_bits(pos);
        if got_pos != case.target_pos_after_retrieve_tick_bits {
            errors.push(format!(
                "{label}: pos after retrieve tick expected {:?} got {got_pos:?} ({pos:?})",
                case.target_pos_after_retrieve_tick_bits
            ));
        }
    }
    errors
}

fn run_extra_case(
    game: &mut Game<'_>,
    case: &FishingExtraCase,
    hook_seed: i64,
    level_seed: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let label = format!("extra {}", case.case);
    reset_arena(game, "extra");
    game.world.random.set_seed(level_seed);

    let (expected_hooked_id, tracked_item): (Option<u32>, Option<u32>) = match case.case.as_str() {
        "owner_vehicle_passenger" => {
            assert!(game.run_command("summon minecraft:oak_boat 2.5 64.0 4.5 {}"));
            let boat_id = game.boats.last().unwrap().id;
            assert!(game.run_command("summon minecraft:pig 2.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
            let pig_id = game.mobs.last().unwrap().id;
            assert!(game.start_riding_target(Target::Player, Target::Boat(boat_id), true));
            assert!(game.start_riding_target(Target::Mob(pig_id), Target::Boat(boat_id), true));
            assert!(game.run_command("summon minecraft:armor_stand 5.5 64.0 4.5 {}"));
            let stand_id = game.armor_stands.last().unwrap().id;
            game.tick_server(false).unwrap();
            spawn_test_hook(
                game,
                Vec3::new(1.5, 64.8, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(stand_id), None)
        }
        "deflected_hook_catches_owner" => {
            assert!(game.run_command("summon minecraft:breeze 3.5 64.0 4.5 {NoAI:1b,Silent:1b}"));
            spawn_test_hook(
                game,
                Vec3::new(1.8, 65.4, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(game.own_entity_id() as u32), None)
        }
        "wall_endpoint_boundary" => {
            assert!(game.run_command("setblock 4 64 4 minecraft:stone"));
            assert!(game.run_command("summon minecraft:armor_stand 4.25 64.0 4.75 {NoGravity:1b}"));
            spawn_test_hook(
                game,
                Vec3::new(3.5, 64.5, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (None, None)
        }
        "tick8_margin_boundary" => {
            assert!(
                game.run_command(
                    "summon minecraft:armor_stand 6.9 64.0 5.050000005 {NoGravity:1b}"
                )
            );
            let stand_id = game.armor_stands.last().unwrap().id;
            spawn_test_hook(
                game,
                Vec3::new(1.5, 65.5, 4.5),
                Vec3::new(0.85, 0.0, 0.0),
                hook_seed,
            );
            (Some(stand_id), None)
        }
        "same_tick_retrieve_item" => {
            let mod4 = case.target_id_mod4.expect("target_id_mod4");
            align_next_entity_mod4(game, mod4);
            assert!(game.run_command(
                r#"summon minecraft:item 3.5 64.0 4.5 {Item:{id:"minecraft:diamond",count:1},Motion:[0.0d,0.0d,0.0d],PickupDelay:32767}"#
            ));
            let item_id = game.dropped.last().unwrap().id;
            assert_eq!(item_id & 3, mod4, "same_tick_retrieve_item tick phase");
            spawn_test_hook(
                game,
                Vec3::new(1.5, 64.15, 4.5),
                Vec3::new(1.0, 0.0, 0.0),
                hook_seed,
            );
            (Some(item_id), Some(item_id))
        }
        other => panic!("unexpected fishing extra case {other}"),
    };
    game.world.random.set_seed(level_seed);

    for expected in &case.ticks {
        game.audio.heard.clear();
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| {
            *e.borrow_mut() = Some(Vec::new());
        });
        game.tick_server(false).unwrap();
        let posted = crate::game::sculk::TEST_POSTED_EVENTS
            .with(|e| e.borrow_mut().take().unwrap_or_default());
        check_hook_tick(
            game,
            &label,
            expected,
            expected_hooked_id,
            false,
            &posted,
            &mut errors,
        );
        let exp_deflect = expected
            .sounds
            .iter()
            .filter(|s| s.as_str() == "minecraft:entity.breeze.deflect")
            .count();
        let got_deflect = game
            .audio
            .heard
            .get("minecraft:entity.breeze.deflect")
            .copied()
            .unwrap_or(0) as usize;
        if got_deflect != exp_deflect {
            errors.push(format!(
                "{label} t={}: breeze.deflect count expected {exp_deflect} got {got_deflect}",
                expected.tick
            ));
        }
        if !errors.is_empty() {
            break;
        }
        if case.retrieve_on_tick == Some(expected.tick) {
            let state_now = match game.hook.as_ref().map(|h| h.state) {
                Some(State::Flying) => "FLYING",
                Some(State::Hooked(_)) => "HOOKED_IN_ENTITY",
                Some(State::Bobbing) => "BOBBING",
                None => "NONE",
            };
            if case.state_at_retrieve.as_deref() != Some(state_now) {
                errors.push(format!(
                    "{label} t={}: state_at_retrieve expected {:?} got {state_now}",
                    expected.tick, case.state_at_retrieve
                ));
            }
            let rod = ItemStack::new("minecraft:fishing_rod", 1);
            let damage = game.retrieve_hook(&rod) as i32;
            if Some(damage) != case.retrieve_damage {
                errors.push(format!(
                    "{label}: retrieve_damage expected {:?} got {damage}",
                    case.retrieve_damage
                ));
            }
            if let Some(item_id) = tracked_item {
                let (_, vel) = target_kinematics(game, TrackedTarget::Item(item_id)).unwrap();
                if Some(vec_bits(vel)) != case.target_vel_after_retrieve_bits {
                    errors.push(format!(
                        "{label}: vel after retrieve expected {:?} got {:?}",
                        case.target_vel_after_retrieve_bits,
                        vec_bits(vel)
                    ));
                }
                game.tick_server(false).unwrap();
                let (pos, vel2) = target_kinematics(game, TrackedTarget::Item(item_id)).unwrap();
                if Some(vec_bits(pos)) != case.target_pos_after_retrieve_tick_bits
                    || Some(vec_bits(vel2)) != case.target_vel_after_retrieve_tick_bits
                {
                    errors.push(format!(
                        "{label}: after retrieve tick expected {:?}/{:?} got {:?}/{:?}",
                        case.target_pos_after_retrieve_tick_bits,
                        case.target_vel_after_retrieve_tick_bits,
                        vec_bits(pos),
                        vec_bits(vel2)
                    ));
                }
            }
        }
    }
    errors
}

#[test]
#[ignore = "run by tools/test_projectile_targets_parity.py"]
fn witness_fishing_bobber_targets() {
    run_witness_fishing_bobber_targets();
}

pub(in crate::game) fn run_witness_fishing_bobber_targets() {
    let witness = load_witness();
    let mut resources = None;
    let mut game = setup_game(&mut resources);
    let mut failures = Vec::new();
    for case in &witness.cases {
        let errs = run_case(&mut game, case, witness.hook_seed, witness.level_seed);
        if !errs.is_empty() {
            failures.push(errs.join("\n  "));
        }
    }
    for case in &witness.extra_cases {
        let errs = run_extra_case(&mut game, case, witness.hook_seed, witness.level_seed);
        if !errs.is_empty() {
            failures.push(errs.join("\n  "));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fishing witness cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
