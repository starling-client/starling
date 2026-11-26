use crate::game::command_harness::spans;
use crate::game::native_entity_refs::tests::{
    bind_local_player, prepare_entity_ticking_neighborhood, run_fresh_cases, with_game,
};
use serde_json::{Value, json};
use std::collections::HashSet;
use sunrise_client::motion::PlayerMotion;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;
use sunrise_core::random::RandomSource;

// `UUIDUtil.createOfflinePlayerUUID("Player")`, matching VanillaCommandExecOracle's single-player owner.
const ORACLE_PLAYER_UUID: u128 = 0xa01e3843_e521_3998_958a_f459800e4d11;

fn tag_contains_bolt(tag: &Tag) -> bool {
    if sunrise_client::entity_nbt::kind(tag).as_deref() == Some("minecraft:lightning_bolt") {
        return true;
    }
    if let Some(Tag::List(passengers)) = tag.get("Passengers") {
        return passengers.iter().any(tag_contains_bolt);
    }
    false
}

/// Replays the vanilla 26.3 witness for `/summon minecraft:lightning_bolt` with trailing NBT (#2690),
/// recorded into `.local` by `tools/test_summon_bolt_nbt_parity.py`.
#[test]
#[ignore = "run by tools/test_summon_bolt_nbt_parity.py"]
fn a_bolt_summoned_with_nbt_strikes_and_goes() {
    run_a_bolt_summoned_with_nbt_strikes_and_goes();
}

pub(in crate::game) fn run_a_bolt_summoned_with_nbt_strikes_and_goes() {
    let witness_path = std::env::var_os("SUNRISE_SUMMON_BOLT_NBT_WITNESS").unwrap_or_else(|| {
        panic!("SUNRISE_SUMMON_BOLT_NBT_WITNESS is unset; run tools/test_summon_bolt_nbt_parity.py")
    });
    let witness_path = std::path::PathBuf::from(witness_path);
    let witness_path = if witness_path.is_dir() {
        witness_path.join("summon_bolt_nbt.json")
    } else {
        witness_path
    };
    let raw = std::fs::read_to_string(&witness_path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (run tools/test_summon_bolt_nbt_parity.py): {e}",
            witness_path.display()
        )
    });

    // Opt in to deterministic thread seeds only in a fresh child process.
    if run_fresh_cases(
        "game::commands::summon_bolt_tests::a_bolt_summoned_with_nbt_strikes_and_goes",
        "SUNRISE_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }

    let witness: Value = serde_json::from_str(&raw).expect("valid 26.3 witness JSON");
    assert_eq!(witness["minecraft"], "26.3");
    let steps = witness["steps"].as_array().unwrap();
    assert!(!steps.is_empty());

    let _language =
        sunrise_client::lang::isolate_publication(sunrise_client::lang::Language::of("en_us", []));
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(15, 0));
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(16, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 230.0, 8.5)).unwrap());
        bind_local_player(game, 1, ORACLE_PLAYER_UUID);
        game.creative = true;
        game.ui.difficulty = 2;
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0"));
        game.update_chunk_tickets();
        for setup_cmd in [
            "gamerule spawn_mobs false",
            "gamerule mob_drops false",
            "gamerule entity_drops false",
            "gamerule advance_time false",
            "gamerule advance_weather false",
        ] {
            let _ = game.run_command(setup_cmd);
        }

        let mut saw_live_bolt_save_check = false;
        let mut saw_mounted_bolt_save_check = false;
        for (index, step) in steps.iter().enumerate() {
            match step["op"].as_str().unwrap() {
                "sync_randoms" => {
                    let uniquifier = step["uniquifier"].as_i64().unwrap();
                    let entity_id = step["entity_id"].as_i64().unwrap();
                    let level_seed = step["level_seed"].as_i64().unwrap();
                    sunrise_client::random_source::set_thread_uniquifier(uniquifier).unwrap();
                    game.next_entity = u32::try_from(entity_id).unwrap();
                    game.world.random.set_seed(level_seed);
                }
                "wait_chunks" => {
                    for pair in step["chunks"].as_array().unwrap() {
                        let cx = pair[0].as_i64().unwrap() as i32;
                        let cz = pair[1].as_i64().unwrap() as i32;
                        prepare_entity_ticking_neighborhood(game, ChunkPos::new(cx, cz));
                    }
                    game.update_chunk_tickets();
                    game.spawn_loaded_entities();
                }
                "cmd" => {
                    let cmd = step["cmd"].as_str().unwrap();
                    let line = cmd.strip_prefix('/').unwrap_or(cmd);
                    game.chat_capture = Some(Vec::new());
                    game.cmds.results = Some(Vec::new());
                    game.as_console(|g| g.run_command(line));
                    let messages = game.chat_capture.take().unwrap_or_default();
                    let results = game.cmds.results.take().unwrap_or_default();
                    if step["record"].as_bool() != Some(false) {
                        let actual_messages = Value::Array(messages.iter().map(spans).collect());
                        let actual_results =
                            Value::Array(results.iter().map(|(s, v)| json!([s, v])).collect());
                        assert_eq!(
                            actual_messages, step["messages"],
                            "step {index} ({cmd}): messages differ from 26.3 witness"
                        );
                        assert_eq!(
                            actual_results, step["results"],
                            "step {index} ({cmd}): results differ from 26.3 witness"
                        );
                    }
                    assert!(
                        game.others
                            .iter()
                            .all(|o| o.kind != "minecraft:lightning_bolt"),
                        "step {index} ({cmd}): lightning_bolt must never be stored in others"
                    );
                    if !game.bolts.is_empty() {
                        let keys: HashSet<(i32, i32)> = game
                            .bolts
                            .iter()
                            .map(|b| ((b.at[0].floor() as i32) >> 4, (b.at[2].floor() as i32) >> 4))
                            .collect();
                        let saved = game.entity_chunks(&keys, false);
                        for (chunk_key, tags) in &saved {
                            for tag in tags {
                                assert!(
                                    !tag_contains_bolt(tag),
                                    "step {index} ({cmd}): chunk {chunk_key:?} saved a lightning_bolt: {tag:?}"
                                );
                            }
                        }
                        saw_live_bolt_save_check = true;
                        if game.bolts.iter().any(|b| b.mount.is_some()) {
                            saw_mounted_bolt_save_check = true;
                        }
                    }
                }
                "tick" => {
                    let n = step["n"].as_u64().unwrap();
                    for _ in 0..n {
                        game.tick_server(false).unwrap();
                    }
                }
                other => panic!("step {index}: unexpected witness op {other}"),
            }
        }
        assert!(
            saw_live_bolt_save_check,
            "witness must exercise live bolt save exclusion"
        );
        assert!(
            saw_mounted_bolt_save_check,
            "witness must exercise mounted bolt save exclusion"
        );
        assert!(
            game.bolts.is_empty(),
            "bolts must be gone after 26.3 witness"
        );
    });
}

/// Verifies `Silent:1b` sound suppression and `minecraft:lightning_strike` game-event timing on first server tick.
#[test]
#[ignore = "daily"]
fn silent_bolt_suppresses_sound_and_posts_strike_event_on_first_tick() {
    let _language =
        sunrise_client::lang::isolate_publication(sunrise_client::lang::Language::of("en_us", []));
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 230.0, 8.5)).unwrap());
        bind_local_player(game, 1, ORACLE_PLAYER_UUID);
        game.creative = true;
        game.ui.difficulty = 2;
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0"));
        game.update_chunk_tickets();

        let take_strike_events = || {
            crate::game::sculk::TEST_POSTED_EVENTS.with(|events| {
                let mut guard = events.borrow_mut();
                let list = guard.get_or_insert_with(Vec::new);
                let count = list
                    .iter()
                    .filter(|(ev, _, _)| *ev == "minecraft:lightning_strike")
                    .count();
                list.clear();
                count
            })
        };

        // Silent bolt: no event on spawn; strikes rod + posts event on tick 1 without playing thunder/impact audio.
        assert!(game.run_command("setblock 4 209 4 minecraft:lightning_rod"));
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
        game.audio.capture = Some(Vec::new());
        assert!(game.run_command(
            "summon minecraft:lightning_bolt 4.5 210 4.5 {Silent:1b,Tags:[\"silent_bolt\"]}"
        ));
        assert_eq!(
            take_strike_events(),
            0,
            "no lightning_strike game event at spawn"
        );
        assert!(
            game.audio.capture.as_ref().unwrap().is_empty(),
            "no sound played at spawn"
        );
        game.tick_server(false).unwrap();
        assert_eq!(
            take_strike_events(),
            1,
            "lightning_strike event posted on first server tick"
        );
        assert!(
            game.audio.capture.as_ref().unwrap().iter().all(|s| {
                s.event != "minecraft:entity.lightning_bolt.thunder"
                    && s.event != "minecraft:entity.lightning_bolt.impact"
            }),
            "Silent:1b bolt must suppress thunder and impact sounds"
        );
        assert!(game.run_command("execute if block 4 209 4 minecraft:lightning_rod[powered=true]"));

        // Audible bolt: no event on spawn; posts event and plays thunder + impact on tick 1.
        game.audio.capture = Some(Vec::new());
        assert!(game.run_command("summon minecraft:lightning_bolt 8.5 210 4.5 {}"));
        assert_eq!(
            take_strike_events(),
            0,
            "no lightning_strike game event at spawn"
        );
        game.tick_server(false).unwrap();
        assert_eq!(
            take_strike_events(),
            1,
            "lightning_strike event posted on first server tick"
        );
        let sounds = game.audio.capture.take().unwrap();
        assert!(
            sounds
                .iter()
                .any(|s| s.event == "minecraft:entity.lightning_bolt.thunder"),
            "non-silent bolt plays thunder sound on first tick"
        );
        assert!(
            sounds
                .iter()
                .any(|s| s.event == "minecraft:entity.lightning_bolt.impact"),
            "non-silent bolt plays impact sound on first tick"
        );
        crate::game::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = None);
    });
}
