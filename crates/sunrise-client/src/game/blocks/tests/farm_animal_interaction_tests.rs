//! Milking a farm animal and shearing a sheep or mooshroom by hand: the milk
//! pitch is fixed at 1.0 (`AbstractCow.mobInteract`, `Goat.mobInteract`,
//! `MushroomCow.mobInteract`) and shearing posts `GameEvent.SHEAR` with the
//! player (`Sheep.mobInteract`, `MushroomCow.mobInteract`).
use crate::game::*;
use std::sync::Arc;
use sunrise_client::chunk::Chunk;
use sunrise_client::mobs::{Mob, Who};
use sunrise_client::motion::PlayerMotion;
use sunrise_core::{geometry::Vec3, position::ChunkPos};

fn game<'a>(resources: &'a mut Option<Arc<Resources>>) -> Game<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    game_at(resources, root)
}

fn game_at<'a>(resources: &'a mut Option<Arc<Resources>>, root: std::path::PathBuf) -> Game<'a> {
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
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.screen = Screen::Playing;
    game.selected = 0;
    game.set_mode_id(0);
    game
}

fn spawn(game: &mut Game<'_>, kind: &str, id: u32, at: [f64; 3]) {
    game.mobs.clear();
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    game.mobs
        .push(Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0));
}

fn target(id: u32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id: id as i32,
        location: [0.0, 0.0, 0.0],
    })
}

fn take_shear_events() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    super::TEST_POSTED_EVENTS.with(|events| {
        std::mem::take(events.borrow_mut().as_mut().unwrap())
            .into_iter()
            .filter(|event| event.0 == "minecraft:shear")
            .collect()
    })
}

#[test]
#[ignore = "daily"]
fn milk_sounds_carry_pitch_one_and_draw_no_level_random() {
    let mut resources = None;
    let mut game = game(&mut resources);
    for (kind, item, sound) in [
        (
            "minecraft:cow",
            "minecraft:bucket",
            "minecraft:entity.cow.milk",
        ),
        (
            "minecraft:goat",
            "minecraft:bucket",
            "minecraft:entity.goat.milk",
        ),
        (
            "minecraft:mooshroom",
            "minecraft:bowl",
            "minecraft:entity.mooshroom.milk",
        ),
    ] {
        spawn(&mut game, kind, 7, [2.5, 64.0, 2.5]);
        game.inventory[0] = Some(ItemStack::new(item, 1));
        game.audio.capture = Some(Vec::new());
        let before = game.world.random.state();
        assert!(game.interact_mob(target(7)).unwrap(), "{kind}");
        let sounds = game.audio.capture.take().unwrap();
        let heard: Vec<_> = sounds.iter().filter(|s| s.event == sound).collect();
        assert_eq!(heard.len(), 1, "{kind}: {sounds:?}");
        assert_eq!(heard[0].pitch, 1.0, "{kind} milk pitch");
        assert_eq!(
            game.world.random.state(),
            before,
            "{kind} milk drew the level random"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_brown_mooshroom_milks_the_effects_of_a_mod_block_item() {
    use crate::game::api_values::TryIntoEngine;
    use sunrise_client::mod_content::{ContentMod, resource_root};
    use sunrise_mod_package::{content::Content, enabled::EnabledMods};

    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let scratch =
        std::env::temp_dir().join(format!("sunrise-mooshroom-stew-{}", std::process::id()));
    let mut block = mod_api::Block::new("stewtest:herb", "minecraft:stone");
    block.suspicious_stew_effects = Some(vec![
        ("minecraft:luck".into(), 81),
        ("minecraft:speed".into(), 24),
    ]);
    let mut content = Content::declared("stewtest");
    content
        .declare(&[block.try_into_engine(&()).unwrap()], &[])
        .unwrap();
    let root = resource_root(
        &base,
        &scratch.join("jars"),
        &[ContentMod {
            id: "stewtest".into(),
            content,
            jars: Vec::new(),
        }],
        &scratch,
        &EnabledMods::new([("stewtest".into(), mod_api::Side::Both)]),
    )
    .unwrap();

    let mut resources = None;
    let mut game = game_at(&mut resources, root);
    spawn(&mut game, "minecraft:mooshroom", 7, [2.5, 64.0, 2.5]);
    game.mobs[0].extra.variant = 1;
    game.inventory[0] = Some(ItemStack::new("stewtest:herb", 1));
    assert!(game.interact_mob(target(7)).unwrap());
    assert!(game.inventory[0].is_none());

    game.inventory[0] = Some(ItemStack::new("minecraft:bowl", 1));
    assert!(game.interact_mob(target(7)).unwrap());
    let stew = game.inventory[0].as_ref().unwrap();
    assert_eq!((&*stew.item, stew.count), ("minecraft:suspicious_stew", 1));
    assert_eq!(
        stew.data().unwrap().stew_effects,
        [
            (Arc::from("minecraft:luck"), 81),
            (Arc::from("minecraft:speed"), 24)
        ]
    );
    assert!(game.mobs[0].extra.stew_effects.is_none());
    drop(game);
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "daily"]
fn shearing_by_hand_posts_the_shear_game_event() {
    let mut resources = None;
    let mut game = game(&mut resources);
    game.inventory[0] = Some(ItemStack::new("minecraft:shears", 1));
    super::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
    for (kind, id, at) in [
        ("minecraft:sheep", 11, [1.5, 64.0, 2.5]),
        ("minecraft:mooshroom", 12, [2.5, 64.0, 2.5]),
        ("minecraft:snow_golem", 13, [3.5, 64.0, 2.5]),
    ] {
        spawn(&mut game, kind, id, at);
        assert!(game.interact_mob(target(id)).unwrap(), "{kind}");
        assert_eq!(
            take_shear_events(),
            vec![("minecraft:shear", at, Some(Who::Player))],
            "{kind}"
        );
    }
    super::TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = None);
}
