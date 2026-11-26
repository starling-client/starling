//! `/summon item` and `/summon experience_orb`: `SummonCommand.createEntity`
//! builds both from `{id:..}` through their `(EntityType, Level)` factory, so
//! the orb is never unstuck, and an empty item is discarded while loading
//! (`ItemEntity.readAdditionalSaveData`) yet the command still succeeds.
use super::super::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use std::sync::Arc;
use sunrise_client::chunk::Chunk;
use sunrise_core::geometry::Vec3;

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    test(&mut game);
}

/// `ItemEntity(EntityType, Level)` leaves the stack `ItemStack.EMPTY`, and
/// `readAdditionalSaveData` discards such an entity, so `addEntity` adds
/// nothing; `tryAddFreshEntityWithPassengers` still answers true, so both
/// summon forms report success. The entity id is taken all the same, and the
/// name is `ItemStack.EMPTY.getItemName()`: an empty component.
#[test]
#[ignore = "daily"]
fn summon_item_succeeds_with_and_without_nbt_without_leaving_an_entity() {
    with_game(|game| {
        for command in [
            "summon minecraft:item 4.5 200 4.5",
            "summon minecraft:item 4.5 200 4.5 {}",
        ] {
            let before = game.next_entity;
            assert!(
                game.run_command(command),
                "{command}: {:?}",
                game.ui.chat.recent_lines(3)
            );
            assert_eq!(game.next_entity, before + 1, "{command}");
            assert!(game.dropped.is_empty(), "{command}");
            assert_eq!(
                game.ui.chat.recent_components(1),
                vec![serde_json::json!({
                    "translate": "commands.summon.success",
                    "with": [{ "text": "" }],
                })],
                "{command}"
            );
        }
    });
}

/// `ExperienceOrb(EntityType, Level)` is the factory `EntityType` uses: the
/// position is `SummonCommand`'s `snapTo` and nothing unsticks it, so an orb
/// summoned against the top of a block stays where it was asked for.
#[test]
#[ignore = "daily"]
fn a_summoned_orb_keeps_the_position_it_was_asked_for() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 100 4 stone"));
        // Its box (0.5 tall) reaches into the stone's top 0.2, which is what
        // makes the constructor's unstuck search move it in the first place.
        assert!(
            game.run_command("summon minecraft:experience_orb 4.5 100.8 4.5"),
            "{:?}",
            game.ui.chat.recent_lines(3)
        );
        let orb = game.orbs.last().expect("an orb");
        assert_eq!(orb.position, Vec3::new(4.5, 100.8, 4.5));
        assert_eq!(orb.velocity, Vec3::ZERO);
        assert_eq!(orb.points(), 0);
        assert_eq!(orb.count, 1);
    });
}

/// Vanilla 26.3 witness: `/summon` replies with `Entity.getDisplayName` (team format, `show_entity` hover, UUID insertion, `click_event` stripped), validates `CustomName` as a text component, keeps selector `name=` plain, and `/execute summon` checks spawn bounds (#2694).
#[test]
#[ignore = "run by tools/test_summon_custom_name_parity.py"]
fn summon_replies_with_the_custom_name_of_every_kind() {
    run_summon_replies_with_the_custom_name_of_every_kind();
}

pub(in crate::game) fn run_summon_replies_with_the_custom_name_of_every_kind() {
    let path = std::env::var_os("SUNRISE_SUMMON_CUSTOM_NAME_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_SUMMON_CUSTOM_NAME_WITNESS is unset; run tools/test_summon_custom_name_parity.py"
        )
    });
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_summon_custom_name_parity.py): {e}",
            std::path::Path::new(&path).display()
        )
    });
    let witness: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(witness["minecraft"], "26.3");
    assert_eq!(
        witness["oracle"],
        "dev.sunriselab.parity.VanillaCommandExecOracle"
    );
    assert_eq!(
        witness["groups"],
        serde_json::json!(["summon custom name replies"])
    );
    let records = witness["records"].as_array().unwrap();

    let mut expected_cases: Vec<(String, bool)> = vec![
        (
            "/execute positioned 4.5 20000000 4.5 summon evoker_fangs run tag @s add out_of_bounds"
                .into(),
            true,
        ),
        ("/execute if entity @e[type=evoker_fangs]".into(), false),
        (
            "/execute if entity @e[type=evoker_fangs,tag=out_of_bounds]".into(),
            false,
        ),
        (
            "/execute positioned 4.5 200 4.5 summon evoker_fangs run tag @s add in_bounds".into(),
            false,
        ),
        (
            "/execute if entity @e[type=evoker_fangs,tag=in_bounds]".into(),
            false,
        ),
        ("/team add t2694".into(), false),
        ("/team modify t2694 color red".into(), false),
        ("/team modify t2694 prefix \"[R] \"".into(), false),
        ("/team modify t2694 suffix \"!\"".into(), false),
    ];
    let kinds = [
        (1u32, "evoker_fangs", "2.5", ""),
        (2, "experience_orb", "5.5", ""),
        (3, "area_effect_cloud", "8.5", ""),
        (4, "ender_dragon", "11.5", ""),
        (5, "pig", "14.5", "NoAI:1b,NoGravity:1b,"),
    ];
    let variants = [
        (1u32, ""),
        (2, "CustomName:\"\""),
        (3, "CustomName:\"fz\""),
        (4, "CustomName:{text:\"fz\",color:\"gold\",bold:true}"),
        (
            5,
            "CustomName:{text:\"fz\",color:\"gold\",click_event:{action:\"run_command\",command:\"/say hi\"},extra:[{text:\"!\",color:\"aqua\",click_event:{action:\"suggest_command\",command:\"/help\"}}]}",
        ),
        (
            6,
            "CustomName:{text:\"fz\",hover_event:{action:\"show_text\",value:\"old\"},insertion:\"old\"}",
        ),
        (7, "CustomName:42"),
        (8, "CustomName:{}"),
        (9, "CustomName:{text:42}"),
        (10, "CustomName:[]"),
    ];
    for (kind_id, kind, x, extra) in kinds {
        for (case_id, name_nbt) in variants {
            let z = 4.5 + f64::from(case_id % 5) * 2.0;
            let suffix = if name_nbt.is_empty() {
                String::new()
            } else {
                format!(",{name_nbt}")
            };
            expected_cases.push((
                format!(
                    "/summon {kind} {x} 200 {z} {{{extra}UUID:[I;2694,0,{kind_id},{case_id}]{suffix}}}"
                ),
                true,
            ));
            if kind == "ender_dragon" {
                expected_cases.push((
                    format!("/kill 00000a86-0000-0000-0000-{kind_id:04x}{case_id:08x}"),
                    true,
                ));
            }
        }
        expected_cases.push((
            format!(
                "/summon {kind} {x} 200 14.5 {{{extra}UUID:[I;2694,1,{kind_id},1],CustomName:\"fz\",Team:\"t2694\",Tags:[\"t2694_team\"]}}"
            ),
            true,
        ));
        expected_cases.push((
            format!("/execute if entity @e[type={kind},tag=t2694_team,name=fz]"),
            false,
        ));
        expected_cases.push((
            format!("/execute if entity @e[type={kind},tag=t2694_team,name=\"[R] fz!\"]"),
            false,
        ));
        if kind == "ender_dragon" {
            expected_cases.push((
                format!("/kill 00000a86-0000-0001-0000-{kind_id:04x}00000001"),
                true,
            ));
        }
    }
    assert_eq!(
        records.len(),
        expected_cases.len(),
        "witness record count differs from expected cases"
    );

    with_game(|game| {
        for (index, (record, (expected_cmd, expect_components))) in
            records.iter().zip(expected_cases.iter()).enumerate()
        {
            let cmd = record["cmd"].as_str().unwrap();
            assert_eq!(cmd, expected_cmd.as_str(), "case #{index} identity/order");
            let line = cmd.strip_prefix('/').unwrap_or(cmd);
            game.chat_capture = Some(Vec::new());
            game.cmds.results = Some(Vec::new());
            game.run_command(line);
            let captured = game.chat_capture.take().unwrap_or_default();
            let expected_messages = record["messages"].as_array().unwrap();
            assert_eq!(
                captured.len(),
                expected_messages.len(),
                "case #{index} message count: {cmd}"
            );
            let messages: Vec<serde_json::Value> = captured
                .iter()
                .map(super::super::command_harness::spans)
                .collect();
            assert_eq!(
                serde_json::Value::Array(messages),
                record["messages"],
                "case #{index} spans: {cmd}"
            );
            if *expect_components {
                let expected_components = record["components"]
                    .as_array()
                    .expect("structured components recorded");
                assert_eq!(
                    captured.len(),
                    expected_components.len(),
                    "case #{index} component count: {cmd}"
                );
                let components: Vec<serde_json::Value> = captured
                    .iter()
                    .map(super::super::command_harness::canonical_component)
                    .collect();
                assert_eq!(
                    serde_json::Value::Array(components),
                    record["components"],
                    "case #{index} components: {cmd}"
                );
            } else {
                assert!(
                    record.get("components").is_none(),
                    "case #{index} unexpected components field: {cmd}"
                );
            }
            let results: Vec<serde_json::Value> = game
                .cmds
                .results
                .take()
                .unwrap_or_default()
                .into_iter()
                .map(|(success, value)| serde_json::json!([success, value]))
                .collect();
            assert_eq!(
                serde_json::Value::Array(results),
                record["results"],
                "case #{index} results: {cmd}"
            );
        }
    });
}
