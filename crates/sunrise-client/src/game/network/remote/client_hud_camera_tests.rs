// 26.3 witness: real Hud, recorded into `.local` by `tools/test_camera_parity.py`.
use crate::game::remote::RemoteDimensionEntry;
use crate::game::{
    Game, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
};
use std::collections::BTreeMap;
use sunrise_client::{items::ItemStack, level_loading::Reason, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    entity_data::{Value, serializer},
    item::Stack,
    packets::play::{self, Clientbound as C, SpawnInfo},
};

const SIZE: [u32; 2] = [427, 240];
const LOCAL: i32 = 7;
const WATCHED: i32 = 50;
const MOB: i32 = 31;

type Sprite = (String, i32, i32, i32, i32);
type Groups = BTreeMap<&'static str, Vec<Sprite>>;

const GROUPS: [(&str, &[&str]); 7] = [
    (
        "hotbar",
        &[
            "hud/hotbar",
            "hud/hotbar_selection",
            "hud/hotbar_offhand_left",
            "hud/hotbar_offhand_right",
            "hud/hotbar_attack_indicator_background",
        ],
    ),
    (
        "armor",
        &["hud/armor_full", "hud/armor_half", "hud/armor_empty"],
    ),
    (
        "hearts",
        &[
            "hud/heart/container",
            "hud/heart/full",
            "hud/heart/half",
            "hud/heart/absorbing_full",
            "hud/heart/absorbing_half",
        ],
    ),
    (
        "food",
        &["hud/food_empty", "hud/food_half", "hud/food_full"],
    ),
    ("air", &["hud/air", "hud/air_empty"]),
    (
        "vehicle",
        &[
            "hud/heart/vehicle_container",
            "hud/heart/vehicle_full",
            "hud/heart/vehicle_half",
        ],
    ),
    ("xp", &["hud/experience_bar_background"]),
];

fn group_of(name: &str) -> Option<&'static str> {
    GROUPS
        .iter()
        .find(|(_, names)| names.contains(&name))
        .map(|(group, _)| *group)
}

/// The recorded vanilla draws of one case, whole sprites only, grouped by layer in draw order.
fn vanilla_groups(case: &serde_json::Value) -> Groups {
    let mut groups = Groups::new();
    for d in case["draws"].as_array().unwrap() {
        if d["type"] == "sprite"
            && d.get("region").is_none()
            && let Some(name) = d["sprite"].as_str()
            && let Some(group) = group_of(name)
        {
            let n = |k: &str| d[k].as_i64().unwrap() as i32;
            groups.entry(group).or_default().push((
                name.to_string(),
                n("x"),
                n("y"),
                n("w"),
                n("h"),
            ));
        }
    }
    groups
}

/// The same sprites, decoded from the quads a frame of the game drew.
fn drawn_groups(game: &mut Game<'_>) -> Groups {
    let batch = game.hud_layers(SIZE);
    let rects: Vec<(&str, [f32; 4])> = GROUPS
        .iter()
        .flat_map(|(_, names)| names.iter())
        .map(|name| {
            let r = *game
                .gui
                .atlas
                .gui_rect(name)
                .unwrap_or_else(|| panic!("the atlas holds {name}"));
            (*name, [r.u0, r.v0, r.u1, r.v1])
        })
        .collect();
    let mut groups = Groups::new();
    for q in batch.vertices.as_chunks::<6>().0 {
        let Some((name, _)) = rects
            .iter()
            .find(|(_, uv)| q[0].uv == [uv[0], uv[1]] && q[2].uv == [uv[2], uv[3]])
        else {
            continue;
        };
        groups.entry(group_of(name).unwrap()).or_default().push((
            name.to_string(),
            q[0].pos[0] as i32,
            q[0].pos[1] as i32,
            (q[2].pos[0] - q[0].pos[0]) as i32,
            (q[2].pos[1] - q[0].pos[1]) as i32,
        ));
    }
    groups
}

fn id(game: &Game<'_>, registry: &str, name: &str) -> i32 {
    game.remote
        .as_ref()
        .unwrap()
        .builtin
        .get(registry)
        .unwrap()
        .id(name)
        .unwrap()
}

fn spawn(game: &mut Game<'_>, entity: i32, kind: &str) {
    let kind = id(game, "minecraft:entity_type", kind);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id: entity,
        uuid: entity as u128 + 1000,
        kind,
        position: [10.0, 64.0, 12.0],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

fn data(game: &mut Game<'_>, entity: i32, values: Vec<(u8, i32, Value)>) {
    game.handle_packet(C::SetEntityData(play::SetEntityData { id: entity, values }))
        .unwrap();
}

fn attributes(game: &mut Game<'_>, entity: i32, values: &[(&str, f64)]) {
    let values = values
        .iter()
        .map(|(name, base)| play::AttributeSnapshot {
            attribute: game.motion.attributes.id(name).unwrap().registry_id(),
            base: *base,
            modifiers: vec![],
        })
        .collect();
    game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
        id: entity,
        values,
    }))
    .unwrap();
}

fn num(v: &serde_json::Value, key: &str) -> f64 {
    v[key].as_f64().unwrap()
}

/// A mount at `entity` carrying `rider`, with the health the case gives it.
fn mount(game: &mut Game<'_>, entity: i32, rider: i32, max: f64, health: f64) {
    spawn(game, entity, "minecraft:horse");
    attributes(game, entity, &[("minecraft:max_health", max)]);
    data(
        game,
        entity,
        vec![(9, serializer::FLOAT, Value::Float(health as f32))],
    );
    game.handle_packet(C::SetPassengers(play::SetPassengers {
        vehicle: entity,
        passengers: vec![rider],
    }))
    .unwrap();
}

/// The server-side scene of one recorded case, delivered as the packets a client gets.
fn set_up(game: &mut Game<'_>, case: &serde_json::Value) {
    let game_type = match case["gameMode"].as_str().unwrap() {
        "survival" => 0,
        "creative" => 1,
        "adventure" => 2,
        "spectator" => 3,
        other => panic!("game mode {other}"),
    };
    game.native_lifecycle = NativeLifecycle::default();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "HudCameraTest").unwrap();
    let spawn_info = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn_info, RemoteDimensionEntry::Login, Reason::Other);
    let remote = game.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = LOCAL;
    remote.level_load.finish();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());

    // The viewer's own state: what the local hotbar and bars would show.
    game.selected = case["selected"].as_u64().unwrap() as usize;
    game.settings.main_hand_left = !case["rightHanded"].as_bool().unwrap();
    for (i, pop) in case["slots"].as_array().unwrap().iter().enumerate() {
        if let Some(pop) = pop.as_u64() {
            game.inventory[i] = Some(ItemStack::new("minecraft:stone", 1));
            game.hotbar_pop[i] = pop as u8;
        }
    }
    game.handle_packet(C::SetHealth(play::SetHealth {
        health: num(case, "health") as f32,
        food: num(case, "food") as i32,
        saturation: num(case, "saturation") as f32,
    }))
    .unwrap();
    attributes(
        game,
        LOCAL,
        &[
            ("minecraft:max_health", num(case, "maxHealth")),
            ("minecraft:armor", num(case, "armor")),
        ],
    );
    game.player.as_mut().unwrap().air = num(case, "air") as i32;
    if case.get("vehicleMaxHealth").is_some() {
        mount(
            game,
            40,
            LOCAL,
            num(case, "vehicleMaxHealth"),
            num(case, "vehicleHealth"),
        );
    }

    match case["cameraKind"].as_str().unwrap() {
        "self" => {}
        "mob" => {
            spawn(game, MOB, "minecraft:creeper");
            game.remote_set_camera(MOB);
        }
        "otherPlayer" => {
            let w = &case["watched"];
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 1, // ADD_PLAYER
                entries: vec![play::PlayerInfo {
                    uuid: WATCHED as u128 + 1000,
                    name: Some("Watched".into()),
                    ..Default::default()
                }],
            }))
            .unwrap();
            spawn(game, WATCHED, "minecraft:player");
            data(
                game,
                WATCHED,
                vec![
                    (9, serializer::FLOAT, Value::Float(num(w, "health") as f32)),
                    (
                        17,
                        serializer::FLOAT,
                        Value::Float(num(w, "absorption") as f32),
                    ),
                    (1, serializer::INT, Value::Int(num(w, "air") as i32)),
                    (
                        15,
                        serializer::HUMANOID_ARM,
                        Value::Id(i32::from(w["rightHanded"].as_bool().unwrap())),
                    ),
                ],
            );
            attributes(
                game,
                WATCHED,
                &[
                    ("minecraft:max_health", num(w, "maxHealth")),
                    ("minecraft:armor", num(w, "armor")),
                ],
            );
            let sword = Stack::new(id(game, "minecraft:item", "minecraft:diamond_sword"), 1);
            let shield = Stack::new(id(game, "minecraft:item", "minecraft:shield"), 1);
            let mut slots = vec![];
            if w["mainHand"].as_bool().unwrap() {
                slots.push((0u8, Some(sword)));
            }
            if w["offhand"].as_bool().unwrap() {
                slots.push((1u8, Some(shield)));
            }
            game.handle_packet(C::SetEquipment(play::SetEquipment { id: WATCHED, slots }))
                .unwrap();
            if w.get("vehicleMaxHealth").is_some() {
                mount(
                    game,
                    41,
                    WATCHED,
                    num(w, "vehicleMaxHealth"),
                    num(w, "vehicleHealth"),
                );
            }
            game.remote_set_camera(WATCHED);
        }
        other => panic!("camera kind {other}"),
    }
    if case["attackIndicator"] == "HOTBAR" {
        game.settings.attack_indicator = 2;
    }
}

/// `Hud.getPlayerVehicleWithHealth` and `Hud.getAirBubbleYLine`: the watched rider can
/// ride the dragon or the local player, but a boat leaves the food and air rows unchanged.
#[test]
#[ignore = "daily"]
fn a_watched_rider_gets_hearts_for_the_dragon_and_local_player_but_not_a_boat() {
    use crate::game::{Screen, in_own_process, remote_camera::CameraPlayer};

    in_own_process(
        "game::remote::tests::client_hud_camera_tests::a_watched_rider_gets_hearts_for_the_dragon_and_local_player_but_not_a_boat",
        || {
            with_client_game(|game| {
                let (_reader, _writer) = super::connect_menu_test(game);
                super::spawn_menu_test(game);
                game.remote.as_mut().unwrap().level_load.finish();
                game.screen = Screen::Playing;
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                // LivingEntity.getMaxHealth converts to f32 before Hud adds the half point.
                attributes(game, LOCAL, &[("minecraft:max_health", 31.499_999_9)]);
                game.handle_packet(C::SetHealth(play::SetHealth {
                    health: 17.25,
                    food: 20,
                    saturation: 5.0,
                }))
                .unwrap();

                const DRAGON: i32 = 60;
                const BOAT: i32 = 80;
                game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1,
                    entries: vec![play::PlayerInfo {
                        uuid: WATCHED as u128 + 1000,
                        name: Some("Watched".into()),
                        ..play::PlayerInfo::default()
                    }],
                }))
                .unwrap();
                spawn(game, WATCHED, "minecraft:player");
                data(game, WATCHED, vec![(1, serializer::INT, Value::Int(150))]);
                spawn(game, DRAGON, "minecraft:ender_dragon");
                // Seed the live dragon, not its generic mob metadata.
                game.dragon.as_mut().unwrap().health = 37.25;
                spawn(game, BOAT, "minecraft:oak_boat");
                game.handle_packet(C::SetCamera(play::SetCamera { camera_id: WATCHED }))
                    .unwrap();
                let camera = game.camera_player();
                let CameraPlayer::Other(watched) = camera else {
                    panic!("expected the watched player to be the camera");
                };

                for (vehicle, expected, air_y, dragon_max_health) in [
                    (DRAGON, Some((30, 37.25_f32.to_bits())), 171, None),
                    (DRAGON, Some((9, 37.25_f32.to_bits())), 191, Some(18.0)),
                    (LOCAL, Some((16, 17.25_f32.to_bits())), 181, None),
                    (BOAT, None, 191, None),
                ] {
                    if let Some(max_health) = dragon_max_health {
                        attributes(game, DRAGON, &[("minecraft:max_health", max_health)]);
                    }
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers: vec![WATCHED],
                    }))
                    .unwrap();
                    assert_eq!(game.mobs[watched].extra.mount, Some(vehicle as u32));
                    assert_eq!(game.camera_vehicle(camera).is_some(), expected.is_some());
                    assert_eq!(
                        game.camera_vehicle_health(camera)
                            .map(|(hearts, health)| (hearts, health.to_bits())),
                        expected,
                        "vehicle {vehicle}"
                    );
                    let drawn = drawn_groups(game);
                    let containers = drawn
                        .get("vehicle")
                        .into_iter()
                        .flatten()
                        .filter(|sprite| sprite.0 == "hud/heart/vehicle_container")
                        .count();
                    assert_eq!(
                        containers,
                        expected.map_or(0, |(hearts, _)| hearts as usize)
                    );
                    assert_eq!(drawn.contains_key("food"), expected.is_none());
                    let air = &drawn["air"];
                    assert!(!air.is_empty());
                    assert!(air.iter().all(|sprite| sprite.2 == air_y));
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers: vec![],
                    }))
                    .unwrap();
                }
            });
        },
    );
}

#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn hud_bars_follow_the_camera_player_like_26_3() {
    run_hud_bars_follow_the_camera_player_like_26_3();
}

pub(in crate::game) fn run_hud_bars_follow_the_camera_player_like_26_3() {
    let path = std::env::var("SUNRISE_HUD_CAMERA_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_HUD_CAMERA_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    let mut cases = 0;
    for line in witness.lines() {
        cases += 1;
        let recorded: serde_json::Value = serde_json::from_str(line).unwrap();
        let case = &recorded["case"];
        let expected = vanilla_groups(&recorded);
        let label = format!(
            "case {cases} {} {}",
            case["gameMode"].as_str().unwrap(),
            case["cameraKind"].as_str().unwrap()
        );
        let has_xp = |g: &Groups| !g.get("xp").unwrap_or(&vec![]).is_empty();
        let survival = matches!(case["gameMode"].as_str(), Some("survival" | "adventure"));
        assert_eq!(
            has_xp(&expected),
            survival,
            "{label}: the recording draws the experience bar"
        );
        with_client_game(|game| {
            set_up(game, case);
            let drawn = drawn_groups(game);
            assert_eq!(
                has_xp(&drawn),
                has_xp(&expected),
                "{label}: the HUD drew the experience bar"
            );
            assert_eq!(drawn, expected, "{label}: sprites drawn per layer");
        });
    }
    assert_eq!(cases, 11, "every recorded case ran");
}
