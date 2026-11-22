//! `Entity.isInvisibleTo`: a spectator sees an invisible body as the translucent
//! one (`LivingEntityRenderer.submit`'s `forceTransparent`), anyone else not at all.
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
use std::sync::Arc;
use sunrise_client::chunk::Chunk;
use sunrise_client::command::args::GameMode;
use sunrise_client::mobs::{Mob, MobTypes};
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;

/// The prepared vanilla resources.
fn root() -> std::path::PathBuf {
    std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .expect("set SUNRISE_TEST_RESOURCES to prepared resources")
}

fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let root = root();
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
    let res = &*resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
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
    // The atlas the game bakes at startup: `mob_geometry` finds a body's texture in it.
    game.gui.atlas = sunrise_client::hud::HudAtlas::build(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
    )
    .unwrap();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in ["forceload add 0 0", "fill 0 63 0 15 63 15 stone"] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// `/effect give @e[type=minecraft:pig,limit=1] minecraft:invisibility 100 0`
/// sets the shared invisible flag a client reads (`LivingEntity.updateInvisibilityStatus`).
/// `Entity.isInvisibleTo` is false for a spectator, so `LivingEntityRenderer.submit`
/// draws the body it cannot show as visible as `entityTranslucentCull` at
/// `ARGB.multiply(654311423, getModelTint)`: alpha 38, in the blending pass.
#[test]
#[ignore = "daily"]
fn a_spectator_sees_the_invisible_body() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let types = MobTypes::load(&root()).unwrap();
    let kind = Arc::new(types.types["minecraft:pig"].clone());
    let mut pig = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    pig.invisible = true;
    game.mobs.push(pig);
    let camera = glam::DVec3::new(6.5, 65.0, 6.5);

    // Anyone but a spectator sees no body at all.
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(out.is_empty(), "an invisible body is hidden from a player");

    // A spectator sees it forced translucent.
    game.set_mode(GameMode::Spectator);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(!out.is_empty(), "a spectator sees the invisible body");
    assert!(
        out.iter().all(|v| v.color >> 24 == 0x26),
        "the body is forced translucent (alpha 38)"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & sunrise_client::gfx::OIT_ENTITY != 0),
        "the translucent body blends"
    );
}
