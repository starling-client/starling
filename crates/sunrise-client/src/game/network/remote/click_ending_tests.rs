use super::*;
use crate::game::remote::use_ends_click;

/// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
/// rocket ends the click and the off hand's shield is not raised (#3424).
/// On foot it passes, so the off hand still gets its turn.
#[test]
#[ignore = "daily"]
fn a_rocket_used_while_gliding_ends_the_click() {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let registry = sunrise_world::level::data::load_registry(&root).unwrap();
    let facts = sunrise_client::blocks::BlockFacts::load(&root, &registry).unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_client::items::Items::load(
        &root,
        &registry,
        &facts,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let rocket = items.get("minecraft:firework_rocket").unwrap();
    assert!(use_ends_click(rocket, true), "gliding");
    assert!(!use_ends_click(rocket, false), "on foot");
    // Other items answer the same whether or not the player glides.
    let shield = items.get("minecraft:shield").unwrap();
    let stick = items.get("minecraft:stick").unwrap();
    for gliding in [false, true] {
        assert!(use_ends_click(shield, gliding));
        assert!(!use_ends_click(stick, gliding));
        // FoodOnAStickItem.use passes on a client: no swing while the boost cools down.
        for rod in [
            "minecraft:carrot_on_a_stick",
            "minecraft:warped_fungus_on_a_stick",
        ] {
            let rod = items.get(rod).unwrap();
            assert!(!use_ends_click(rod, gliding), "{}", rod.id);
        }
        // A carved pumpkin has an equip_slot (head) but is not swappable, so it has no Item.use.
        let pumpkin = items.get("minecraft:carved_pumpkin").unwrap();
        assert!(!use_ends_click(pumpkin, gliding));
    }
}

/// #14773: Flint and steel and fire charge useOn on a client level ends the
/// click (returning true), so when held with an unusable off-hand item, the
/// click does not fall through to UseItem or try the off hand.
#[test]
#[ignore = "daily"]
fn flint_and_steel_use_on_ends_the_click() {
    use crate::game::Hit;
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use sunrise_client::{first_person::Hand, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;

    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
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
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
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
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "FlintTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.inventory[0] = Some(ItemStack::new("minecraft:flint_and_steel", 1));
    game.offhand = Some(ItemStack::new("minecraft:stick", 1));
    game.selected = 0;
    game.forced_hit = Some(Hit {
        pos: [0, 63, 0],
        face: [0, 1, 0],
        point: [0.5, 64.0, 0.5],
        inside: false,
    });

    let done = game.remote_use_block(Hand::Main, mod_api::InteractionAttempt::Press, None);
    assert!(done, "flint and steel useOn must end the click");
}

/// EndCrystalItem.useOn ends the click either way: SUCCESS (the arm swings) on obsidian with
/// air and nothing above, FAIL (no swing) on grass or with the player in the space above. Each
/// sends one use_item_on and tries neither UseItem nor the off hand, whose shield stays down.
/// On cooldown its useOn doesn't run, and the click goes on to the item.
#[test]
#[ignore = "daily"]
fn an_end_crystal_click_on_a_block_ends_there() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        use crate::game::Hit;
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, first_person::Hand, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play;
        let (mut peer, _writer) = connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_local_mode(sunrise_client::command::args::GameMode::Survival);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = &game.pack.assets.registry;
        let obsidian = registry.default_state("minecraft:obsidian").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        // Under the player (its own box fills the space above), clear, and grass.
        game.world.set(0, 63, 0, obsidian).unwrap();
        game.world.set(0, 63, 2, obsidian).unwrap();
        game.world.set(0, 63, 4, grass).unwrap();
        let crystal = ItemStack::new("minecraft:end_crystal", 4);
        game.inventory[0] = Some(crystal.clone());
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        game.selected = 0;
        while peer.recv().is_ok() {}
        let aim = |game: &mut crate::game::Game<'_>, z: i32| {
            game.forced_hit = Some(Hit {
                pos: [0, 63, z],
                face: [0, 1, 0],
                point: [0.5, 64.0, f64::from(z) + 0.5],
                inside: false,
            });
        };
        // The whole click (Minecraft.startUseItem): it ends at the block, so neither the main
        // hand's air use nor the offhand's shield is tried, and nothing is held in use.
        let mut click = |game: &mut crate::game::Game<'_>, z: i32| {
            aim(game, z);
            let swings = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);
            let mut sent = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match play::Serverbound::decode(id, &bytes) {
                    Ok(play::Serverbound::UseItemOn(_)) => sent.push("use_item_on"),
                    Ok(play::Serverbound::UseItem(_)) => sent.push("use_item"),
                    _ => {}
                }
            }
            assert!(game.using.is_none(), "z={z}");
            (game.swings_started != swings, sent)
        };
        assert_eq!(click(game, 2), (true, vec!["use_item_on"]));
        assert_eq!(click(game, 4), (false, vec!["use_item_on"]));
        assert_eq!(click(game, 0), (false, vec!["use_item_on"]));
        // On cooldown the block use itself passes.
        game.cooldowns
            .add(sunrise_client::cooldowns::group(&crystal, None), 20);
        aim(game, 2);
        assert!(!game.remote_use_block(Hand::Main, mod_api::InteractionAttempt::Press, None));
    });
}
